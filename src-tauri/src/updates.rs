//! Manual, application-only network access. Document data never enters this channel.
use crate::files::FileError;
use futures_util::future::{AbortHandle, Abortable};
use reqwest::{Client, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FEED: &str = "https://github.com/MohamedElashri/marktub/releases/latest/download/latest.json";
const KEY: &str = include_str!("../update-public-key.txt");
const MANIFEST_LIMIT: usize = 32 * 1024;
const PACKAGE_LIMIT: usize = 128 * 1024 * 1024;

pub fn configure(context: &mut tauri::Context<tauri::Wry>) {
    context.config_mut().plugins.0.insert(
        "updater".into(),
        serde_json::json!({
            "pubkey": KEY.trim(), "requireSignedVersion": true,
        }),
    );
}
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}
fn err(message: &str) -> FileError {
    FileError::new("update", message)
}
fn target() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin-universal"
    } else {
        "linux-x86_64-deb"
    }
}
fn supported() -> bool {
    use tauri::utils::{config::BundleType, platform::bundle_type};
    #[cfg(target_os = "macos")]
    return bundle_type() == Some(BundleType::App);
    #[cfg(target_os = "linux")]
    return cfg!(target_arch = "x86_64")
        && bundle_type() == Some(BundleType::Deb)
        && tauri::utils::platform::current_exe().ok().as_deref()
            == Some(std::path::Path::new("/usr/bin/marktub"));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    false
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    phase: &'static str,
    current_version: &'static str,
    platform: &'static str,
    can_install: bool,
    version: Option<String>,
    notes: String,
    received: usize,
    total: Option<u64>,
    error: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "idle",
            current_version: env!("CARGO_PKG_VERSION"),
            platform: target(),
            can_install: supported(),
            version: None,
            notes: String::new(),
            received: 0,
            total: None,
            error: None,
        }
    }
}
#[derive(Clone)]
struct Candidate {
    update: Update,
    bytes: Option<Arc<Vec<u8>>>,
}
#[derive(Default)]
pub struct Updates(Mutex<Session>);
struct Session {
    status: Status,
    generation: uuid::Uuid,
    candidate: Option<Candidate>,
    abort: Option<AbortHandle>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            status: Status::default(),
            generation: uuid::Uuid::new_v4(),
            candidate: None,
            abort: None,
        }
    }
}
impl Updates {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Session>, FileError> {
        self.0
            .lock()
            .map_err(|_| err("Update service unavailable."))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: BTreeMap<String, Package>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    url: String,
    signature: String,
}
fn validate(value: &serde_json::Value) -> Result<Version, FileError> {
    let m: Manifest = serde_json::from_value(value.clone())
        .map_err(|_| err("The update manifest is invalid."))?;
    let version = Version::parse(&m.version).map_err(|_| err("Invalid update version."))?;
    if version.to_string() != m.version
        || !version.pre.is_empty()
        || !version.build.is_empty()
        || m.notes.len() > 4096
        || m.pub_date.as_ref().is_some_and(|s| s.len() > 64)
        || m.platforms.len() != 2
    {
        return Err(err("Invalid stable update manifest."));
    }
    for (platform, suffix) in [
        ("darwin-universal", "universal.app.tar.gz"),
        ("linux-x86_64-deb", "amd64.deb"),
    ] {
        let p = m
            .platforms
            .get(platform)
            .ok_or_else(|| err("An update package is missing."))?;
        let expected = format!(
            "https://github.com/MohamedElashri/marktub/releases/download/v{version}/Marktub_{version}_{suffix}"
        );
        if p.url != expected || p.signature.is_empty() || p.signature.len() > 4096 {
            return Err(err("The update package address or signature is invalid."));
        }
    }
    Ok(version)
}
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "github-releases.githubusercontent.com"
            )
        )
}
fn client() -> Result<Client, FileError> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("Marktub/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 6 && allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Unapproved update redirect")
            }
        }))
        .build()
        .map_err(|_| err("Could not start the secure update connection."))
}
fn active(app: &AppHandle, generation: uuid::Uuid) -> Result<(), FileError> {
    if app.state::<Updates>().lock()?.generation != generation {
        Err(err("Update request cancelled."))
    } else {
        Ok(())
    }
}
async fn fetch(
    app: &AppHandle,
    generation: uuid::Uuid,
    url: Url,
    limit: usize,
    progress: bool,
) -> Result<Vec<u8>, FileError> {
    if !allowed(&url) {
        return Err(err("Unapproved update address."));
    }
    let mut response = client()?.get(url).send().await.map_err(|_| {
        err("Could not reach the update server. Check your connection and try again.")
    })?;
    if !response.status().is_success() {
        return Err(err(
            "The update feed or package is unavailable. Try again later or use the installer on the releases page.",
        ));
    }
    let total = response.content_length();
    if total.is_some_and(|n| n > limit as u64) {
        return Err(err("The update download exceeds the size limit."));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| err("The download was interrupted. Try again."))?
    {
        active(app, generation)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(err("The update download exceeds the size limit."));
        }
        bytes.extend_from_slice(&chunk);
        if progress {
            let host = app.state::<Updates>();
            let mut s = host.lock()?;
            if s.generation == generation {
                s.status.received = bytes.len();
                s.status.total = total;
            }
        }
    }
    active(app, generation)?;
    Ok(bytes)
}
#[tauri::command]
pub fn application_update_status(host: tauri::State<'_, Updates>) -> Result<Status, FileError> {
    Ok(host.lock()?.status.clone())
}
#[tauri::command]
pub fn cancel_application_update(host: tauri::State<'_, Updates>) -> Result<(), FileError> {
    let mut s = host.lock()?;
    if s.status.phase == "installing" {
        return Err(err("Installation is already in progress."));
    }
    if let Some(abort) = s.abort.take() {
        abort.abort();
    }
    s.generation = uuid::Uuid::new_v4();
    s.status.phase = if s.candidate.as_ref().is_some_and(|c| c.bytes.is_some()) {
        "ready"
    } else if s.candidate.is_some() {
        "available"
    } else {
        "idle"
    };
    Ok(())
}
#[tauri::command]
pub async fn check_application_update(app: AppHandle) -> Result<(), FileError> {
    let (abort, registration) = AbortHandle::new_pair();
    let generation = {
        let host = app.state::<Updates>();
        let mut s = host.lock()?;
        if matches!(s.status.phase, "checking" | "downloading" | "installing") {
            return Err(err("An update request is already in progress."));
        }
        s.generation = uuid::Uuid::new_v4();
        s.candidate = None;
        s.status = Status::default();
        s.status.phase = "checking";
        s.abort = Some(abort);
        s.generation
    };
    let result = Abortable::new(
        async {
            let bytes = fetch(
                &app,
                generation,
                Url::parse(FEED).unwrap(),
                MANIFEST_LIMIT,
                false,
            )
            .await?;
            let json: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| err("The update manifest is invalid."))?;
            let version = validate(&json)?;
            let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
            let candidate = if version > current && supported() {
                let update = app
                    .updater_builder()
                    .target(target())
                    .endpoints(vec![Url::parse(FEED).unwrap()])
                    .map_err(|_| err("Update configuration unavailable."))?
                    .build()
                    .map_err(|_| {
                        err("This installation cannot be updated here. Use the released installer.")
                    })?
                    .restore_update(json.clone())
                    .map_err(|_| err("The update manifest could not be restored."))?
                    .ok_or_else(|| err("The update version is not newer."))?;
                Some(Candidate {
                    update,
                    bytes: None,
                })
            } else {
                None
            };
            let host = app.state::<Updates>();
            let mut s = host.lock()?;
            if s.generation == generation {
                s.status.phase = if version > current {
                    "available"
                } else {
                    "current"
                };
                s.status.version = Some(version.to_string());
                s.status.notes = json["notes"].as_str().unwrap_or_default().to_owned();
                s.candidate = candidate;
            }
            Ok(())
        },
        registration,
    )
    .await
    .unwrap_or_else(|_| Err(err("Update request cancelled.")));
    finish(&app, generation, result)
}
fn finish(
    app: &AppHandle,
    generation: uuid::Uuid,
    result: Result<(), FileError>,
) -> Result<(), FileError> {
    let host = app.state::<Updates>();
    let mut s = host.lock()?;
    if s.generation == generation {
        s.abort = None;
        if let Err(error) = result {
            s.status.error = Some(error.message);
            s.status.phase = if s.candidate.is_some() {
                "available"
            } else {
                "idle"
            };
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn download_application_update(app: AppHandle) -> Result<(), FileError> {
    let (abort, registration) = AbortHandle::new_pair();
    let (generation, candidate) = {
        let host = app.state::<Updates>();
        let mut s = host.lock()?;
        if s.status.phase != "available" {
            return Err(err("Check for an update first."));
        }
        let candidate = s
            .candidate
            .clone()
            .ok_or_else(|| err("Use the released installer for this installation."))?;
        s.generation = uuid::Uuid::new_v4();
        s.status.phase = "downloading";
        s.abort = Some(abort);
        s.status.error = None;
        s.status.received = 0;
        s.status.total = None;
        (s.generation, candidate)
    };
    let result = Abortable::new(
        async {
            let bytes = fetch(
                &app,
                generation,
                candidate.update.download_url.clone(),
                PACKAGE_LIMIT,
                true,
            )
            .await?;
            candidate
                .update
                .verify(&bytes)
                .map_err(|_| err("Update signature verification failed. Nothing was installed."))?;
            let host = app.state::<Updates>();
            let mut s = host.lock()?;
            if s.generation == generation {
                s.candidate = Some(Candidate {
                    bytes: Some(Arc::new(bytes)),
                    ..candidate
                });
                s.status.phase = "ready";
            }
            Ok(())
        },
        registration,
    )
    .await
    .unwrap_or_else(|_| Err(err("Update request cancelled.")));
    finish(&app, generation, result)
}

#[tauri::command]
pub async fn prepare_application_update(app: AppHandle) -> Result<(), FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Updates>();
        let s = host.lock()?;
        if s.status.phase != "ready" || !supported() {
            return Err(err("Download and verify an update first."));
        }
        let candidate = s
            .candidate
            .as_ref()
            .ok_or_else(|| err("No verified update is available."))?;
        let bytes = candidate
            .bytes
            .as_ref()
            .ok_or_else(|| err("Download the update first."))?;
        candidate
            .update
            .verify(bytes)
            .map_err(|_| err("Update signature verification failed."))?;
        #[cfg(target_os = "linux")]
        installed(&candidate.update.version)?;
        Ok(())
    })
    .await
    .map_err(|_| err("Update preparation interrupted."))?
}

/// Called only after the app's ordinary unsaved-work prompts, under the native
/// editing/job reservation. No renderer-supplied paths, versions or packages.
pub fn install(app: &AppHandle) -> Result<(), FileError> {
    let candidate = {
        let host = app.state::<Updates>();
        let mut s = host.lock()?;
        if s.status.phase != "ready" || !supported() {
            return Err(err("Download and verify an update first."));
        }
        let c = s
            .candidate
            .clone()
            .ok_or_else(|| err("No verified update is available."))?;
        s.status.phase = "installing";
        c
    };
    let result = (|| {
        let bytes = candidate
            .bytes
            .as_ref()
            .ok_or_else(|| err("Download the update first."))?;
        candidate
            .update
            .verify(bytes)
            .map_err(|_| err("Update signature verification failed."))?;
        #[cfg(target_os = "macos")]
        candidate.update.install(bytes.as_slice()).map_err(|_| err("Installation failed. The app has not restarted; try again or use the released installer."))?;
        #[cfg(target_os = "linux")]
        installed(&candidate.update.version)?;
        Ok(())
    })();
    if result.is_err() {
        app.state::<Updates>().lock()?.status.phase = "ready";
    }
    result
}
#[tauri::command]
pub async fn open_application_installer(app: AppHandle) -> Result<(), FileError> {
    #[cfg(target_os = "linux")]
    return tauri::async_runtime::spawn_blocking(move || {
        use std::{io::Write, os::unix::fs::OpenOptionsExt, process::{Command, Stdio}};
        let host = app.state::<Updates>(); let s = host.lock()?;
        if s.status.phase != "ready" || !supported() { return Err(err("Download and verify an update first.")); }
        let candidate = s.candidate.as_ref().ok_or_else(|| err("No update is available."))?;
        let bytes = candidate.bytes.as_ref().ok_or_else(|| err("Download the update first."))?;
        candidate.update.verify(bytes).map_err(|_| err("Update signature verification failed."))?;
        let dir = app.path().download_dir().map_err(|_| err("Downloads folder unavailable."))?.join(format!("Marktub-update-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir)?;
        let path = dir.join(format!("Marktub_{}_amd64.deb", candidate.update.version));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&path)?;
        file.write_all(bytes)?; file.sync_all()?; drop(file);
        // Hand off an exclusively created, signed package. The OS installer owns
        // privilege approval; Marktub never asks for a password or invokes sudo.
        let mut child = Command::new("/usr/bin/xdg-open").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            .map_err(|_| err("Could not open the system installer. Open the Marktub update package in Downloads."))?;
        std::thread::sleep(Duration::from_millis(100));
        if let Some(status) = child.try_wait()? { if !status.success() { return Err(err("No system installer accepted the package. Open it from Downloads with your package installer.")); } }
        else { std::thread::spawn(move || { let _ = child.wait(); }); }
        Ok(())
    }).await.map_err(|_| err("Installer handoff interrupted."))?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Err(err("The system package installer is only used on Linux."))
    }
}
#[cfg(target_os = "linux")]
fn installed(version: &str) -> Result<(), FileError> {
    use std::{
        io::Read,
        os::unix::fs::MetadataExt,
        process::{Command, Stdio},
        time::Instant,
    };
    let metadata = std::fs::symlink_metadata("/usr/bin/marktub")?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(err("The installed executable failed its ownership check."));
    }
    let mut child = Command::new("/usr/bin/dpkg-query")
        .args(["-W", "-f=${Version}", "--", "marktub"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            let mut output = String::new();
            child
                .stdout
                .take()
                .unwrap()
                .take(128)
                .read_to_string(&mut output)?;
            if status.success() && output.trim() == version {
                return Ok(());
            }
            return Err(err(
                "Finish installing this update in the system installer before restarting.",
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err(
                "Could not confirm installation. Try again after the system installer finishes.",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> serde_json::Value {
        serde_json::json!({"version":"9.0.0", "notes":"Example", "platforms": {
            "darwin-universal":{"url":"https://github.com/MohamedElashri/marktub/releases/download/v9.0.0/Marktub_9.0.0_universal.app.tar.gz","signature":"signed"},
            "linux-x86_64-deb":{"url":"https://github.com/MohamedElashri/marktub/releases/download/v9.0.0/Marktub_9.0.0_amd64.deb","signature":"signed"}}})
    }
    #[test]
    fn rejects_remote_overrides_and_wrong_packages() {
        assert_eq!(validate(&manifest()).unwrap(), Version::new(9, 0, 0));
        for key in ["config", "url", "signature"] {
            let mut m = manifest();
            m[key] = serde_json::json!({"pubkey":"attacker"});
            assert!(validate(&m).is_err());
        }
        let mut m = manifest();
        m["platforms"]["linux-x86_64-deb"]["url"] = "https://example.com/package.deb".into();
        assert!(validate(&m).is_err());
        m = manifest();
        m["version"] = "9.0.0-rc.1".into();
        assert!(validate(&m).is_err());
        m = manifest();
        m["notes"] = "x".repeat(4097).into();
        assert!(validate(&m).is_err());
        m = manifest();
        m["platforms"]["linux-x86_64-deb"]["signature"] = "".into();
        assert!(validate(&m).is_err());
    }
    #[test]
    fn redirects_are_https_and_closed() {
        for url in [
            FEED,
            "https://release-assets.githubusercontent.com/asset?token=test",
        ] {
            assert!(allowed(&Url::parse(url).unwrap()));
        }
        for url in [
            "http://github.com/file",
            "https://github.com.evil.test/file",
            "https://user:pass@github.com/file",
            "https://github.com:444/file",
            "https://127.0.0.1/file",
        ] {
            assert!(!allowed(&Url::parse(url).unwrap()));
        }
    }
    #[test]
    fn construction_never_starts_a_check() {
        let host = Updates::default();
        let s = host.lock().unwrap();
        assert_eq!(s.status.phase, "idle");
        assert!(s.candidate.is_none());
    }
}
