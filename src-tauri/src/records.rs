use crate::{
    files::FileError,
    private_store::{self, PrivateStore},
    settings::{Diagnostic, Settings, Value},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
const LIMIT: u64 = 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Position {
    pub byte: u64,
    pub caret: usize,
    pub anchor: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Data {
    schema: u32,
    global: String,
    workspaces: BTreeMap<String, String>,
    files: BTreeMap<String, String>,
    positions: BTreeMap<String, Position>,
    paths: BTreeMap<String, String>,
}
pub struct Records {
    blocked: bool,
    data: Data,
    pub warning: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResult {
    pub text: String,
    pub resolved: BTreeMap<String, Value>,
    pub diagnostics: Vec<Diagnostic>,
}
fn decode(bytes: &[u8]) -> Result<Data, FileError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| {
        FileError::new(
            "settings-storage",
            "Settings record is invalid; its backup was preserved.",
        )
    })?;
    if value.get("schema").and_then(|v| v.as_u64()) == Some(0) {
        if value.as_object().is_none_or(|v| {
            v.keys()
                .any(|key| !["schema", "settings"].contains(&key.as_str()))
        }) {
            return Err(FileError::new(
                "settings-schema",
                "Unsupported legacy settings shape; original preserved.",
            ));
        }
        let global = value
            .get("settings")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                FileError::new(
                    "settings-schema",
                    "Legacy settings must contain literal text.",
                )
            })?
            .to_owned();
        value = serde_json::json!({"schema":1,"global":global});
    }
    if value.get("schema").and_then(|v| v.as_u64()) != Some(1) {
        return Err(FileError::new(
            "settings-schema",
            "Unsupported settings schema. The original record was kept.",
        ));
    }
    let data: Data = serde_json::from_value(value)
        .map_err(|_| FileError::new("settings-storage", "Unsupported settings record shape."))?;
    if data.schema != 1 {
        return Err(FileError::new(
            "settings-schema",
            "Unsupported settings schema. The original record was kept.",
        ));
    }
    for text in std::iter::once(&data.global)
        .chain(data.workspaces.values())
        .chain(data.files.values())
    {
        Settings::parse(text.as_bytes()).map_err(|_| {
            FileError::new(
                "settings-storage",
                "Stored settings failed validation; no values were applied.",
            )
        })?;
    }
    Ok(data)
}
impl Records {
    pub fn load(store: &PrivateStore) -> Result<Self, FileError> {
        let path = store.root.join("metadata.json");
        if !path.exists() {
            return Ok(Self {
                data: Data {
                    schema: 1,
                    ..Data::default()
                },
                warning: None,
                blocked: false,
            });
        }
        match private_store::read(&path, LIMIT).and_then(|b| decode(&b)) {
            Ok(data) => Ok(Self {
                data,
                warning: None,
                blocked: false,
            }),
            Err(error) => {
                let backup = private_store::read(&store.root.join("metadata.bak"), LIMIT)
                    .and_then(|b| decode(&b));
                let blocked = backup.is_err() || error.code == "settings-schema";
                Ok(Self {
                    data: backup.unwrap_or_else(|_| Data {
                        schema: 1,
                        ..Data::default()
                    }),
                    blocked,
                    warning: Some(format!(
                        "{} {}",
                        error.message,
                        if blocked {
                            "Defaults or a backup are in use without overwriting the original. Reset settings explicitly to repair storage."
                        } else {
                            "Loaded the last-good backup; the damaged file remains available."
                        }
                    )),
                })
            }
        }
    }
    fn persist(&mut self, store: &PrivateStore, data: Data) -> Result<(), FileError> {
        if self.blocked {
            return Err(FileError::new(
                "settings-storage",
                "Settings storage needs an explicit reset. The damaged record is preserved.",
            ));
        }
        let bytes = serde_json::to_vec(&data)
            .map_err(|_| FileError::new("settings-storage", "Cannot encode settings."))?;
        if bytes.len() as u64 > LIMIT
            || data.positions.len() > 10000
            || data.files.len() > 10000
            || data.workspaces.len() > 10000
            || data.paths.len() > 10000
        {
            return Err(FileError::new(
                "quota",
                "Durable settings/position records reached their storage budget.",
            ));
        }
        store.reserve(bytes.len() as u64 * 2)?;
        let path = store.root.join("metadata.json");
        if let Ok(previous) = private_store::read(&path, LIMIT)
            && decode(&previous).is_ok()
        {
            private_store::atomic(&store.root.join("metadata.bak"), &previous)?;
        } else if path.exists() {
            std::fs::hard_link(
                &path,
                store
                    .root
                    .join(format!("metadata-rejected-{}.json", uuid::Uuid::new_v4())),
            )?;
        }
        private_store::atomic(&path, &bytes)?;
        self.data = data;
        Ok(())
    }
    pub fn reset(&mut self, store: &PrivateStore) -> Result<(), FileError> {
        let path = store.root.join("metadata.json");
        if path.exists() {
            std::fs::hard_link(
                &path,
                store
                    .root
                    .join(format!("metadata-rejected-{}.json", uuid::Uuid::new_v4())),
            )?;
        }
        let blocked = self.blocked;
        self.blocked = false;
        let data = Data {
            schema: 1,
            positions: self.data.positions.clone(),
            ..Data::default()
        };
        if let Err(error) = self.persist(store, data) {
            self.blocked = blocked;
            return Err(error);
        }
        self.warning = None;
        Ok(())
    }
    pub fn settings(
        &self,
        scope: &str,
        file: &str,
        workspace: &str,
    ) -> Result<SettingsResult, FileError> {
        let text = match scope {
            "global" => self.data.global.clone(),
            "workspace" => self
                .data
                .workspaces
                .get(workspace)
                .cloned()
                .unwrap_or_default(),
            "file" => self.data.files.get(file).cloned().unwrap_or_default(),
            _ => return Err(FileError::new("scope", "Unknown settings scope.")),
        };
        let empty = Settings::parse(b"").expect("empty");
        let global = Settings::parse(self.data.global.as_bytes()).unwrap_or_else(|_| empty.clone());
        let workspace = Settings::parse(
            self.data
                .workspaces
                .get(workspace)
                .map_or("", String::as_str)
                .as_bytes(),
        )
        .unwrap_or_else(|_| empty.clone());
        let file = Settings::parse(
            self.data
                .files
                .get(file)
                .map_or("", String::as_str)
                .as_bytes(),
        )
        .unwrap_or_else(|_| empty.clone());
        let resolved = Settings::resolve(&global, &workspace, &file, &empty).values();
        Ok(SettingsResult {
            text,
            resolved,
            diagnostics: vec![],
        })
    }
    pub fn preview(
        &self,
        scope: &str,
        file: &str,
        workspace: &str,
        text: &str,
    ) -> Result<SettingsResult, FileError> {
        let mut result = self.settings(scope, file, workspace)?;
        if let Err(diagnostics) = Settings::parse(text.as_bytes()) {
            result.diagnostics = diagnostics;
            return Ok(result);
        }
        let mut temporary = Self {
            blocked: self.blocked,
            data: self.data.clone(),
            warning: self.warning.clone(),
        };
        match scope {
            "global" => temporary.data.global = text.into(),
            "workspace" => {
                temporary
                    .data
                    .workspaces
                    .insert(workspace.into(), text.into());
            }
            "file" => {
                temporary.data.files.insert(file.into(), text.into());
            }
            _ => return Err(FileError::new("scope", "Unknown settings scope.")),
        }
        let mut result = temporary.settings(scope, file, workspace)?;
        result.diagnostics = crate::preferences::effective(&result.resolved);
        Ok(result)
    }
    pub fn update(
        &mut self,
        store: &PrivateStore,
        scope: &str,
        file: &str,
        workspace: &str,
        text: &str,
    ) -> Result<SettingsResult, FileError> {
        if let Err(diagnostics) = Settings::parse(text.as_bytes()) {
            let mut result = self.settings(scope, file, workspace)?;
            result.diagnostics = diagnostics;
            return Ok(result);
        }
        let mut data = self.data.clone();
        match scope {
            "global" => data.global = text.into(),
            "workspace" => {
                data.workspaces.insert(workspace.into(), text.into());
            }
            "file" => {
                data.files.insert(file.into(), text.into());
            }
            _ => return Err(FileError::new("scope", "Unknown settings scope.")),
        };
        let preview = self.preview(scope, file, workspace, text)?;
        if !preview.diagnostics.is_empty() {
            return Ok(preview);
        }
        self.persist(store, data)?;
        self.settings(scope, file, workspace)
    }
    pub fn note_path(&mut self, store: &PrivateStore, path: &Path) -> Result<(), FileError> {
        use base64::Engine;
        use std::os::unix::ffi::OsStrExt;
        let key = crate::editing::key(path);
        if self.data.paths.contains_key(&key) {
            return Ok(());
        }
        let mut data = self.data.clone();
        data.paths.insert(
            key,
            base64::engine::general_purpose::STANDARD.encode(path.as_os_str().as_bytes()),
        );
        self.persist(store, data)
    }
    pub fn relocate(
        &mut self,
        store: &PrivateStore,
        old: &Path,
        new: &Path,
    ) -> Result<(), FileError> {
        use base64::Engine;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut data = self.data.clone();
        let mut pairs = vec![(old.to_owned(), new.to_owned())];
        for encoded in data.paths.values() {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes));
                if let Ok(suffix) = path.strip_prefix(old) {
                    pairs.push((path.clone(), new.join(suffix)));
                }
            }
        }
        for (from, to) in pairs {
            let from = crate::editing::key(&from);
            let dest = crate::editing::key(&to);
            if let Some(value) = data.positions.remove(&from) {
                data.positions.insert(dest.clone(), value);
            }
            if let Some(value) = data.files.remove(&from) {
                data.files.insert(dest.clone(), value);
            }
            if let Some(value) = data.workspaces.remove(&from) {
                data.workspaces.insert(dest.clone(), value);
            }
            if data.paths.remove(&from).is_some() {
                data.paths.insert(
                    dest,
                    base64::engine::general_purpose::STANDARD.encode(to.as_os_str().as_bytes()),
                );
            }
        }
        self.persist(store, data)
    }
    pub fn position(&self, key: &str) -> Position {
        self.data.positions.get(key).cloned().unwrap_or_default()
    }
    pub fn remember(
        &mut self,
        store: &PrivateStore,
        key: &str,
        position: Position,
    ) -> Result<(), FileError> {
        let mut data = self.data.clone();
        data.positions.insert(key.into(), position);
        self.persist(store, data)
    }
    pub fn export(path: &Path, text: &str) -> Result<(), FileError> {
        Settings::parse(text.as_bytes()).map_err(|_| {
            FileError::new("settings", "Correct invalid settings before exporting.")
        })?;
        let target = crate::scoped_file::ScopedFile::destination(path)?;
        let source = crate::document::Provenance::new(text.as_bytes())
            .map_err(|_| FileError::new("encoding", "Invalid settings encoding."))?;
        crate::atomic_save::save_as(&target, &source.snapshot())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_moves_relocate_recorded_file_and_workspace_settings_and_positions() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrivateStore::open(&dir.path().join("private")).unwrap();
        let mut records = Records::load(&store).unwrap();
        let old = dir.path().join("old/nested/a.txt");
        let old_parent = old.parent().unwrap();
        let new = dir.path().join("new/nested/a.txt");
        let from = crate::editing::key(&old);
        let workspace = crate::editing::key(old_parent);
        records.note_path(&store, &old).unwrap();
        records.note_path(&store, old_parent).unwrap();
        records
            .update(&store, "file", &from, &workspace, "$reader.width=810")
            .unwrap();
        records
            .update(
                &store,
                "workspace",
                &from,
                &workspace,
                "$reader.font_size=21",
            )
            .unwrap();
        records
            .remember(
                &store,
                &from,
                Position {
                    byte: 55,
                    caret: 8,
                    anchor: Some("h-test".into()),
                },
            )
            .unwrap();
        records
            .relocate(&store, &dir.path().join("old"), &dir.path().join("new"))
            .unwrap();
        let target = crate::editing::key(&new);
        let workspace = crate::editing::key(new.parent().unwrap());
        assert_eq!(records.position(&target).byte, 55);
        assert!(
            records
                .settings("file", &target, &workspace)
                .unwrap()
                .text
                .contains("810")
        );
        assert!(
            records
                .settings("workspace", &target, &workspace)
                .unwrap()
                .text
                .contains("21")
        );
        assert_eq!(records.position(&from).byte, 0);
    }
    #[test]
    fn backup_fallback_and_cache_reset_preserve_durable_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrivateStore::open(&dir.path().join("private")).unwrap();
        let mut r = Records::load(&store).unwrap();
        r.update(&store, "global", "f", "w", "$reader.width=800")
            .unwrap();
        r.remember(
            &store,
            "f",
            Position {
                byte: 12,
                caret: 7,
                anchor: None,
            },
        )
        .unwrap();
        r.update(&store, "global", "f", "w", "$reader.width=900")
            .unwrap();
        private_store::atomic(&store.root.join("metadata.json"), b"damaged record").unwrap();
        let mut recovered = Records::load(&store).unwrap();
        assert!(recovered.warning.is_some());
        assert_eq!(
            recovered.settings("global", "f", "w").unwrap().text,
            "$reader.width=800"
        );
        assert_eq!(recovered.position("f").caret, 7);
        recovered
            .update(&store, "global", "f", "w", "$reader.width=1000")
            .unwrap();
        let metadata = private_store::read(&store.root.join("metadata.json"), LIMIT).unwrap();
        std::fs::write(
            store.recovery().join("durable.fixture"),
            b"unsaved recovery",
        )
        .unwrap();
        std::fs::write(store.root.join("cache/junk"), b"disposable").unwrap();
        store.reset_cache().unwrap();
        assert_eq!(
            private_store::read(&store.root.join("metadata.json"), LIMIT).unwrap(),
            metadata
        );
        assert_eq!(
            std::fs::read(store.recovery().join("durable.fixture")).unwrap(),
            b"unsaved recovery"
        );
        assert!(PrivateStore::open(&store.root).is_err());
    }
    #[test]
    fn legacy_migration_is_atomic_on_first_mutation_and_future_schemas_require_reset() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrivateStore::open(&dir.path().join("private")).unwrap();
        let original = br#"{"schema":0,"settings":"$theme=dark"}"#;
        private_store::atomic(&store.root.join("metadata.json"), original).unwrap();
        let mut r = Records::load(&store).unwrap();
        assert_eq!(r.settings("global", "f", "w").unwrap().text, "$theme=dark");
        r.remember(
            &store,
            "f",
            Position {
                byte: 1,
                caret: 1,
                anchor: None,
            },
        )
        .unwrap();
        assert_eq!(
            private_store::read(&store.root.join("metadata.bak"), LIMIT).unwrap(),
            original
        );
        private_store::atomic(
            &store.root.join("metadata.json"),
            br#"{"schema":9,"future":"preserve"}"#,
        )
        .unwrap();
        let mut r = Records::load(&store).unwrap();
        assert!(
            r.update(&store, "global", "f", "w", "$theme=light")
                .is_err()
        );
        r.reset(&store).unwrap();
        assert_eq!(r.settings("global", "f", "w").unwrap().text, "");
        assert!(std::fs::read_dir(&store.root).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("metadata-rejected-")
        }));
    }
    #[test]
    fn quota_refusal_and_invalid_settings_never_replace_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let store = PrivateStore::open(&dir.path().join("private")).unwrap();
        let mut r = Records::load(&store).unwrap();
        r.update(&store, "global", "f", "w", "$theme=dark").unwrap();
        let before = private_store::read(&store.root.join("metadata.json"), LIMIT).unwrap();
        assert!(
            !r.update(&store, "global", "f", "w", "$theme=light\n$unknown=1")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        assert_eq!(
            private_store::read(&store.root.join("metadata.json"), LIMIT).unwrap(),
            before
        );
        assert_eq!(
            store
                .reserve(crate::private_store::STORE_QUOTA + 1)
                .unwrap_err()
                .code,
            "quota"
        );
    }
}
