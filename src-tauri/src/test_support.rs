//! Debug-only deterministic native-picker fixture. Not compiled or granted in releases.
use crate::files::FileError;
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex};
#[derive(Default)]
pub struct Fixtures(Mutex<Option<Fixture>>);
struct Fixture {
    folder: PathBuf,
    open: bool,
    save: bool,
    export: bool,
    image: bool,
    power_picks: std::collections::HashMap<String, PathBuf>,
    power_confirm: Option<bool>,
    last_confirmation: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureResult {
    pub text: String,
    pub path: String,
    pub pid: u32,
}
impl Fixtures {
    pub fn action(&self, action: &str, text: Option<String>) -> Result<FixtureResult, FileError> {
        let (action, text) = match action {
            "seed-1MiB" | "seed-10MiB" | "seed-100MiB" | "seed-reader-1MiB"
            | "seed-reader-10MiB" | "seed-reader-100MiB" => {
                let size = if action.ends_with("-1MiB") {
                    1024 * 1024
                } else {
                    if action.ends_with("-100MiB") {
                        100 * 1024 * 1024
                    } else {
                        10 * 1024 * 1024
                    }
                };
                let ordinary = "# Reader benchmark\n\n[Forward reference][final]\n\n".to_string()
                    + &("Local documents stay useful while the reader works. ".repeat(30) + "\n\n")
                        .repeat(32);
                let line = if action.starts_with("seed-reader-") {
                    ordinary.as_str()
                } else {
                    "# Notes\r\n\r\nA bounded source session.\n\n"
                };
                (
                    "seed",
                    Some(
                        line.repeat(size / line.len())
                            + &"x".repeat(size % line.len())
                            + if action.starts_with("seed-reader-") {
                                "\n\nFINAL_READER_SENTINEL\n\n[final]: final.md\n"
                            } else {
                                ""
                            },
                    ),
                )
            }
            _ => (action, text),
        };
        let mut guard = self.0.lock().unwrap();
        if action == "seed" {
            let folder = std::env::temp_dir().join(format!("marktub-e2e-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&folder)?;
            std::fs::write(
                folder.join("source.txt"),
                text.as_deref()
                    .unwrap_or("\u{feff}# Original 🦀\r\n\r\nline العربية\nlast\r"),
            )?;
            *guard = Some(Fixture {
                folder,
                open: true,
                save: false,
                export: false,
                image: false,
                power_picks: Default::default(),
                power_confirm: None,
                last_confirmation: String::new(),
            });
        }
        let fixture = guard
            .as_mut()
            .ok_or_else(|| FileError::new("test", "Seed a fixture first."))?;
        match action {
            "external" => std::fs::write(
                fixture.folder.join("source.txt"),
                text.as_deref().unwrap_or("# External\n"),
            )?,
            "save-as" => fixture.save = true,
            "export" => fixture.export = true,
            "image" => {
                use base64::Engine;
                let bytes=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=").unwrap();
                std::fs::write(fixture.folder.join("pixel.png"), bytes)?;
                fixture.image = true;
            }
            "open" => fixture.open = true,
            "power-pick" => {
                let v: serde_json::Value = serde_json::from_str(text.as_deref().unwrap_or("{}"))
                    .map_err(|_| FileError::new("test", "Invalid test choice"))?;
                fixture.power_picks.insert(
                    v["role"].as_str().unwrap_or("").into(),
                    PathBuf::from(v["path"].as_str().unwrap_or("")),
                );
            }
            "power-confirm" => fixture.power_confirm = Some(text.as_deref() == Some("yes")),
            "power-read-confirmation" => {}
            "seed" | "read" | "read-copy" | "read-export" => {}
            _ => return Err(FileError::new("test", "Unknown fixture action.")),
        };
        let path = fixture.folder.join(if action == "read-export" {
            "document.html"
        } else if action == "read-copy" {
            "copy.txt"
        } else {
            "source.txt"
        });
        Ok(FixtureResult {
            pid: std::process::id(),
            text: if action == "power-read-confirmation" {
                fixture.last_confirmation.clone()
            } else if std::fs::metadata(&path).map_or(0, |m| m.len()) <= 65536 {
                std::fs::read_to_string(&path).unwrap_or_default()
            } else {
                String::new()
            },
            path: path.to_string_lossy().into_owned(),
        })
    }
    pub fn take_power_pick(&self, role: &str) -> Option<PathBuf> {
        self.0.lock().unwrap().as_mut()?.power_picks.remove(role)
    }
    pub fn take_power_confirm(&self, message: &str) -> Option<bool> {
        let mut guard = self.0.lock().unwrap();
        let f = guard.as_mut()?;
        f.last_confirmation = message.into();
        f.power_confirm.take()
    }
    pub fn root(&self) -> Option<PathBuf> {
        self.0.lock().unwrap().as_ref().map(|f| f.folder.clone())
    }
    pub fn take_open(&self) -> Option<PathBuf> {
        let mut g = self.0.lock().unwrap();
        let f = g.as_mut()?;
        if !f.open {
            return None;
        }
        f.open = false;
        Some(f.folder.join("source.txt"))
    }
    pub fn take_image(&self) -> Option<PathBuf> {
        let mut g = self.0.lock().unwrap();
        let f = g.as_mut()?;
        if !f.image {
            return None;
        }
        f.image = false;
        Some(f.folder.join("pixel.png"))
    }
    pub fn take_export(&self) -> Option<PathBuf> {
        let mut g = self.0.lock().unwrap();
        let f = g.as_mut()?;
        if !f.export {
            return None;
        }
        f.export = false;
        Some(f.folder.join("document.html"))
    }
    pub fn take_save(&self) -> Option<PathBuf> {
        let mut g = self.0.lock().unwrap();
        let f = g.as_mut()?;
        if !f.save {
            return None;
        }
        f.save = false;
        Some(f.folder.join("copy.txt"))
    }
}

pub fn verify_update(app: &tauri::AppHandle, text: &str) -> Result<FixtureResult, FileError> {
    use base64::Engine;
    use tauri_plugin_updater::UpdaterExt;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Verification {
        public_key: String,
        manifest: serde_json::Value,
        payload: String,
    }
    let input: Verification = serde_json::from_str(text)
        .map_err(|_| FileError::new("test", "Invalid verification fixture."))?;
    let update = app
        .updater_builder()
        .pubkey(input.public_key)
        .target(if cfg!(target_os = "macos") {
            "darwin-universal"
        } else {
            "linux-x86_64-deb"
        })
        .endpoints(vec![
            "https://github.com/MohamedElashri/marktub/releases/latest/download/latest.json"
                .parse()
                .unwrap(),
        ])
        .unwrap()
        .build()
        .map_err(|_| FileError::new("test", "Fixture updater unavailable."))?
        .restore_update(input.manifest)
        .map_err(|_| FileError::new("test", "Invalid fixture manifest."))?
        .ok_or_else(|| FileError::new("test", "Fixture version is not newer."))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.payload)
        .map_err(|_| FileError::new("test", "Invalid fixture payload."))?;
    update
        .verify(&bytes)
        .map_err(|_| FileError::new("test", "Fixture signature or signed version rejected."))?;
    Ok(FixtureResult {
        text: "verified".into(),
        path: String::new(),
        pid: std::process::id(),
    })
}
