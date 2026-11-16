use crate::{
    atomic_save::{self, Identity},
    document::{
        Snapshot,
        journal::{Ack, Batch, JournalError, Session},
    },
    files::{Documents, FileError},
    private_store::{self, PrivateStore},
    records::{Position, Records, SettingsResult},
    scoped_file::ScopedFile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
const CHECKPOINT_INTERVAL: u64 = 256;
fn error(e: JournalError) -> FileError {
    match e {
        JournalError::Io(e) => e.into(),
        JournalError::Capacity => FileError::new(
            "capacity",
            "Recovery reached its resource budget. Save an independent copy, or save and reopen to start a fresh undo history.",
        ),
        JournalError::Poisoned => FileError::new(
            "recovery",
            "A recovery append failed. Your text remains open. Retry or use Save As.",
        ),
        JournalError::Generation => {
            FileError::new("generation", "This editing generation is stale.")
        }
        JournalError::Revision | JournalError::Duplicate => FileError::new(
            "revision",
            "The edit sequence is stale or out of order. Pending input was kept.",
        ),
        JournalError::Edits(e) => FileError::new(
            "edit",
            format!("Invalid source changeset ({e:?}); pending input was kept."),
        ),
        _ => FileError::new(
            "recovery",
            "Recovery validation failed. Original records were preserved.",
        ),
    }
}
pub(crate) fn key(path: &Path) -> String {
    crate::scoped_file::path_key(path)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    schema: u32,
    name: String,
    saved_hash: String,
    saved_revision: u64,
    saved_file: String,
    updated: u64,
    #[serde(default)]
    pending: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditState {
    pub received_revision: u64,
    pub durable_revision: u64,
    pub saved_revision: u64,
    pub dirty: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditDescriptor {
    pub id: String,
    pub generation: String,
    pub journal_generation: String,
    pub text: String,
    pub state: EditState,
    pub name: String,
    pub caret: usize,
    pub needs_save_as: bool,
    pub range: Option<EditWindow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub acknowledgements: Vec<Ack>,
    pub state: EditState,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub revision: u64,
    pub noop: bool,
    pub rescue: bool,
    pub name: String,
    pub state: EditState,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub updated: u64,
    pub damaged: bool,
    pub clean: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub base: String,
    pub disk: String,
    pub clipped: bool,
    pub missing: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditWindow {
    pub from: usize,
    pub to: usize,
    pub total: usize,
}
fn window_text(session: &Session, start: usize) -> Result<(String, Option<EditWindow>), FileError> {
    let source = session.source();
    let total = source.len_utf16();
    if total <= 16 * 1024 * 1024 && source.backing_len() <= 16 * 1024 * 1024 {
        return Ok((source.logical_text(), None));
    }
    let mut from = start.min(total);
    while source.logical_range(from, from).is_err() {
        from = from.saturating_sub(1);
    }
    let mut to = (from + 131072).min(total);
    while source.logical_range(to, to).is_err() {
        to = to.saturating_sub(1);
    }
    Ok((
        source
            .logical_range(from, to)
            .map_err(|_| FileError::new("range", "Invalid editing window."))?,
        Some(EditWindow { from, to, total }),
    ))
}
struct Live {
    session: Session,
    transport: String,
    target: Option<Arc<ScopedFile>>,
    expected: String,
    identity: Option<Identity>,
    saved: Snapshot,
    saved_revision: u64,
    name: String,
    file_key: String,
    workspace_key: String,
    checkpoint_revision: u64,
    uncertain: bool,
    window: Option<EditWindow>,
    reader: Option<(String, Snapshot)>,
}
impl Live {
    fn state(&self) -> EditState {
        EditState {
            received_revision: self.session.received_revision(),
            durable_revision: self.session.durable_revision(),
            saved_revision: self.saved_revision,
            dirty: self.uncertain
                || !self
                    .session
                    .source()
                    .snapshot()
                    .same_provenance(&self.saved),
        }
    }
}
// A cache of listing diagnostics only. Source restore/save never consumes it.
#[derive(PartialEq, Eq)]
struct RecoveryStamp {
    folder: Identity,
    files: Vec<(std::ffi::OsString, Identity)>,
}
fn recovery_stamp(folder: &Path) -> Option<RecoveryStamp> {
    let metadata = fs::symlink_metadata(folder).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let mut files = vec![];
    for entry in fs::read_dir(folder).ok()? {
        if files.len() >= 128 {
            return None;
        }
        let entry = entry.ok()?;
        let metadata = fs::symlink_metadata(entry.path()).ok()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return None;
        }
        files.push((entry.file_name(), Identity::of(&metadata)));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Some(RecoveryStamp {
        folder: Identity::of(&metadata),
        files,
    })
}
pub struct Editing {
    store: PrivateStore,
    records: Records,
    recovery_catalogue: std::cell::RefCell<HashMap<String, (RecoveryStamp, Recovery)>>,
    live: HashMap<String, Live>,
    session_settings: String,
    workspaces: HashMap<String, PathBuf>,
    library_mutation: bool,
    protected: Vec<PathBuf>,
}
fn safe_saved_name(name: &str) -> bool {
    if name == "base.bin" || name == "saved.bin" {
        return true;
    }
    let Some(raw) = name
        .strip_prefix("save-")
        .and_then(|s| s.strip_suffix(".bin"))
    else {
        return false;
    };
    let Some((revision, id)) = raw.split_once('-') else {
        return false;
    };
    revision.parse::<u64>().is_ok() && Uuid::parse_str(id).is_ok()
}
impl Editing {
    pub fn open(path: &Path) -> Result<Self, FileError> {
        let store = PrivateStore::open(path)?;
        let records = Records::load(&store)?;
        let mut result = Self {
            store,
            records,
            recovery_catalogue: Default::default(),
            live: HashMap::new(),
            session_settings: String::new(),
            workspaces: HashMap::new(),
            library_mutation: false,
            protected: vec![fs::canonicalize(path)?],
        };
        result.expire_clean()?;
        Ok(result)
    }
    pub fn library_config(&self) -> (PathBuf, Vec<PathBuf>) {
        (self.store.root.clone(), self.protected.clone())
    }
    pub fn library_reserve(&mut self) -> Result<(), FileError> {
        if !self.live.is_empty() || self.library_mutation {
            return Err(FileError::new(
                "editing-active",
                "Close all Source sessions before changing library files. Reading snapshots can stay open.",
            ));
        }
        self.library_mutation = true;
        Ok(())
    }
    pub fn relocate(&mut self, old: &Path, new: &Path) -> Result<(), FileError> {
        self.records.relocate(&self.store, old, new)
    }
    pub fn library_release(&mut self) {
        self.library_mutation = false;
    }
    pub fn protect(&mut self, path: &Path) {
        if let Ok(path) = fs::canonicalize(path) {
            self.protected.push(path);
        }
    }
    pub fn reset_settings(&mut self) -> Result<(), FileError> {
        self.records.reset(&self.store)?;
        self.session_settings.clear();
        Ok(())
    }
    pub fn warning(&self) -> Option<String> {
        self.records.warning.clone()
    }
    fn get(&self, id: &str, generation: &str) -> Result<&Live, FileError> {
        let live = self
            .live
            .get(id)
            .ok_or_else(|| FileError::new("unauthorized", "Unknown editing handle."))?;
        if live.transport != generation {
            return Err(FileError::new(
                "generation",
                "This editing generation is stale.",
            ));
        }
        Ok(live)
    }
    fn get_mut(&mut self, id: &str, generation: &str) -> Result<&mut Live, FileError> {
        self.get(id, generation)?;
        Ok(self.live.get_mut(id).expect("checked"))
    }
    fn folder(&self, id: &str) -> Result<PathBuf, FileError> {
        if Uuid::parse_str(id).map(|u| u.to_string()).ok().as_deref() != Some(id) {
            return Err(FileError::new("unauthorized", "Invalid recovery handle."));
        }
        Ok(self.store.recovery().join(id))
    }
    pub fn suggested_name(&self, id: &str, generation: &str) -> Result<String, FileError> {
        let name = &self.get(id, generation)?.name;
        Ok(
            if name.is_empty()
                || name.len() > 240
                || matches!(name.as_str(), "." | "..")
                || name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\'))
            {
                "Untitled.md".into()
            } else {
                name.clone()
            },
        )
    }
    pub fn authorize(&self, id: &str, generation: &str) -> Result<(), FileError> {
        self.get(id, generation).map(|_| ())
    }
    #[cfg(test)]
    pub fn begin(&mut self, docs: &Documents, id: &str) -> Result<EditDescriptor, FileError> {
        self.begin_at(docs, id, None)
    }
    pub fn begin_at(
        &mut self,
        docs: &Documents,
        id: &str,
        start_byte: Option<usize>,
    ) -> Result<EditDescriptor, FileError> {
        if self.library_mutation {
            return Err(FileError::new(
                "busy",
                "A library mutation is running. Wait before starting Source.",
            ));
        }
        if self.live.len() >= 8 {
            return Err(FileError::new(
                "capacity",
                "Close an editor before starting another.",
            ));
        }
        let (target, bytes, can_save, name) = docs.editing_base(id)?;
        self.store.reserve(bytes.len() as u64 * 3 + 65536)?;
        let (recovery, session) = Session::create(&self.store.recovery(), &bytes).map_err(error)?;
        drop(bytes);
        let expected = atomic_save::source_hash(&session.source().snapshot())?;
        // The captured snapshot stays recoverable after Trash/missing/revoked-path errors.
        // A failed native target validation never grants that path again: this becomes Save As only.
        let identity = target
            .validate_selection()
            .and_then(|_| target.open())
            .and_then(|f| f.metadata().map_err(Into::into))
            .ok()
            .map(|m| Identity::of(&m));
        let detached = identity.is_none();
        self.records
            .note_path(&self.store, target.canonical_path())?;
        if let Some(parent) = target.canonical_path().parent() {
            self.records.note_path(&self.store, parent)?;
        }
        let file_key = key(target.canonical_path());
        let workspace_key = key(target.canonical_path().parent().unwrap_or(Path::new("/")));
        let position = self.records.position(&file_key);
        let logical_caret =
            start_byte.map_or(position.caret, |byte| session.source().utf16_at_byte(byte));
        let (text, window) = window_text(&session, logical_caret)?;
        let caret = logical_caret
            .saturating_sub(window.as_ref().map_or(0, |w| w.from))
            .min(text.encode_utf16().count());
        let live = Live {
            saved: session.source().snapshot(),
            session,
            transport: Uuid::new_v4().to_string(),
            target: if can_save
                && identity.is_some()
                && !self
                    .protected
                    .iter()
                    .any(|root| target.canonical_path().starts_with(root))
            {
                Some(target)
            } else {
                None
            },
            expected: expected.clone(),
            identity,
            saved_revision: 0,
            name: name.clone(),
            file_key,
            workspace_key,
            checkpoint_revision: 0,
            uncertain: detached,
            window: window.clone(),
            reader: None,
        };
        let marker = Marker {
            schema: 1,
            name: name.clone(),
            saved_hash: expected,
            saved_revision: 0,
            saved_file: "base.bin".into(),
            updated: now(),
            pending: detached,
        };
        private_store::atomic(
            &self.folder(&recovery)?.join("marker.json"),
            &serde_json::to_vec(&marker)
                .map_err(|_| FileError::new("state", "Cannot encode recovery metadata."))?,
        )?;
        let descriptor = EditDescriptor {
            id: recovery.clone(),
            generation: live.transport.clone(),
            journal_generation: recovery.clone(),
            text,
            state: live.state(),
            name,
            caret,
            needs_save_as: live.target.is_none(),
            range: window,
        };
        self.live.insert(recovery, live);
        Ok(descriptor)
    }
    pub fn power_capture(
        &self,
        id: &str,
        generation: &str,
        revision: u64,
    ) -> Result<(String, Vec<u8>, Option<PathBuf>), FileError> {
        let live = self.get(id, generation)?;
        if revision != live.session.received_revision()
            || live.session.durable_revision() < revision
        {
            return Err(FileError::new(
                "revision",
                "Wait for the working revision's durable ACK before capture.",
            ));
        }
        let snapshot = live.session.source().snapshot();
        if snapshot.byte_len() > crate::power::CAPTURE_LIMIT {
            return Err(FileError::new(
                "capacity",
                "Export/run capture admits 2 MiB. The full editor is kept.",
            ));
        }
        let mut bytes = Vec::with_capacity(snapshot.byte_len());
        snapshot.write_to(&mut bytes)?;
        Ok((
            live.name.clone(),
            bytes,
            live.target.as_ref().map(|t| t.canonical_path().to_owned()),
        ))
    }
    pub fn capture_read(
        &mut self,
        id: &str,
        generation: &str,
        revision: u64,
    ) -> Result<String, FileError> {
        let live = self.get_mut(id, generation)?;
        if revision != live.session.received_revision() {
            return Err(FileError::new(
                "revision",
                "Wait for the working revision ACK.",
            ));
        }
        let token = Uuid::new_v4().to_string();
        live.reader = Some((token.clone(), live.session.source().snapshot()));
        Ok(token)
    }
    pub fn read_snapshot(
        &self,
        id: &str,
        generation: &str,
        token: &str,
        start: usize,
    ) -> Result<crate::files::SourceRange, FileError> {
        let live = self.get(id, generation)?;
        let (_, snapshot) = live
            .reader
            .as_ref()
            .filter(|(key, _)| key == token)
            .ok_or_else(|| FileError::new("snapshot", "Working snapshot was replaced."))?;
        let (start, end, text) = snapshot
            .read_range(start, 65536)
            .map_err(|_| FileError::new("range", "Invalid working snapshot range."))?;
        Ok(crate::files::SourceRange {
            start: start as u64,
            end: end as u64,
            total: snapshot.byte_len() as u64,
            text,
        })
    }
    pub fn byte_anchor(&self, id: &str, generation: &str, byte: usize) -> Result<usize, FileError> {
        Ok(self
            .get(id, generation)?
            .session
            .source()
            .utf16_at_byte(byte))
    }
    pub fn change_window(
        &mut self,
        id: &str,
        generation: &str,
        revision: u64,
        start: usize,
    ) -> Result<EditDescriptor, FileError> {
        let live = self.get(id, generation)?;
        if live.window.is_none() || revision != live.session.received_revision() {
            return Err(FileError::new(
                "revision",
                "Flush current range before switching.",
            ));
        }
        self.flush(id, generation)?;
        let live = self.get_mut(id, generation)?;
        let (text, range) = window_text(&live.session, start)?;
        live.window = range.clone();
        live.transport = Uuid::new_v4().to_string();
        live.reader = None;
        // Keep replay budgets identical to live budgets: large journals do not checkpoint.
        // Window-local frontend undo resets, but native replay receipts/tokens remain bounded.

        Ok(EditDescriptor {
            id: id.into(),
            generation: live.transport.clone(),
            journal_generation: id.into(),
            text,
            state: live.state(),
            name: live.name.clone(),
            caret: 0,
            needs_save_as: live.target.is_none(),
            range,
        })
    }
    pub fn apply(
        &mut self,
        id: &str,
        generation: &str,
        batches: Vec<Batch>,
        retained: Vec<String>,
    ) -> Result<Applied, FileError> {
        self.authorize(id, generation)?;
        if batches.is_empty()
            || batches.len() > 64
            || retained.len() > 16384
            || serde_json::to_vec(&batches)
                .map_err(|_| FileError::new("edit", "Invalid edits."))?
                .len()
                > 2 * 1024 * 1024
        {
            return Err(FileError::new(
                "capacity",
                "Queued edits exceeded the IPC budget. Input remains open.",
            ));
        }
        self.store.reserve(3 * 1024 * 1024)?;
        let root = self.store.recovery();
        let live = self.get_mut(id, generation)?;
        if live.session.is_poisoned() {
            live.session.repair(&root, id).map_err(error)?;
        }
        let mut keep: HashSet<String> = retained.into_iter().collect();
        for b in &batches {
            keep.insert(b.operation_id.clone());
            for c in &b.changes {
                if let crate::document::Insert::Restore { token } = &c.insert
                    && let Some((op, _)) = token.split_once(':')
                {
                    keep.insert(op.into());
                }
            }
        }
        if live.window.is_none() {
            live.session.retain_operations(&keep);
        }
        let mut acknowledgements = vec![];
        for batch in batches {
            if let Some(w) = &live.window
                && batch.new_revision > live.session.received_revision()
                && batch.changes.iter().any(|c| c.from < w.from || c.to > w.to)
            {
                return Err(FileError::new(
                    "range",
                    "Edits cannot cross the active range boundary.",
                ));
            }
            let before = live.session.source().len_utf16();
            let ack = live.session.append(batch).map_err(error)?;
            let after = live.session.source().len_utf16();
            if let Some(w) = &mut live.window {
                w.to = w.to + after - before;
                w.total = after;
            }
            acknowledgements.push(ack);
        }
        Ok(Applied {
            acknowledgements,
            state: live.state(),
        })
    }
    pub fn flush(&mut self, id: &str, generation: &str) -> Result<EditState, FileError> {
        let root = self.store.recovery();
        let folder = self.folder(id)?;
        let current = self.get(id, generation)?;
        if current.session.durable_revision() == current.session.received_revision()
            && !current.session.is_poisoned()
        {
            return Ok(current.state());
        }
        self.store.reserve(1024 * 1024)?;
        let should = {
            let live = self.get(id, generation)?;
            live.window.is_none()
                && (live.session.received_revision() - live.checkpoint_revision
                    >= CHECKPOINT_INTERVAL
                    || live.session.log_bytes() > 1024 * 1024)
        };
        if should {
            let estimate = self
                .get(id, generation)?
                .session
                .checkpoint_size()
                .map_err(error)?;
            self.store.reserve(estimate as u64 + 65536)?;
        }
        let live = self.get_mut(id, generation)?;
        if live.session.is_poisoned() {
            live.session.repair(&root, id).map_err(error)?;
        }
        live.session.flush().map_err(error)?;
        if should {
            live.session.checkpoint(&root, id).map_err(error)?;
            live.checkpoint_revision = live.session.received_revision();
        }
        // Directory sync makes newly created checkpoint/log names durable too.
        File::open(folder)?.sync_all()?;
        Ok(live.state())
    }
    pub fn save(
        &mut self,
        id: &str,
        generation: &str,
        revision: u64,
        destination: Option<&Path>,
        pending: Vec<Batch>,
    ) -> Result<Saved, FileError> {
        self.authorize(id, generation)?;
        if pending.len() > 4096
            || serde_json::to_vec(&pending)
                .map_err(|_| FileError::new("edit", "Invalid pending edits."))?
                .len()
                > 2 * 1024 * 1024
        {
            return Err(FileError::new(
                "capacity",
                "Recovery export exceeded the IPC budget.",
            ));
        }
        let folder = self.folder(id)?;
        let reserve = self.store.reserve(
            if self.get(id, generation)?.window.is_some() {
                160
            } else {
                65
            } * 1024
                * 1024,
        );
        let protected = self.protected.clone();
        let live = self.get_mut(id, generation)?;
        if let Some(w) = &live.window {
            let mut end = w.to;
            for b in &pending {
                if b.base_revision < live.session.received_revision() {
                    continue;
                }
                if b.changes.iter().any(|c| c.from < w.from || c.to > end) {
                    return Err(FileError::new(
                        "range",
                        "Pending edits cross the active window.",
                    ));
                }
                // Restore lengths are validated by the native preview. Reject uncertain rescues;
                // ordinary acknowledged undo/save remains available.
                for c in &b.changes {
                    match &c.insert {
                        crate::document::Insert::Text { text } => {
                            let length = text
                                .replace("\r\n", "\n")
                                .replace('\r', "\n")
                                .encode_utf16()
                                .count();
                            end = end + length - (c.to - c.from);
                        }
                        _ => return Err(FileError::new("range", "ACK range undo before Save As.")),
                    }
                }
            }
        }
        let rescue = destination.is_some()
            && (!pending.is_empty() || live.session.is_poisoned() || reserve.is_err());
        let source = if rescue {
            live.session.preview(&pending).map_err(error)?
        } else {
            reserve?;
            live.session.capture_save(revision).map_err(error)?.source
        };
        let target = if let Some(path) = destination {
            Arc::new(ScopedFile::destination(path)?)
        } else {
            live.target.clone().ok_or_else(|| {
                FileError::new("save-as", "Use Save As to select a new destination.")
            })?
        };
        if protected
            .iter()
            .any(|root| target.canonical_path().starts_with(root))
        {
            return Err(FileError::new(
                "private-storage",
                "App-private records cannot be selected as save destinations.",
            ));
        }
        if !rescue {
            let mut marker: Marker = private_store::read(&folder.join("marker.json"), 65536)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Marker {
                    schema: 1,
                    name: live.name.clone(),
                    saved_hash: live.expected.clone(),
                    saved_revision: live.saved_revision,
                    saved_file: "base.bin".into(),
                    updated: now(),
                    pending: false,
                });
            marker.pending = true;
            marker.updated = now();
            private_store::atomic(
                &folder.join("marker.json"),
                &serde_json::to_vec(&marker)
                    .map_err(|_| FileError::new("state", "Cannot encode pending commit."))?,
            )?;
            live.uncertain = true;
        }
        let attempt = if destination.is_some() {
            atomic_save::save_as(&target, &source)
        } else {
            let identity = live
                .identity
                .as_ref()
                .ok_or_else(|| FileError::new("save-as", "Use Save As to select a destination."))?;
            atomic_save::save(&target, &source, &live.expected, identity, |disk, _| {
                disk.rewind()?;
                let mut bytes = vec![];
                Read::by_ref(disk)
                    .take(160 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 160 * 1024 * 1024 {
                    return Err(FileError::new(
                        "capacity",
                        "Disk conflict snapshot exceeded the ordinary-file budget; no overwrite occurred.",
                    ));
                }
                private_store::atomic(&folder.join("conflict.bin"), &bytes)
            })
        };
        let committed = match attempt {
            Ok(committed) => committed,
            Err(error) => {
                // The rename/link may have committed before a later error. Adopt only
                // bytes equal to this exact save input, never an unrelated disk revision.
                if destination.is_none()
                    && error.code == "durability"
                    && let Ok((_, hash, _)) = target.snapshot()
                    && atomic_save::source_hash(&source).ok().as_deref() == Some(hash.as_str())
                    && let Ok(file) = target.open()
                {
                    live.expected = hash;
                    live.identity = Some(Identity::of(&file.metadata()?));
                    live.target = Some(target.clone());
                }
                return Err(error);
            }
        };
        let name = target
            .canonical_path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if rescue {
            return Ok(Saved {
                revision,
                noop: false,
                rescue: true,
                name,
                state: live.state(),
            });
        }
        // A committed user file may outlive a failed app-record sync. Advance the
        // expected disk hash immediately, but keep the old saved ACK until records sync.
        if destination.is_none() {
            live.expected = committed.hash.clone();
            live.identity = Some(committed.identity.clone());
        }

        let mut bytes = vec![];
        source.write_to(&mut bytes)?;
        let saved_file = format!("save-{revision}-{}.bin", Uuid::new_v4());
        private_store::atomic(&folder.join(&saved_file),&bytes).map_err(|_|FileError::new("durability","The file or copy was written, but recovery metadata could not sync. A failed Save As keeps the original editing target. Keep the open text; retry or reopen the copy."))?;
        let marker = Marker {
            schema: 1,
            name: name.clone(),
            saved_hash: committed.hash.clone(),
            saved_revision: revision,
            saved_file: saved_file.clone(),
            updated: now(),
            pending: false,
        };
        private_store::atomic(
            &folder.join("marker.json"),
            &serde_json::to_vec(&marker)
                .map_err(|_| FileError::new("state", "Cannot encode saved metadata."))?,
        )?;
        live.expected = committed.hash.clone();
        live.identity = Some(committed.identity);
        live.target = Some(target.clone());
        live.uncertain = false;
        live.saved = source;
        live.saved_revision = revision;
        live.name = name.clone();
        live.file_key = key(target.canonical_path());
        live.workspace_key = key(target.canonical_path().parent().unwrap_or(Path::new("/")));
        let mut backups = fs::read_dir(&folder)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("save-") && name.ends_with(".bin") {
                    Some((entry.metadata().ok()?.modified().ok()?, entry.path(), name))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        backups.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (_, path, _) in backups
            .into_iter()
            .filter(|(_, _, name)| name != &saved_file)
            .skip(2)
        {
            let _ = fs::remove_file(path);
        }
        Ok(Saved {
            revision,
            noop: committed.noop,
            rescue: false,
            name,
            state: live.state(),
        })
    }
    pub fn status(&self, id: &str, generation: &str) -> Result<&'static str, FileError> {
        let live = self.get(id, generation)?;
        let Some(target) = &live.target else {
            return Ok("unchanged");
        };
        if let Err(e) = target.validate_selection() {
            return match e.code {
                "missing" => Ok("missing"),
                "changed" => Ok("changed"),
                _ => Err(e),
            };
        }
        let mut file = match target.open() {
            Ok(f) => f,
            Err(e) if e.code == "missing" => return Ok("missing"),
            Err(e) if e.code == "changed" => return Ok("changed"),
            Err(e) => return Err(e),
        };
        Ok(
            if crate::scoped_file::fingerprint(&mut file, None)?.0 == live.expected {
                "unchanged"
            } else {
                "changed"
            },
        )
    }
    pub fn compare(&self, id: &str, generation: &str) -> Result<Comparison, FileError> {
        self.store.reserve(160 * 1024 * 1024)?;
        let live = self.get(id, generation)?;
        let mut base = vec![];
        live.saved.write_to(&mut base)?;
        let mut missing = false;
        let disk = if let Some(target) = &live.target {
            match target
                .validate_selection()
                .and_then(|_| target.snapshot_bounded(160 * 1024 * 1024))
            {
                Ok((f, _, _)) => {
                    let mut b = vec![];
                    f.take(160 * 1024 * 1024 + 1).read_to_end(&mut b)?;
                    if b.len() > 160 * 1024 * 1024 {
                        return Err(FileError::new(
                            "capacity",
                            "Disk comparison exceeded the ordinary-file budget.",
                        ));
                    }
                    private_store::atomic(&self.folder(id)?.join("conflict.bin"), &b)?;
                    b
                }
                Err(e) if e.code == "missing" => {
                    missing = true;
                    vec![]
                }
                Err(e) => return Err(e),
            }
        } else {
            vec![]
        };
        fn excerpt(bytes: &[u8]) -> String {
            String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]).into_owned()
        }
        Ok(Comparison {
            base: excerpt(&base),
            disk: excerpt(&disk),
            clipped: base.len() > 65536 || disk.len() > 65536,
            missing,
        })
    }
    #[cfg(test)]
    pub fn list(&self) -> Result<Vec<Recovery>, FileError> {
        self.list_with_saved(false)
    }
    pub fn list_with_saved(&self, include_saved: bool) -> Result<Vec<Recovery>, FileError> {
        let mut result = vec![];
        let mut seen = HashSet::new();
        for entry in fs::read_dir(self.store.recovery())? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if self.live.contains_key(&id) || Uuid::parse_str(&id).is_err() {
                continue;
            }
            seen.insert(id.clone());
            let before = recovery_stamp(&entry.path());
            let cached = before.as_ref().and_then(|stamp| {
                self.recovery_catalogue
                    .borrow()
                    .get(&id)
                    .filter(|(cached, _)| cached == stamp)
                    .map(|(_, row)| row.clone())
            });
            let row = if let Some(row) = cached {
                row
            } else {
                self.recovery_catalogue.borrow_mut().remove(&id);
                let marker = private_store::read(&entry.path().join("marker.json"), 65536)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Marker>(&b).ok());
                let validated = Session::zero_history_summary(&self.store.recovery(), &id)
                    .and_then(|zero| {
                        if let Some(hash) = zero {
                            Ok((0, hash))
                        } else {
                            let session = Session::recover(&self.store.recovery(), &id)?;
                            let hash = atomic_save::source_hash(&session.source().snapshot())
                                .map_err(|e| JournalError::Io(std::io::Error::other(e.message)))?;
                            Ok((session.received_revision(), hash))
                        }
                    });
                let row = match validated {
                    Ok((revision, hash)) => {
                        let dirty = marker
                            .as_ref()
                            .is_none_or(|m| m.pending || hash != m.saved_hash);
                        Recovery {
                            id: id.clone(),
                            name: marker
                                .as_ref()
                                .map_or("Recovered document", |m| m.name.as_str())
                                .into(),
                            revision,
                            updated: marker.map_or(0, |m| m.updated),
                            damaged: false,
                            clean: !dirty,
                        }
                    }
                    Err(_) => Recovery {
                        id: id.clone(),
                        name: "Recovery needs inspection (records preserved)".into(),
                        revision: 0,
                        updated: 0,
                        damaged: true,
                        clean: false,
                    },
                };
                // Cache only a complete, unchanged flat-file fingerprint around
                // complete-byte replay/zero-history validation. Never trust a marker alone.
                if let Some(stamp) = before
                    && !row.damaged
                    && recovery_stamp(&entry.path()).as_ref() == Some(&stamp)
                {
                    let mut cache = self.recovery_catalogue.borrow_mut();
                    if cache.len() >= 64
                        && let Some(key) = cache.keys().next().cloned()
                    {
                        cache.remove(&key);
                    }
                    cache.insert(id, (stamp, row.clone()));
                }
                row
            };
            if !row.clean || include_saved {
                result.push(row);
            }
        }
        self.recovery_catalogue
            .borrow_mut()
            .retain(|id, _| seen.contains(id));
        Ok(result)
    }
    pub fn create_note(
        &mut self,
        title: &str,
        template: &str,
    ) -> Result<EditDescriptor, FileError> {
        if self.library_mutation || self.live.len() >= 8 {
            return Err(FileError::new(
                "capacity",
                "Wait for file operations or close an editor before creating a note.",
            ));
        }
        let title = title.trim();
        if title.is_empty()
            || title.len() > 160
            || title
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
            || matches!(title, "." | "..")
        {
            return Err(FileError::new(
                "title",
                "Use a short note title without slashes or control characters.",
            ));
        }
        let text = match template {
            "blank" => String::new(),
            "note" => format!("# {title}\n\n## Notes\n\n\n## Next steps\n\n- [ ] \n"),
            "meeting" => format!(
                "# {title}\n\n## Attendees\n\n- \n\n## Agenda\n\n- \n\n## Notes\n\n\n## Decisions\n\n- \n\n## Action items\n\n- [ ] \n"
            ),
            "journal" => {
                format!("# {title}\n\n## Priorities\n\n- [ ] \n\n## Notes\n\n\n## Reflection\n\n")
            }
            _ => {
                return Err(FileError::new(
                    "template",
                    "Choose a built-in note template.",
                ));
            }
        };
        self.store.reserve(text.len() as u64 * 3 + 65536)?;
        let (id, session) =
            Session::create(&self.store.recovery(), text.as_bytes()).map_err(error)?;
        let marker = Marker {
            schema: 1,
            name: format!("{title}.md"),
            saved_hash: format!("{:x}", Sha256::digest(text.as_bytes())),
            saved_revision: 0,
            saved_file: "base.bin".into(),
            updated: now(),
            pending: true,
        };
        private_store::atomic(
            &self.folder(&id)?.join("marker.json"),
            &serde_json::to_vec(&marker)
                .map_err(|_| FileError::new("state", "Cannot encode note recovery metadata."))?,
        )?;
        drop(session);
        self.restore(&id)
    }
    pub fn restore(&mut self, id: &str) -> Result<EditDescriptor, FileError> {
        if self.library_mutation {
            return Err(FileError::new(
                "busy",
                "Wait for the library mutation before restoring Source.",
            ));
        }
        if self.live.contains_key(id) {
            return Err(FileError::new("state", "This recovery is already open."));
        }
        let folder = self.folder(id)?;
        let session = Session::recover(&self.store.recovery(), id).map_err(error)?;
        let marker: Option<Marker> = private_store::read(&folder.join("marker.json"), 65536)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let name = marker
            .as_ref()
            .map_or("Recovered document", |m| m.name.as_str())
            .to_owned();
        let saved_bytes = marker
            .as_ref()
            .and_then(|m| {
                if safe_saved_name(&m.saved_file) {
                    private_store::read(&folder.join(&m.saved_file), 160 * 1024 * 1024)
                        .ok()
                        .filter(|bytes| format!("{:x}", Sha256::digest(bytes)) == m.saved_hash)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let saved = crate::document::Provenance::new(&saved_bytes)
            .map_err(|_| FileError::new("recovery", "Saved base encoding is invalid."))?
            .snapshot();
        let (text, window) = window_text(&session, 0)?;
        let revision = session.received_revision();
        let live = Live {
            session,
            transport: Uuid::new_v4().to_string(),
            target: None,
            expected: marker
                .as_ref()
                .map_or_else(String::new, |m| m.saved_hash.clone()),
            identity: None,
            saved,
            saved_revision: marker.map_or(0, |m| m.saved_revision),
            name: name.clone(),
            file_key: format!("recovery:{id}"),
            workspace_key: "recovery".into(),
            checkpoint_revision: revision,
            uncertain: true,
            window: window.clone(),
            reader: None,
        };
        let descriptor = EditDescriptor {
            id: id.into(),
            generation: live.transport.clone(),
            journal_generation: id.into(),
            text,
            state: live.state(),
            name,
            caret: 0,
            needs_save_as: true,
            range: window,
        };
        self.live.insert(id.into(), live);
        Ok(descriptor)
    }
    pub fn release(&mut self, id: &str, generation: &str, discard: bool) -> Result<(), FileError> {
        let live = self.get(id, generation)?;
        let untouched = !discard
            && live.target.is_some()
            && live.session.received_revision() == 0
            && !live.state().dirty
            && self
                .status(id, generation)
                .is_ok_and(|status| status == "unchanged");
        let redundant_base = untouched.then(|| live.expected.clone());
        if !discard {
            self.flush(id, generation)?;
        }
        self.live.remove(id);
        if discard {
            fs::remove_dir_all(self.folder(id)?)?;
            File::open(self.store.recovery())?.sync_all()?;
        } else if let Some(expected) = redundant_base {
            // Housekeeping cannot turn a successful close into a stale editor.
            // Any failed proof/retirement keeps the original recovery or cache bytes.
            let _ = self.store.retire_unused_source(id, &expected);
            self.recovery_catalogue.borrow_mut().remove(id);
        }
        Ok(())
    }
    pub fn discard(&mut self, id: &str) -> Result<(), FileError> {
        if self.live.contains_key(id) {
            return Err(FileError::new(
                "state",
                "Close this editor before discarding its recovery.",
            ));
        }
        let folder = self.folder(id)?;
        fs::remove_dir_all(folder)?;
        File::open(self.store.recovery())?.sync_all()?;
        Ok(())
    }
    fn expire_clean(&mut self) -> Result<(), FileError> {
        for entry in fs::read_dir(self.store.recovery())? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if Uuid::parse_str(&id).is_err() {
                continue;
            }
            if let Ok(bytes) = private_store::read(&entry.path().join("marker.json"), 65536)
                && let Ok(marker) = serde_json::from_slice::<Marker>(&bytes)
                && marker.schema == 1
                && !marker.pending
                && now().saturating_sub(marker.updated) > 7 * 24 * 3600
                && let Ok(session) = Session::recover(&self.store.recovery(), &id)
                && atomic_save::source_hash(&session.source().snapshot())? == marker.saved_hash
            {
                drop(session);
                fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }
    pub fn workspace(&mut self, id: &str, root: Option<PathBuf>) {
        if let Some(root) = root {
            self.workspaces.insert(id.into(), root);
        } else {
            self.workspaces.remove(id);
        }
        if self.workspaces.len() > 16 {
            self.workspaces.retain(|key, _| key == id);
        }
    }
    pub fn preview_settings(
        &self,
        docs: &Documents,
        id: Option<&str>,
        scope: &str,
        text: &str,
    ) -> Result<SettingsResult, FileError> {
        let (f, w) = self.record_keys(docs, id)?;
        if ["workspace", "file"].contains(&scope)
            && crate::settings::Settings::parse(text.as_bytes()).is_ok_and(|s| {
                s.values()
                    .keys()
                    .any(|k| k.starts_with("shortcut.") || k == "library.width")
            })
        {
            let mut result = self.settings(docs, id, scope)?;
            result.diagnostics = vec![crate::settings::Diagnostic {
                line: 1,
                message: "Shortcuts and library layout are global or session-only preferences."
                    .into(),
            }];
            return Ok(result);
        }
        let mut result = if scope == "session" {
            self.records.settings("global", &f, &w)?
        } else {
            self.records.preview(scope, &f, &w, text)?
        };
        let session_text = if scope == "session" {
            text
        } else {
            &self.session_settings
        };
        match crate::settings::Settings::parse(session_text.as_bytes()) {
            Ok(s) => {
                result.resolved.extend(s.values());
                if scope == "session" {
                    result.text = text.into();
                }
                if result.diagnostics.is_empty() {
                    result.diagnostics = crate::preferences::effective(&result.resolved);
                }
            }
            Err(diagnostics) => result.diagnostics = diagnostics,
        }
        Ok(result)
    }
    pub fn settings(
        &self,
        docs: &Documents,
        id: Option<&str>,
        scope: &str,
    ) -> Result<SettingsResult, FileError> {
        let (f, w) = self.record_keys(docs, id)?;
        let mut result =
            self.records
                .settings(if scope == "session" { "global" } else { scope }, &f, &w)?;
        let session = crate::settings::Settings::parse(self.session_settings.as_bytes())
            .map_err(|_| FileError::new("settings", "Invalid session settings."))?;
        result.resolved.extend(session.values());
        if scope == "session" {
            result.text = self.session_settings.clone();
        }
        result.diagnostics = crate::preferences::effective(&result.resolved);
        if !result.diagnostics.is_empty() {
            // A different file/layer can invalidate an older effective palette or binding.
            // Preserve every stored literal for repair; only the applied values fall back.
            result.resolved = crate::settings::Settings::defaults().values();
        }
        Ok(result)
    }
    pub fn update_settings(
        &mut self,
        docs: &Documents,
        id: Option<&str>,
        scope: &str,
        text: &str,
    ) -> Result<SettingsResult, FileError> {
        let preview = self.preview_settings(docs, id, scope, text)?;
        if !preview.diagnostics.is_empty() {
            return Ok(preview);
        }
        if scope == "session" {
            if let Err(diagnostics) = crate::settings::Settings::parse(text.as_bytes()) {
                let mut result = self.settings(docs, id, scope)?;
                result.diagnostics = diagnostics;
                return Ok(result);
            }
            self.session_settings = text.into();
            return self.settings(docs, id, scope);
        }
        let (f, w) = self.record_keys(docs, id)?;
        if let Some(id) = id {
            let path = docs.scoped(id)?.canonical_path().to_owned();
            self.records.note_path(&self.store, &path)?;
            if let Some(parent) = path.parent() {
                self.records.note_path(&self.store, parent)?;
            }
            if let Some(root) = self.workspaces.get(id) {
                self.records.note_path(&self.store, root)?;
            }
        }
        let result = self.records.update(&self.store, scope, &f, &w, text)?;
        if !result.diagnostics.is_empty() {
            return Ok(result);
        }
        self.settings(docs, id, scope)
    }
    fn record_keys(
        &self,
        docs: &Documents,
        id: Option<&str>,
    ) -> Result<(String, String), FileError> {
        if let Some(id) = id {
            let t = docs.scoped(id)?;
            Ok((
                key(t.canonical_path()),
                key(self.workspaces.get(id).map_or_else(
                    || t.canonical_path().parent().unwrap_or(Path::new("/")),
                    |p| p.as_path(),
                )),
            ))
        } else {
            Ok(("session".into(), "session".into()))
        }
    }
    pub fn position(&self, docs: &Documents, id: &str) -> Result<Position, FileError> {
        let (f, _) = self.record_keys(docs, Some(id))?;
        Ok(self.records.position(&f))
    }
    pub fn remember(
        &mut self,
        docs: &Documents,
        id: &str,
        position: Position,
    ) -> Result<(), FileError> {
        let (f, _) = self.record_keys(docs, Some(id))?;
        if position.byte > docs.byte_length(id)?
            || position.caret > 160 * 1024 * 1024
            || position
                .anchor
                .as_ref()
                .is_some_and(|a| a.len() > 1024 || a.chars().any(char::is_control))
        {
            return Err(FileError::new(
                "range",
                "Reading position exceeds the document budget.",
            ));
        }
        let path = docs.scoped(id)?.canonical_path().to_owned();
        self.records.note_path(&self.store, &path)?;
        self.records.remember(&self.store, &f, position)
    }
    pub fn flush_all(&mut self) -> Result<(), FileError> {
        let ids = self
            .live
            .iter()
            .map(|(id, l)| (id.clone(), l.transport.clone()))
            .collect::<Vec<_>>();
        for (id, g) in ids {
            self.flush(&id, &g)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Change, Insert};
    fn fixture() -> (
        tempfile::TempDir,
        PathBuf,
        Documents,
        crate::files::Descriptor,
        Editing,
        EditDescriptor,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        fs::write(&path, "\u{feff}a\r\nb\nc\r🦀").unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let mut editing = Editing::open(&dir.path().join("private")).unwrap();
        let edit = editing.begin(&docs, &doc.id).unwrap();
        (dir, path, docs, doc, editing, edit)
    }
    #[test]
    fn new_notes_are_durable_pathless_and_require_an_exclusive_save_as() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private");
        let mut service = Editing::open(&private).unwrap();
        for template in ["blank", "note", "meeting", "journal"] {
            let note = service.create_note("Project notes", template).unwrap();
            assert!(note.needs_save_as && note.state.dirty);
            assert_eq!(note.name, "Project notes.md");
            assert_eq!(
                service.suggested_name(&note.id, &note.generation).unwrap(),
                "Project notes.md"
            );
            assert!(
                service
                    .save(&note.id, &note.generation, 0, None, vec![])
                    .is_err()
            );
            service.release(&note.id, &note.generation, false).unwrap();
            let restored = service.restore(&note.id).unwrap();
            assert_eq!(restored.text, note.text);
            assert!(restored.needs_save_as);
            let target = dir.path().join(format!("{template}.md"));
            service
                .save(&restored.id, &restored.generation, 0, Some(&target), vec![])
                .unwrap();
            assert_eq!(fs::read_to_string(&target).unwrap(), note.text);
            service
                .release(&restored.id, &restored.generation, true)
                .unwrap();
        }
        for title in ["", "../private", "a/b", "a\\b", "bad\nname"] {
            assert!(service.create_note(title, "note").is_err());
        }
        assert!(service.create_note("Valid", "unknown").is_err());
        service.library_reserve().unwrap();
        assert!(service.create_note("Valid", "note").is_err());
    }
    fn retain_edited_history(service: &mut Editing, editor: &EditDescriptor) {
        let ack = service
            .apply(
                &editor.id,
                &editor.generation,
                vec![batch(editor, 1, vec![text(0, 0, "x")])],
                vec![],
            )
            .unwrap();
        let undone = service
            .apply(
                &editor.id,
                &editor.generation,
                vec![batch(editor, 2, ack.acknowledgements[0].inverse.clone())],
                vec!["op1".into()],
            )
            .unwrap();
        assert!(!undone.state.dirty);
    }
    #[test]
    fn untouched_close_retires_only_redundant_authorized_bases_and_keeps_undo_history() {
        let (_dir, path, docs, doc, mut service, editor) = fixture();
        let original = fs::read(&path).unwrap();
        service
            .release(&editor.id, &editor.generation, false)
            .unwrap();
        assert!(!service.folder(&editor.id).unwrap().exists());
        assert!(service.list_with_saved(true).unwrap().is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        let edited = service.begin(&docs, &doc.id).unwrap();
        retain_edited_history(&mut service, &edited);
        service
            .release(&edited.id, &edited.generation, false)
            .unwrap();
        assert!(service.folder(&edited.id).unwrap().exists());
        assert!(service.list_with_saved(true).unwrap()[0].clean);
        let restored = service.restore(&edited.id).unwrap();
        assert!(restored.needs_save_as);
        assert_eq!(restored.text, "a\nb\nc\n🦀");
        service
            .release(&restored.id, &restored.generation, false)
            .unwrap();
        assert!(service.folder(&edited.id).unwrap().exists());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn untouched_retirement_keeps_changed_targets_failed_proofs_and_occupied_cache_slots() {
        for case in ["changed", "proof failure", "occupied"] {
            let (dir, path, _docs, _doc, mut service, editor) = fixture();
            let original = fs::read(&path).unwrap();
            let folder = service.folder(&editor.id).unwrap();
            let destination = service
                .store
                .root
                .join("cache")
                .join(format!("unused-source-{}", editor.id));
            match case {
                "changed" => fs::write(&path, b"external revision").unwrap(),
                "proof failure" => private_store::fail_next("unused-source"),
                _ => {
                    private_store::directory(&destination).unwrap();
                    fs::write(destination.join("canary"), b"keep unrelated cache").unwrap();
                }
            }
            service
                .release(&editor.id, &editor.generation, false)
                .unwrap();
            assert_eq!(fs::read(folder.join("base.bin")).unwrap(), original);
            drop(service);
            let mut service = Editing::open(&dir.path().join("private")).unwrap();
            if case == "occupied" {
                assert_eq!(
                    fs::read(destination.join("canary")).unwrap(),
                    b"keep unrelated cache"
                );
            }
            let recovered = service.restore(&editor.id).unwrap();
            assert!(recovered.needs_save_as);
            assert_eq!(recovered.text, "a\nb\nc\n🦀");
            assert_eq!(
                fs::read(&path).unwrap(),
                if case == "changed" {
                    b"external revision".to_vec()
                } else {
                    original
                }
            );
        }
    }
    #[test]
    fn retirement_cache_replays_zero_history_proof_and_refuses_receipt_bearing_records() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path, _docs, _doc, mut service, editor) = fixture();
        let original = fs::read(&path).unwrap();
        let cache = service.store.root.join("cache");
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o500)).unwrap();
        service
            .release(&editor.id, &editor.generation, false)
            .unwrap();
        let folder = service.folder(&editor.id).unwrap();
        assert!(folder.join("unused-source.json").is_file());
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
        let retired = cache.join(format!("unused-source-{}", editor.id));
        fs::rename(&folder, &retired).unwrap();
        drop(service);
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        assert!(!retired.exists());
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let edited = service.begin(&docs, &doc.id).unwrap();
        retain_edited_history(&mut service, &edited);
        service
            .release(&edited.id, &edited.generation, false)
            .unwrap();
        let folder = service.folder(&edited.id).unwrap();
        let journal = fs::read(folder.join("journal.jsonl")).unwrap();
        let proof = serde_json::json!({"schema":1,"id":edited.id,"baseHash":format!("{:x}",Sha256::digest(&original)),"journalHash":format!("{:x}",Sha256::digest(&journal))});
        private_store::atomic(
            &folder.join("unused-source.json"),
            &serde_json::to_vec(&proof).unwrap(),
        )
        .unwrap();
        let retired = cache.join(format!("unused-source-{}", edited.id));
        fs::rename(&folder, &retired).unwrap();
        drop(service);
        let _service = Editing::open(&dir.path().join("private")).unwrap();
        assert_eq!(fs::read(retired.join("base.bin")).unwrap(), original);
        assert_eq!(fs::read(retired.join("journal.jsonl")).unwrap(), journal);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn repeated_untouched_hundred_mib_source_does_not_fill_the_private_quota() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        fs::write(&path, vec![b'x'; 100 * 1024 * 1024]).unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        for _ in 0..4 {
            let editor = service.begin(&docs, &doc.id).unwrap();
            assert!(editor.range.is_some());
            assert_eq!(editor.state.received_revision, 0);
            service
                .release(&editor.id, &editor.generation, false)
                .unwrap();
            assert!(!service.folder(&editor.id).unwrap().exists());
            assert_eq!(fs::read_dir(service.store.recovery()).unwrap().count(), 0);
        }
        assert_eq!(docs.status(&doc.id).unwrap(), "unchanged");
        assert!(service.list_with_saved(true).unwrap().is_empty());
    }
    #[test]
    fn recovery_catalogue_repeated_scan_observation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalogue-profile.txt");
        let bytes = vec![b'x'; 8 * 1024 * 1024];
        fs::write(&path, &bytes).unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        let editor = service.begin(&docs, &doc.id).unwrap();
        retain_edited_history(&mut service, &editor);
        service
            .release(&editor.id, &editor.generation, false)
            .unwrap();
        let mut times = vec![];
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let entries = service.list_with_saved(true).unwrap();
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(entries.len(), 1);
            assert!(entries[0].clean && !entries[0].damaged);
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
        if let Some(output) = std::env::var_os("MARKTUB_RECOVERY_BENCH_OUTPUT") {
            fs::write(output, serde_json::to_vec_pretty(&serde_json::json!({
                "scope":"isolated native Rust debug; repeated validated listing of one closed 8 MiB archive; not app/input/paint latency",
                "bytes":bytes.len(),"milliseconds":times
            })).unwrap()).unwrap();
        }
    }
    #[test]
    fn recovery_catalogue_invalidates_changes_and_restore_never_uses_cached_authority() {
        let (_dir, path, _docs, _doc, mut service, editor) = fixture();
        retain_edited_history(&mut service, &editor);
        service
            .release(&editor.id, &editor.generation, false)
            .unwrap();
        assert!(service.list_with_saved(true).unwrap()[0].clean);
        assert!(service.list().unwrap().is_empty());
        let marker_path = service.folder(&editor.id).unwrap().join("marker.json");
        let mut marker: Marker = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        marker.pending = true;
        fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        assert!(!service.list().unwrap()[0].clean);
        let restored = service.restore(&editor.id).unwrap();
        assert!(restored.needs_save_as);
        assert!(service.list_with_saved(true).unwrap().is_empty());
        assert!(
            service
                .save(
                    &restored.id,
                    &restored.generation,
                    restored.state.received_revision,
                    None,
                    vec![]
                )
                .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), "\u{feff}a\r\nb\nc\r🦀".as_bytes());
    }
    #[test]
    fn recovery_catalogue_cannot_hide_corruption_or_follow_replaced_symlinks() {
        for symlink in [false, true] {
            let (_dir, path, _docs, _doc, mut service, editor) = fixture();
            retain_edited_history(&mut service, &editor);
            service
                .release(&editor.id, &editor.generation, false)
                .unwrap();
            assert!(service.list_with_saved(true).unwrap()[0].clean);
            let base = service.folder(&editor.id).unwrap().join("base.bin");
            let original = fs::read(&path).unwrap();
            if symlink {
                fs::remove_file(&base).unwrap();
                std::os::unix::fs::symlink(&path, &base).unwrap();
            } else {
                let mut bytes = fs::read(&base).unwrap();
                bytes[0] ^= 0xff;
                fs::write(&base, bytes).unwrap();
            }
            assert!(service.list().unwrap()[0].damaged);
            assert!(service.restore(&editor.id).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }
    #[test]
    fn missing_read_snapshot_enters_save_as_only_and_remains_recoverable() {
        let (dir, path, docs, doc, mut e, initial) = fixture();
        e.release(&initial.id, &initial.generation, true).unwrap();
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let captured = e.begin(&docs, &doc.id).unwrap();
        assert!(captured.needs_save_as);
        assert!(captured.state.dirty);
        assert!(
            e.save(&captured.id, &captured.generation, 0, None, vec![])
                .is_err()
        );
        assert!(!path.exists());
        e.release(&captured.id, &captured.generation, false)
            .unwrap();
        assert!(
            e.list()
                .unwrap()
                .iter()
                .any(|r| r.id == captured.id && !r.clean)
        );
        let restored = e.restore(&captured.id).unwrap();
        let destination = ScopedFile::destination(&dir.path().join("recovered.txt")).unwrap();
        e.save(
            &restored.id,
            &restored.generation,
            0,
            Some(destination.canonical_path()),
            vec![],
        )
        .unwrap();
        assert_eq!(
            fs::read(dir.path().join("recovered.txt")).unwrap(),
            original
        );
        assert!(!path.exists());
    }
    #[test]
    fn library_mutation_reservation_excludes_clean_editors_and_new_source_sessions() {
        let (_dir, _path, docs, doc, mut editing, e) = fixture();
        assert!(!e.state.dirty);
        assert_eq!(
            editing.library_reserve().unwrap_err().code,
            "editing-active"
        );
        editing.release(&e.id, &e.generation, false).unwrap();
        editing.library_reserve().unwrap();
        assert_eq!(editing.begin(&docs, &doc.id).err().unwrap().code, "busy");
        editing.library_release();
        assert!(editing.begin(&docs, &doc.id).is_ok());
    }
    fn batch(e: &EditDescriptor, rev: u64, changes: Vec<Change>) -> Batch {
        Batch {
            schema: 1,
            generation: e.journal_generation.clone(),
            operation_id: format!("op{rev}"),
            base_revision: rev - 1,
            new_revision: rev,
            changes,
        }
    }
    fn text(from: usize, to: usize, s: &str) -> Change {
        Change {
            from,
            to,
            insert: Insert::Text { text: s.into() },
        }
    }
    #[test]
    fn actual_save_undo_across_saved_base_and_noop_reopen_are_byte_exact() {
        let (_dir, path, docs, doc, mut service, e) = fixture();
        let original = fs::read(&path).unwrap();
        let applied = service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "B")])],
                vec![],
            )
            .unwrap();
        assert!(applied.state.dirty);
        let saved = service.save(&e.id, &e.generation, 1, None, vec![]).unwrap();
        assert!(!saved.state.dirty);
        assert_eq!(fs::read(&path).unwrap(), "\u{feff}a\r\nB\nc\r🦀".as_bytes());
        assert_eq!(service.status(&e.id, &e.generation).unwrap(), "unchanged");
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 2, applied.acknowledgements[0].inverse.clone())],
                vec!["op1".into()],
            )
            .unwrap();
        service.save(&e.id, &e.generation, 2, None, vec![]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        let before = Identity::of(&fs::metadata(&path).unwrap());
        assert!(
            service
                .save(&e.id, &e.generation, 2, None, vec![])
                .unwrap()
                .noop
        );
        assert_eq!(Identity::of(&fs::metadata(&path).unwrap()), before);
        let reopened = docs.grant(&path, |_| {}).unwrap();
        assert_eq!(
            docs.read_range(&reopened.id, 0, 65536).unwrap().text,
            String::from_utf8(original).unwrap()
        );
        docs.release(&doc.id).unwrap();
    }
    #[test]
    fn post_commit_storage_failure_and_undo_remain_recoverable_even_at_original_text() {
        let (dir, path, _docs, _doc, mut service, e) = fixture();
        let ack = service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "B")])],
                vec![],
            )
            .unwrap();
        crate::private_store::fail_next("save-");
        assert_eq!(
            service
                .save(&e.id, &e.generation, 1, None, vec![])
                .err()
                .unwrap()
                .code,
            "durability"
        );
        assert_eq!(fs::read(&path).unwrap(), "\u{feff}a\r\nB\nc\r🦀".as_bytes());
        let undone = service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 2, ack.acknowledgements[0].inverse.clone())],
                vec!["op1".into()],
            )
            .unwrap();
        assert!(
            undone.state.dirty,
            "uncertain disk commit cannot clear dirty state after undo"
        );
        service.release(&e.id, &e.generation, false).unwrap();
        drop(service);
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        assert_eq!(service.list().unwrap().len(), 1);
        let restored = service.restore(&e.id).unwrap();
        assert!(restored.state.dirty);
        assert_eq!(restored.text, "a\nb\nc\n🦀");
    }
    #[test]
    fn failed_save_as_never_silently_changes_the_editing_target() {
        for fail_after_write in [false, true] {
            let (dir, path, _docs, _doc, mut service, e) = fixture();
            service
                .apply(
                    &e.id,
                    &e.generation,
                    vec![batch(&e, 1, vec![text(2, 3, "B")])],
                    vec![],
                )
                .unwrap();
            let copy = dir.path().join("copy.txt");
            if fail_after_write {
                crate::private_store::fail_next("save-");
            } else {
                fs::write(&copy, "\u{feff}a\r\nB\nc\r🦀").unwrap();
            }
            assert!(
                service
                    .save(&e.id, &e.generation, 1, Some(&copy), vec![])
                    .is_err()
            );
            assert_eq!(
                service
                    .get(&e.id, &e.generation)
                    .unwrap()
                    .target
                    .as_ref()
                    .unwrap()
                    .canonical_path(),
                fs::canonicalize(&path).unwrap()
            );
            service
                .apply(
                    &e.id,
                    &e.generation,
                    vec![batch(&e, 2, vec![text(4, 5, "C")])],
                    vec!["op1".into()],
                )
                .unwrap();
            service.save(&e.id, &e.generation, 2, None, vec![]).unwrap();
            assert_eq!(fs::read(&path).unwrap(), "\u{feff}a\r\nB\nC\r🦀".as_bytes());
            assert_eq!(fs::read(copy).unwrap(), "\u{feff}a\r\nB\nc\r🦀".as_bytes());
        }
    }
    #[test]
    fn saved_snapshots_are_immutable_and_clean_archives_can_be_discarded_explicitly() {
        let (_dir, _path, _docs, _doc, mut service, e) = fixture();
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "B")])],
                vec![],
            )
            .unwrap();
        service.save(&e.id, &e.generation, 1, None, vec![]).unwrap();
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 2, vec![text(4, 5, "C")])],
                vec!["op1".into()],
            )
            .unwrap();
        service.save(&e.id, &e.generation, 2, None, vec![]).unwrap();
        let files = fs::read_dir(service.folder(&e.id).unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("save-"))
            .map(|e| fs::read(e.path()).unwrap())
            .collect::<Vec<_>>();
        assert!(files.contains(&"\u{feff}a\r\nB\nc\r🦀".as_bytes().to_vec()));
        assert!(files.contains(&"\u{feff}a\r\nB\nC\r🦀".as_bytes().to_vec()));
        service.release(&e.id, &e.generation, false).unwrap();
        assert!(service.list().unwrap().is_empty());
        assert_eq!(service.list_with_saved(true).unwrap().len(), 1);
        service.discard(&e.id).unwrap();
        assert!(service.list_with_saved(true).unwrap().is_empty());
    }
    #[test]
    fn external_conflict_keeps_disk_base_pending_and_snapshot() {
        let (_dir, path, _docs, _doc, mut service, e) = fixture();
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "local")])],
                vec![],
            )
            .unwrap();
        fs::write(&path, "external\n").unwrap();
        assert_eq!(
            service
                .save(&e.id, &e.generation, 1, None, vec![])
                .err()
                .unwrap()
                .code,
            "conflict"
        );
        assert_eq!(fs::read(&path).unwrap(), b"external\n");
        assert_eq!(
            private_store::read(&service.folder(&e.id).unwrap().join("conflict.bin"), 1024)
                .unwrap(),
            b"external\n"
        );
        assert!(
            service
                .get(&e.id, &e.generation)
                .unwrap()
                .session
                .source()
                .logical_text()
                .contains("local")
        );
    }
    #[test]
    fn checkpoint_restore_requires_native_new_destination_and_keeps_mixed_eols() {
        let (dir, path, _docs, _doc, mut service, e) = fixture();
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "文")])],
                vec![],
            )
            .unwrap();
        service.flush(&e.id, &e.generation).unwrap();
        service
            .live
            .get_mut(&e.id)
            .unwrap()
            .session
            .checkpoint(&service.store.recovery(), &e.id)
            .unwrap();
        service.release(&e.id, &e.generation, false).unwrap();
        drop(service);
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        assert_eq!(service.list().unwrap().len(), 1);
        let restored = service.restore(&e.id).unwrap();
        assert!(restored.needs_save_as);
        assert_ne!(restored.generation, e.generation);
        assert!(service.flush(&e.id, &e.generation).is_err());
        assert_eq!(
            service
                .save(&e.id, &restored.generation, 1, None, vec![])
                .err()
                .unwrap()
                .code,
            "save-as"
        );
        let copy = dir.path().join("restored.txt");
        service
            .save(&e.id, &restored.generation, 1, Some(&copy), vec![])
            .unwrap();
        assert_eq!(fs::read(copy).unwrap(), "\u{feff}a\r\n文\nc\r🦀".as_bytes());
        assert_eq!(fs::read(path).unwrap(), "\u{feff}a\r\nb\nc\r🦀".as_bytes());
    }
    #[test]
    fn explicit_save_as_can_rescue_unacknowledged_edits_without_a_journal_ack() {
        let (dir, path, _docs, _doc, mut service, e) = fixture();
        let pending = batch(&e, 1, vec![text(2, 3, "rescue")]);
        let copy = dir.path().join("rescue.txt");
        let saved = service
            .save(&e.id, &e.generation, 1, Some(&copy), vec![pending])
            .unwrap();
        assert!(saved.rescue);
        assert_eq!(
            service
                .get(&e.id, &e.generation)
                .unwrap()
                .session
                .received_revision(),
            0
        );
        assert_eq!(
            fs::read(copy).unwrap(),
            "\u{feff}a\r\nrescue\nc\r🦀".as_bytes()
        );
        assert_eq!(fs::read(path).unwrap(), "\u{feff}a\r\nb\nc\r🦀".as_bytes());
    }
    #[test]
    fn settings_are_atomic_scoped_and_do_not_grant_execution() {
        let (_dir, _path, docs, doc, mut s, _e) = fixture();
        let good = s
            .update_settings(&docs, Some(&doc.id), "global", "$reader.width=800")
            .unwrap();
        assert!(good.diagnostics.is_empty());
        let bad = s
            .update_settings(
                &docs,
                Some(&doc.id),
                "global",
                "$reader.width=900\n$security.execution=true",
            )
            .unwrap();
        assert!(!bad.diagnostics.is_empty());
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "global").unwrap().text,
            "$reader.width=800"
        );
        s.update_settings(&docs, Some(&doc.id), "file", "$reader.width=1000")
            .unwrap();
        s.update_settings(&docs, Some(&doc.id), "session", "$reader.width=1100")
            .unwrap();
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "global")
                .unwrap()
                .resolved
                .get("reader.width"),
            Some(&crate::settings::Value::Number(1100.0))
        );
    }
    #[test]
    fn preference_preview_is_temporary_and_workspace_grants_control_scope() {
        let (dir, path, docs, doc, mut s, _e) = fixture();
        let before = s.settings(&docs, Some(&doc.id), "global").unwrap().resolved;
        let draft = s
            .preview_settings(&docs, Some(&doc.id), "global", "$reader.width=880")
            .unwrap();
        assert!(draft.diagnostics.is_empty());
        assert_eq!(
            draft.resolved.get("reader.width"),
            Some(&crate::settings::Value::Number(880.))
        );
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "global").unwrap().resolved,
            before
        );
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();
        s.workspace(&doc.id, Some(root));
        assert!(
            s.update_settings(&docs, Some(&doc.id), "workspace", "$reader.width=990")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "file")
                .unwrap()
                .resolved
                .get("reader.width"),
            Some(&crate::settings::Value::Number(990.))
        );
        s.workspace(&doc.id, None);
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "file").unwrap().resolved,
            before
        );
        assert!(
            !s.update_settings(&docs, Some(&doc.id), "file", "$shortcut.save=\"Mod+Alt+S\"")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        for scope in ["workspace", "file"] {
            assert!(
                !s.update_settings(&docs, Some(&doc.id), scope, "$library.width=440")
                    .unwrap()
                    .diagnostics
                    .is_empty()
            );
        }
        assert!(
            s.update_settings(&docs, None, "global", "$library.width=440")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        assert_eq!(
            s.settings(&docs, None, "global")
                .unwrap()
                .resolved
                .get("library.width"),
            Some(&crate::settings::Value::Number(440.))
        );
        assert_eq!(fs::read(path).unwrap(), "\u{feff}a\r\nb\nc\r🦀".as_bytes());
    }
    #[test]
    fn inherited_invalid_palette_preserves_literals_and_applies_safe_defaults() {
        let (_dir, _path, docs, doc, mut s, _e) = fixture();
        s.update_settings(&docs, Some(&doc.id), "file", "$color.ink=\"#ffffff\"")
            .unwrap();
        assert!(
            s.update_settings(&docs, None, "global", "$theme=custom")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        let result = s.settings(&docs, Some(&doc.id), "file").unwrap();
        assert!(!result.diagnostics.is_empty());
        assert_eq!(result.text, "$color.ink=\"#ffffff\"");
        assert_eq!(
            result.resolved.get("theme"),
            Some(&crate::settings::Value::Enum("system".into()))
        );
        assert!(
            s.update_settings(&docs, Some(&doc.id), "file", "")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        assert_eq!(
            s.settings(&docs, Some(&doc.id), "file")
                .unwrap()
                .resolved
                .get("theme"),
            Some(&crate::settings::Value::Enum("custom".into()))
        );
    }
    #[test]
    fn hundred_mib_ranges_preserve_bytes_recovery_and_stale_grants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        let mut original = vec![b'a'; 100 * 1024 * 1024];
        original[..3].copy_from_slice(&[0xef, 0xbb, 0xbf]);
        original[10..14].copy_from_slice(b"\r\nb\r");
        std::fs::write(&path, &original).unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        let first = service.begin(&docs, &doc.id).unwrap();
        assert_eq!(first.text.len(), 131072);
        assert!(first.range.is_some());
        let outside = batch(&first, 1, vec![text(200000, 200001, "bad")]);
        assert_eq!(
            service
                .apply(&first.id, &first.generation, vec![outside], vec![])
                .err()
                .unwrap()
                .code,
            "range"
        );
        service
            .apply(
                &first.id,
                &first.generation,
                vec![batch(&first, 1, vec![text(0, 1, "T")])],
                vec![],
            )
            .unwrap();
        let token = service
            .capture_read(&first.id, &first.generation, 1)
            .unwrap();
        let snap = service
            .read_snapshot(&first.id, &first.generation, &token, 0)
            .unwrap();
        assert!(snap.text.starts_with("\u{feff}T"));
        let second = service
            .change_window(&first.id, &first.generation, 1, 99 * 1024 * 1024)
            .unwrap();
        assert!(service.flush(&first.id, &first.generation).is_err());
        let at = second.range.as_ref().unwrap().from + 10;
        service
            .apply(
                &second.id,
                &second.generation,
                vec![batch(&second, 2, vec![text(at, at + 1, "文")])],
                vec![],
            )
            .unwrap();
        let back = service
            .change_window(&second.id, &second.generation, 2, 0)
            .unwrap();
        assert!(back.text.starts_with('T'));
        assert!(back.state.dirty);
        service
            .save(&back.id, &back.generation, 2, None, vec![])
            .unwrap();
        let mut expected = original.clone();
        expected[3] = b'T'; // Original logical CRLF contributes one fewer UTF-16 unit.
        expected.splice(at + 4..at + 5, "文".bytes());
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        service.release(&back.id, &back.generation, false).unwrap();
        drop(service);
        let mut service = Editing::open(&dir.path().join("private")).unwrap();
        let recovered = service.restore(&back.id).unwrap();
        assert!(recovered.needs_save_as);
        assert!(recovered.range.is_some());
        assert!(recovered.text.len() <= 131072);
        assert_eq!(
            service
                .save(&recovered.id, &recovered.generation, 2, None, vec![])
                .err()
                .unwrap()
                .code,
            "save-as"
        );
    }
    #[test]
    fn private_store_aliases_are_never_write_targets() {
        let (_dir, _path, docs, _doc, mut s, _e) = fixture();
        let owned = s.store.root.join("owned.txt");
        private_store::atomic(&owned, b"owned").unwrap();
        let doc = docs.grant(&owned, |_| {}).unwrap();
        let e = s.begin(&docs, &doc.id).unwrap();
        assert!(e.needs_save_as);
        let destination = s.store.root.join("not-a-user-file.txt");
        assert_eq!(
            s.save(&e.id, &e.generation, 0, Some(&destination), vec![])
                .err()
                .unwrap()
                .code,
            "private-storage"
        );
        assert!(!destination.exists());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_handles_preserve_non_utf8_filename_identity() {
        use std::os::unix::ffi::OsStringExt;
        let (dir, _path, docs, _doc, mut s, _e) = fixture();
        let path = dir
            .path()
            .join(std::ffi::OsString::from_vec(vec![b'f', 0xff]));
        fs::write(&path, b"base").unwrap();
        let other = dir.path().join("f�");
        fs::write(&other, b"not selected").unwrap();
        let doc = docs.grant(&path, |_| {}).unwrap();
        let e = s.begin(&docs, &doc.id).unwrap();
        s.apply(
            &e.id,
            &e.generation,
            vec![batch(&e, 1, vec![text(0, 4, "edited")])],
            vec![],
        )
        .unwrap();
        s.save(&e.id, &e.generation, 1, None, vec![]).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"edited");
        assert_eq!(fs::read(other).unwrap(), b"not selected");
    }
    #[test]
    fn guessed_handles_cannot_edit_save_compare_or_escape_recovery() {
        let (_dir, _path, _docs, _doc, mut s, e) = fixture();
        for id in ["../elsewhere", "/etc/passwd", "guessed"] {
            assert_eq!(
                s.flush(id, &e.generation).err().unwrap().code,
                "unauthorized"
            );
            assert!(s.discard(id).is_err());
            assert!(s.restore(id).is_err());
        }
        assert_eq!(
            s.save(&e.id, "stale", 0, None, vec![]).err().unwrap().code,
            "generation"
        );
    }
    #[test]
    fn crashed_process_replays_durable_input_after_checkpoint_and_rotation() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("private");
        let path = dir.path().join("source.txt");
        fs::write(&path, "\u{feff}a\r\nb\nc\r🦀").unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "editing::tests::crash_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("MARKTUB_CRASH_ROOT", &root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(id) = line.strip_prefix("READY=") {
                    let _ = tx.send(id.to_owned());
                    break;
                }
            }
        });
        let ready = rx.recv_timeout(std::time::Duration::from_secs(15));
        let _ = child.kill();
        let status = child.wait().unwrap();
        assert!(!status.success());
        let id = ready.expect("child must durably acknowledge before kill");
        let mut service = Editing::open(&root).unwrap();
        let e = service.restore(&id).unwrap();
        assert_eq!(e.state.received_revision, 2);
        assert_eq!(e.state.durable_revision, 2);
        let copy = dir.path().join("crash-copy.txt");
        service
            .save(&id, &e.generation, 2, Some(&copy), vec![])
            .unwrap();
        assert_eq!(fs::read(copy).unwrap(), "\u{feff}a\r\nB\nC\r🦀".as_bytes());
        assert_eq!(fs::read(path).unwrap(), "\u{feff}a\r\nb\nc\r🦀".as_bytes());
    }
    #[test]
    fn killed_large_range_process_restores_unsaved_distant_edits_without_a_path_grant() {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        let root = dir.path().join("private");
        let original = vec![b'a'; 100 * 1024 * 1024];
        fs::write(&path, &original).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "editing::tests::crash_worker",
                "--nocapture",
            ])
            .env("MARKTUB_CRASH_ROOT", &root)
            .env("MARKTUB_CRASH_RANGE", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(id) = line.strip_prefix("READY=") {
                    let _ = tx.send(id.to_owned());
                    break;
                }
            }
        });
        let ready = rx.recv_timeout(std::time::Duration::from_secs(90));
        let _ = child.kill();
        assert!(!child.wait().unwrap().success());
        let id = ready.expect("large child must durably ACK before SIGKILL");
        let mut service = Editing::open(&root).unwrap();
        let recovered = service.restore(&id).unwrap();
        assert!(recovered.needs_save_as && recovered.range.is_some());
        assert_eq!(recovered.state.durable_revision, 2);
        assert!(recovered.text.starts_with('T'));
        assert_eq!(
            service
                .save(&id, &recovered.generation, 2, None, vec![])
                .err()
                .unwrap()
                .code,
            "save-as"
        );
        let copy = dir.path().join("rescued.txt");
        service
            .save(&id, &recovered.generation, 2, Some(&copy), vec![])
            .unwrap();
        let mut expected = original.clone();
        expected[0] = b'T';
        expected[90 * 1024 * 1024 + 10] = b'Z';
        assert!(fs::read(copy).unwrap() == expected);
        assert!(fs::read(path).unwrap() == original);
    }
    #[test]
    #[ignore = "subprocess worker, executed by the crash acceptance test"]
    fn crash_worker() {
        let Some(root) = std::env::var_os("MARKTUB_CRASH_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let docs = Documents::default();
        let doc = docs
            .grant(&root.parent().unwrap().join("source.txt"), |_| {})
            .unwrap();
        let mut service = Editing::open(&root).unwrap();
        let e = service.begin(&docs, &doc.id).unwrap();
        if std::env::var_os("MARKTUB_CRASH_RANGE").is_some() {
            service
                .apply(
                    &e.id,
                    &e.generation,
                    vec![batch(&e, 1, vec![text(0, 1, "T")])],
                    vec![],
                )
                .unwrap();
            let next = service
                .change_window(&e.id, &e.generation, 1, 90 * 1024 * 1024)
                .unwrap();
            service
                .apply(
                    &next.id,
                    &next.generation,
                    vec![batch(
                        &next,
                        2,
                        vec![text(90 * 1024 * 1024 + 10, 90 * 1024 * 1024 + 11, "Z")],
                    )],
                    vec![],
                )
                .unwrap();
            service.flush(&next.id, &next.generation).unwrap();
            println!("READY={}", e.id);
            use std::io::Write;
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 1, vec![text(2, 3, "B")])],
                vec![],
            )
            .unwrap();
        service.flush(&e.id, &e.generation).unwrap();
        service
            .live
            .get_mut(&e.id)
            .unwrap()
            .session
            .checkpoint(&service.store.recovery(), &e.id)
            .unwrap();
        service
            .apply(
                &e.id,
                &e.generation,
                vec![batch(&e, 2, vec![text(4, 5, "C")])],
                vec!["op1".into()],
            )
            .unwrap();
        service.flush(&e.id, &e.generation).unwrap();
        println!("READY={}", e.id);
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
}
