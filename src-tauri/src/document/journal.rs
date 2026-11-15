//! Ordered, bounded recovery log. Append acknowledges receipt; flush acknowledges durability.
//! Checkpoints publish through an atomic log-header replacement; old replay remains valid until commit.
use super::provenance::ProvenanceData;
use super::{Change, EditError, Provenance, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
};
use uuid::Uuid;

const SCHEMA: u32 = 1;
const MAX_LOG: usize = 16 * 1024 * 1024;
const MAX_BASE: usize = 128 * 1024 * 1024;
const MAX_OPERATIONS: usize = 50_000;

#[derive(Debug)]
pub enum JournalError {
    Io(std::io::Error),
    Format,
    Schema,
    Base,
    Generation,
    Revision,
    Duplicate,
    Capacity,
    Poisoned,
    Edits(EditError),
}
impl From<std::io::Error> for JournalError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<EditError> for JournalError {
    fn from(e: EditError) -> Self {
        Self::Edits(e)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Batch {
    pub schema: u32,
    pub generation: String,
    pub operation_id: String,
    pub base_revision: u64,
    pub new_revision: u64,
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ack {
    pub generation: String,
    pub applied_revision: u64,
    pub received_revision: u64,
    pub durable_revision: u64,
    pub inverse: Vec<Change>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Header {
    schema: u32,
    generation: String,
    base_hash: String,
    #[serde(default)]
    checkpoint: Option<CheckpointRef>,
}
pub(crate) fn is_untouched_base_header(bytes: &[u8], id: &str, expected: &str) -> bool {
    bytes.len() <= 65536
        && bytes.ends_with(b"\n")
        && bytes.iter().filter(|&&b| b == b'\n').count() == 1
        && expected.len() == 64
        && expected.bytes().all(|b| b.is_ascii_hexdigit())
        && serde_json::from_slice::<Header>(bytes).is_ok_and(|header| {
            header.schema == SCHEMA
                && header.generation == id
                && header.checkpoint.is_none()
                && header.base_hash == expected
        })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRef {
    file: String,
    hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    hash: String,
    revision: u64,
    inverse: Vec<Change>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema: u32,
    generation: String,
    revision: u64,
    source: ProvenanceData,
    receipts: HashMap<String, Receipt>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Frame {
    batch: Batch,
    checksum: String,
}

/// Injectable persistence boundary; failures exercise the same append/flush implementation.
pub trait LogStore: Write + Seek {
    fn flush_durable(&mut self) -> std::io::Result<()>;
}
impl LogStore for File {
    fn flush_durable(&mut self) -> std::io::Result<()> {
        crate::private_store::sync(self)
    }
}

pub struct SaveInput {
    pub generation: String,
    pub revision: u64,
    pub source: Snapshot,
}

pub struct Session<S: LogStore = File> {
    source: Provenance,
    generation: String,
    revision: u64,
    durable: u64,
    operations: HashMap<String, Receipt>,
    base_hash: String,
    log: S,
    lease: Option<LogLease>,
    log_length: usize,
    checksum: String,
    poisoned: bool,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn checksum(previous: &str, batch: &Batch) -> String {
    let mut digest = Sha256::new();
    digest.update(previous.as_bytes());
    digest.update(serde_json::to_vec(batch).expect("serializable batch"));
    format!("{:x}", digest.finalize())
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, JournalError> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| JournalError::Format)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn private_root(root: &Path) -> Result<(), JournalError> {
    let metadata = fs::symlink_metadata(root)?;
    // SAFETY: geteuid has no preconditions or memory arguments.
    if !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(JournalError::Format);
    }
    Ok(())
}
fn create_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
struct LogLease(File);
impl Drop for LogLease {
    fn drop(&mut self) {
        // dup/fork may retain the open description after the logical owner ends.
        // Unlock explicitly; closing an unrelated duplicate must not unlock a
        // subsequent session that acquired its own open description.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}
fn lock_log(file: &File) -> std::io::Result<LogLease> {
    let lease = LogLease(file.try_clone()?);
    // SAFETY: file owns a valid FD for the entire call. Keep the advisory lock
    // until this Session drops, including poisoned/failed-flush sessions.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(lease)
}

impl Session<File> {
    /// Constant-space catalogue validation for the unique zero-history form.
    /// This is not a restored session; edits/checkpoints always use full replay.
    pub(crate) fn zero_history_summary(
        root: &Path,
        id: &str,
    ) -> Result<Option<String>, JournalError> {
        private_root(root)?;
        if Uuid::parse_str(id)
            .map_err(|_| JournalError::Format)?
            .to_string()
            != id
        {
            return Err(JournalError::Format);
        }
        let folder = root.join(id);
        private_root(&folder)?;
        let mut log = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(folder.join("journal.jsonl"))?;
        let _lease = lock_log(&log)?;
        let log_before = crate::atomic_save::Identity::of(&log.metadata()?);
        if !log.metadata()?.is_file() {
            return Err(JournalError::Format);
        }
        if log.metadata()?.len() > 65536 {
            return Ok(None);
        }
        let mut header_bytes = vec![];
        Read::by_ref(&mut log)
            .take(65537)
            .read_to_end(&mut header_bytes)?;
        let Ok(header) = serde_json::from_slice::<Header>(&header_bytes) else {
            return Ok(None);
        };
        if !is_untouched_base_header(&header_bytes, id, &header.base_hash) {
            return Ok(None);
        }
        let mut base = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(folder.join("base.bin"))?;
        let metadata = base.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_BASE as u64 {
            return Err(JournalError::Capacity);
        }
        let before = crate::atomic_save::Identity::of(&metadata);
        let mut input = Read::by_ref(&mut base).take(metadata.len() + 1);
        let mut buffer = [0_u8; 65540];
        let mut carry = 0;
        let mut length = 0_u64;
        let mut digest = Sha256::new();
        loop {
            let n = input.read(&mut buffer[carry..carry + 65536])?;
            if n == 0 {
                break;
            }
            length += n as u64;
            digest.update(&buffer[carry..carry + n]);
            let total = carry + n;
            if buffer[..total].contains(&0) {
                return Err(JournalError::Edits(EditError::Encoding));
            }
            match std::str::from_utf8(&buffer[..total]) {
                Ok(_) => carry = 0,
                Err(error) if error.error_len().is_none() => {
                    let valid = error.valid_up_to();
                    carry = total - valid;
                    buffer.copy_within(valid..total, 0);
                }
                Err(_) => return Err(JournalError::Edits(EditError::Encoding)),
            }
        }
        if carry != 0 {
            return Err(JournalError::Edits(EditError::Encoding));
        }
        let actual = format!("{:x}", digest.finalize());
        if length != metadata.len()
            || actual != header.base_hash
            || before != crate::atomic_save::Identity::of(&base.metadata()?)
            || log_before != crate::atomic_save::Identity::of(&log.metadata()?)
        {
            return Err(JournalError::Base);
        }
        Ok(Some(actual))
    }
    /// `root` is app-private native storage, never a frontend-supplied path.
    /// The caller must retain this ID until an explicit discard; drop never deletes recovery.
    pub fn create(root: &Path, base: &[u8]) -> Result<(String, Self), JournalError> {
        private_root(root)?;
        if base.len() > MAX_BASE {
            return Err(JournalError::Capacity);
        }
        // Build and validate once before creating recovery files. Keep this
        // exact provenance for the live session instead of allocating it twice.
        let source = Provenance::new(base)?;
        let id = Uuid::new_v4().to_string();
        let folder = root.join(&id);
        fs::DirBuilder::new().mode(0o700).create(&folder)?;
        let mut file = create_file(&folder.join("base.bin"))?;
        file.write_all(base)?;
        crate::private_store::sync(&file)?;
        let log = create_file(&folder.join("journal.jsonl"))?;
        let lease = lock_log(&log)?;
        let mut session = Self::start_source(base, &id, log, source)?;
        session.lease = Some(lease);
        File::open(&folder)?.sync_all()?;
        File::open(root)?.sync_all()?;
        Ok((id, session))
    }
    pub fn checkpoint_size(&self) -> Result<usize, JournalError> {
        Ok(encode(&Checkpoint {
            schema: SCHEMA,
            generation: self.generation.clone(),
            revision: self.revision,
            source: self.source.checkpoint(),
            receipts: self.operations.clone(),
        })?
        .len())
    }
    pub fn repair(&mut self, root: &Path, id: &str) -> Result<(), JournalError> {
        if id != self.generation {
            return Err(JournalError::Generation);
        }
        let folder = root.join(id);
        let base = crate::private_store::read(&folder.join("base.bin"), MAX_BASE as u64)
            .map_err(|_| JournalError::Base)?;
        let mut log = self.log.try_clone()?;
        log.rewind()?;
        let mut bytes = vec![];
        Read::by_ref(&mut log)
            .take(MAX_LOG as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_LOG {
            return Err(JournalError::Capacity);
        }
        let (mut repaired, complete) = Self::replay(&base, id, log, &bytes, Some(&folder))?;
        repaired.log.set_len(complete as u64)?;
        crate::private_store::sync(&repaired.log)?;
        repaired.lease = self.lease.take();
        *self = repaired;
        Ok(())
    }
    pub fn checkpoint(&mut self, root: &Path, id: &str) -> Result<(), JournalError> {
        private_root(root)?;
        if id != self.generation || Uuid::parse_str(id).is_err() {
            return Err(JournalError::Generation);
        }
        self.flush()?;
        let folder = root.join(id);
        let bytes = encode(&Checkpoint {
            schema: SCHEMA,
            generation: self.generation.clone(),
            revision: self.revision,
            source: self.source.checkpoint(),
            receipts: self.operations.clone(),
        })?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err(JournalError::Capacity);
        }
        let name = format!("{}.checkpoint", Uuid::new_v4());
        let mut file = create_file(&folder.join(&name))?;
        file.write_all(&bytes)?;
        crate::private_store::sync(&file)?;
        File::open(&folder)?.sync_all()?;
        let checksum = hash(&bytes);
        let header = encode(&Header {
            schema: SCHEMA,
            generation: self.generation.clone(),
            base_hash: self.base_hash.clone(),
            checkpoint: Some(CheckpointRef {
                file: name.clone(),
                hash: checksum.clone(),
            }),
        })?;
        let temp = folder.join(format!("{}.log", Uuid::new_v4()));
        let mut new_log = create_file(&temp)?;
        let lease = lock_log(&new_log)?;
        new_log.write_all(&header)?;
        crate::private_store::sync(&new_log)?;
        fs::rename(&temp, folder.join("journal.jsonl"))?;
        // Keep both old and new log FDs locked until the new header is published.
        self.log = new_log;
        self.lease = Some(lease);
        self.log_length = header.len();
        self.checksum = checksum;
        File::open(&folder)?.sync_all()?;
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "checkpoint")
                && entry.file_name() != std::ffi::OsStr::new(&name)
            {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    pub fn recover(root: &Path, id: &str) -> Result<Self, JournalError> {
        private_root(root)?;
        let parsed = Uuid::parse_str(id).map_err(|_| JournalError::Format)?;
        if parsed.to_string() != id {
            return Err(JournalError::Format);
        }
        let folder = root.join(id);
        private_root(&folder)?;
        let open = |name: &str, write: bool| {
            OpenOptions::new()
                .read(true)
                .write(write)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(folder.join(name))
        };
        let mut base_file = open("base.bin", false)?;
        if !base_file.metadata()?.is_file() || base_file.metadata()?.len() > MAX_BASE as u64 {
            return Err(JournalError::Capacity);
        }
        let mut base = Vec::new();
        Read::by_ref(&mut base_file)
            .take(MAX_BASE as u64 + 1)
            .read_to_end(&mut base)?;
        if base.len() > MAX_BASE {
            return Err(JournalError::Capacity);
        }
        let mut log = open("journal.jsonl", true)?;
        let lease = lock_log(&log)?;
        if !log.metadata()?.is_file() || log.metadata()?.len() > MAX_LOG as u64 {
            return Err(JournalError::Capacity);
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut log)
            .take(MAX_LOG as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_LOG {
            return Err(JournalError::Capacity);
        }
        let (mut session, complete) = Self::replay(&base, id, log, &bytes, Some(&folder))?;
        session.lease = Some(lease);
        // Only a torn final frame may be discarded. Complete corrupt frames fail closed.
        // A no-op set_len still changes inode timestamps on macOS, invalidating
        // read-only catalogue fingerprints. Preserve metadata for a complete log.
        if complete < bytes.len() {
            session.log.set_len(complete as u64)?;
        }
        crate::private_store::sync(&session.log)?;
        Ok(session)
    }
}
impl<S: LogStore> Session<S> {
    #[cfg(test)]
    fn start(base: &[u8], generation: &str, log: S) -> Result<Self, JournalError> {
        if base.len() > MAX_BASE {
            return Err(JournalError::Capacity);
        }
        let source = Provenance::new(base)?;
        Self::start_source(base, generation, log, source)
    }
    fn start_source(
        base: &[u8],
        generation: &str,
        mut log: S,
        source: Provenance,
    ) -> Result<Self, JournalError> {
        if Uuid::parse_str(generation).is_err() {
            return Err(JournalError::Generation);
        }
        let base_hash = hash(base);
        let bytes = encode(&Header {
            schema: SCHEMA,
            generation: generation.into(),
            base_hash: base_hash.clone(),
            checkpoint: None,
        })?;
        // A new log must be empty. Do not overwrite an existing session accidentally.
        if log.seek(SeekFrom::End(0))? != 0 {
            return Err(JournalError::Format);
        }
        log.write_all(&bytes)?;
        log.flush_durable()?;
        Ok(Self {
            source,
            generation: generation.into(),
            revision: 0,
            durable: 0,
            operations: HashMap::new(),
            log,
            lease: None,
            log_length: bytes.len(),
            checksum: base_hash.clone(),
            base_hash,
            poisoned: false,
        })
    }
    fn replay(
        base: &[u8],
        generation: &str,
        log: S,
        bytes: &[u8],
        checkpoint_folder: Option<&Path>,
    ) -> Result<(Self, usize), JournalError> {
        let complete = bytes
            .iter()
            .rposition(|b| *b == b'\n')
            .ok_or(JournalError::Format)?
            + 1;
        let mut lines = bytes[..complete - 1].split(|b| *b == b'\n');
        let header: Header = serde_json::from_slice(lines.next().ok_or(JournalError::Format)?)
            .map_err(|_| JournalError::Format)?;
        if header.schema != SCHEMA {
            return Err(JournalError::Schema);
        }
        if header.generation != generation {
            return Err(JournalError::Generation);
        }
        if header.base_hash != hash(base) {
            return Err(JournalError::Base);
        }
        let mut session = Self {
            source: Provenance::new(base)?,
            generation: generation.into(),
            revision: 0,
            durable: 0,
            operations: HashMap::new(),
            log,
            lease: None,
            log_length: complete,
            checksum: header.base_hash.clone(),
            base_hash: header.base_hash,
            poisoned: false,
        };
        if let Some(reference) = header.checkpoint {
            if !reference.file.ends_with(".checkpoint")
                || Uuid::parse_str(reference.file.trim_end_matches(".checkpoint")).is_err()
            {
                return Err(JournalError::Format);
            }
            let folder = checkpoint_folder.ok_or(JournalError::Format)?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(folder.join(&reference.file))?;
            if !file.metadata()?.is_file() || file.metadata()?.len() > 128 * 1024 * 1024 {
                return Err(JournalError::Capacity);
            }
            let mut data = vec![];
            Read::by_ref(&mut file)
                .take(128 * 1024 * 1024 + 1)
                .read_to_end(&mut data)?;
            if data.len() > 128 * 1024 * 1024 || hash(&data) != reference.hash {
                return Err(JournalError::Format);
            }
            let state: Checkpoint =
                serde_json::from_slice(&data).map_err(|_| JournalError::Format)?;
            if state.schema != SCHEMA
                || state.generation != generation
                || state.receipts.len() > MAX_OPERATIONS
            {
                return Err(JournalError::Format);
            }
            session.source = Provenance::from_checkpoint(state.source)?;
            session.revision = state.revision;
            session.operations = state.receipts;
            session.checksum = reference.hash;
        }
        for line in lines {
            let frame: Frame = serde_json::from_slice(line).map_err(|_| JournalError::Format)?;
            session.validate(&frame.batch)?;
            if session.operations.contains_key(&frame.batch.operation_id)
                || checksum(&session.checksum, &frame.batch) != frame.checksum
            {
                return Err(JournalError::Format);
            }
            let prepared = session
                .source
                .prepare(&frame.batch.operation_id, &frame.batch.changes)?;
            let inverse = prepared.inverse.clone();
            session.source.commit(prepared);
            session.revision = frame.batch.new_revision;
            session.checksum = frame.checksum;
            session.operations.insert(
                frame.batch.operation_id.clone(),
                Receipt {
                    hash: hash(
                        &serde_json::to_vec(&frame.batch).map_err(|_| JournalError::Format)?,
                    ),
                    revision: frame.batch.new_revision,
                    inverse,
                },
            );
        }
        session.log.seek(SeekFrom::Start(complete as u64))?;
        // The recovered prefix is synced by recover() before this session is exposed.
        session.durable = session.revision;
        Ok((session, complete))
    }
    fn validate(&self, batch: &Batch) -> Result<(), JournalError> {
        if batch.schema != SCHEMA {
            return Err(JournalError::Schema);
        }
        if batch.generation != self.generation {
            return Err(JournalError::Generation);
        }
        if batch.base_revision != self.revision
            || self.revision.checked_add(1) != Some(batch.new_revision)
        {
            return Err(JournalError::Revision);
        }
        if batch.operation_id.is_empty()
            || batch.operation_id.len() > 64
            || !batch
                .operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(JournalError::Format);
        }
        if self.operations.len() >= MAX_OPERATIONS {
            return Err(JournalError::Capacity);
        }
        Ok(())
    }
    fn ack(&self, revision: u64, inverse: Vec<Change>) -> Ack {
        Ack {
            generation: self.generation.clone(),
            applied_revision: revision,
            received_revision: self.revision,
            durable_revision: self.durable,
            inverse,
        }
    }
    pub fn append(&mut self, batch: Batch) -> Result<Ack, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        if batch.generation != self.generation {
            return Err(JournalError::Generation);
        }
        if let Some(previous) = self.operations.get(&batch.operation_id) {
            if previous.hash != hash(&serde_json::to_vec(&batch).map_err(|_| JournalError::Format)?)
            {
                return Err(JournalError::Duplicate);
            }
            return Ok(self.ack(previous.revision, previous.inverse.clone()));
        }
        self.validate(&batch)?;
        let prepared = self.source.prepare(&batch.operation_id, &batch.changes)?;
        let inverse = prepared.inverse.clone();
        let checksum = checksum(&self.checksum, &batch);
        let frame = encode(&Frame {
            batch: batch.clone(),
            checksum: checksum.clone(),
        })?;
        if self.log_length + frame.len() > MAX_LOG {
            return Err(JournalError::Capacity);
        }
        if let Err(error) = self.log.write_all(&frame) {
            // A partial append has an uncertain tail. Preserve earlier ACKs and forbid
            // further writes until recovery validates/truncates the log.
            self.poisoned = true;
            return Err(error.into());
        }
        self.log_length += frame.len();
        self.source.commit(prepared);
        self.revision = batch.new_revision;
        self.checksum = checksum;
        self.operations.insert(
            batch.operation_id.clone(),
            Receipt {
                hash: hash(&serde_json::to_vec(&batch).map_err(|_| JournalError::Format)?),
                revision: batch.new_revision,
                inverse: inverse.clone(),
            },
        );
        Ok(self.ack(self.revision, inverse))
    }
    pub fn flush(&mut self) -> Result<Ack, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        self.log.flush_durable()?; // A failed flush never advances the durable watermark.
        self.durable = self.revision;
        Ok(self.ack(self.revision, vec![]))
    }
    pub fn capture_save(&mut self, revision: u64) -> Result<SaveInput, JournalError> {
        if revision != self.revision {
            return Err(JournalError::Revision);
        }
        self.flush()?;
        Ok(SaveInput {
            generation: self.generation.clone(),
            revision,
            source: self.source.snapshot(),
        })
    }
    pub fn retain_operations(&mut self, keep: &std::collections::HashSet<String>) {
        let tokens = self
            .operations
            .iter()
            .filter(|(id, _)| keep.contains(*id))
            .flat_map(|(_, r)| r.inverse.iter())
            .filter_map(|c| match &c.insert {
                super::Insert::Restore { token } => Some(token.clone()),
                _ => None,
            })
            .collect();
        self.source.retain_tokens(&tokens);
        for (id, receipt) in &mut self.operations {
            if !keep.contains(id) {
                receipt.inverse.clear();
            }
        }
    }
    pub fn preview(&self, batches: &[Batch]) -> Result<Snapshot, JournalError> {
        let mut source = Provenance::from_checkpoint(self.source.checkpoint())?;
        // Preview is an independent-copy rescue, not journal mutation. Release
        // history that pending restore operations cannot reference before applying.
        let needed = batches
            .iter()
            .flat_map(|b| &b.changes)
            .filter_map(|c| match &c.insert {
                super::Insert::Restore { token } => Some(token.clone()),
                _ => None,
            })
            .collect();
        source.retain_tokens(&needed);
        let mut revision = self.revision;
        for batch in batches {
            if batch.generation != self.generation || batch.schema != SCHEMA {
                return Err(JournalError::Generation);
            }
            if let Some(receipt) = self.operations.get(&batch.operation_id) {
                if receipt.hash
                    != hash(&serde_json::to_vec(batch).map_err(|_| JournalError::Format)?)
                {
                    return Err(JournalError::Duplicate);
                }
                continue;
            }
            if batch.base_revision != revision
                || revision.checked_add(1) != Some(batch.new_revision)
            {
                return Err(JournalError::Revision);
            }
            source.apply(&batch.operation_id, &batch.changes)?;
            revision = batch.new_revision;
        }
        Ok(source.snapshot())
    }
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn log_bytes(&self) -> usize {
        self.log_length
    }
    pub fn source(&self) -> &Provenance {
        &self.source
    }
    pub fn received_revision(&self) -> u64 {
        self.revision
    }
    pub fn durable_revision(&self) -> u64 {
        self.durable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Insert;
    use std::io::Cursor;
    #[derive(Default)]
    struct FaultLog {
        bytes: Cursor<Vec<u8>>,
        fail_after: Option<u64>,
        fail_sync: bool,
    }
    impl Write for FaultLog {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = self
                .fail_after
                .map(|end| end.saturating_sub(self.bytes.position()) as usize)
                .unwrap_or(bytes.len())
                .min(bytes.len());
            if count == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
            }
            self.bytes.write(&bytes[..count])
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for FaultLog {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.bytes.seek(p)
        }
    }
    impl LogStore for FaultLog {
        fn flush_durable(&mut self) -> std::io::Result<()> {
            if self.fail_sync {
                Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
            } else {
                Ok(())
            }
        }
    }
    fn batch(generation: &str, revision: u64, from: usize, to: usize, text: &str) -> Batch {
        Batch {
            schema: SCHEMA,
            generation: generation.into(),
            operation_id: format!("op{revision}"),
            base_revision: revision - 1,
            new_revision: revision,
            changes: vec![Change {
                from,
                to,
                insert: Insert::Text { text: text.into() },
            }],
        }
    }
    fn output(source: &Provenance) -> Vec<u8> {
        let mut out = vec![];
        source.write_to(&mut out).unwrap();
        out
    }
    #[test]
    fn validates_order_generation_duplicate_payload_and_durable_watermarks() {
        let generation = Uuid::new_v4().to_string();
        let mut s = Session::start("🦀\r\nb".as_bytes(), &generation, FaultLog::default()).unwrap();
        let first = batch(&generation, 1, 3, 4, "c");
        let ack = s.append(first.clone()).unwrap();
        assert_eq!((ack.received_revision, ack.durable_revision), (1, 0));
        assert_eq!(s.append(first.clone()).unwrap(), ack);
        let mut changed = first;
        changed.changes[0].insert = Insert::Text {
            text: "different".into(),
        };
        assert!(matches!(s.append(changed), Err(JournalError::Duplicate)));
        assert!(matches!(
            s.append(batch(&generation, 3, 0, 0, "x")),
            Err(JournalError::Revision)
        ));
        assert!(matches!(
            s.append(batch(&Uuid::new_v4().to_string(), 2, 0, 0, "x")),
            Err(JournalError::Generation)
        ));
        assert_eq!(s.flush().unwrap().durable_revision, 1);
        assert_eq!(output(s.source()), "🦀\r\nc".as_bytes());
    }
    #[test]
    fn save_n_captures_immutable_pieces_while_n_plus_one_remains_received_only() {
        let generation = Uuid::new_v4().to_string();
        let mut s = Session::start(b"a\r\nb", &generation, FaultLog::default()).unwrap();
        s.append(batch(&generation, 1, 2, 3, "c")).unwrap();
        let saved = s.capture_save(1).unwrap();
        s.append(batch(&generation, 2, 2, 3, "d")).unwrap();
        let mut out = vec![];
        saved.source.write_to(&mut out).unwrap();
        assert_eq!(out, b"a\r\nc");
        assert_eq!(output(s.source()), b"a\r\nd");
        assert_eq!((s.received_revision(), s.durable_revision()), (2, 1));
        assert!(matches!(s.capture_save(1), Err(JournalError::Revision)));
    }
    #[test]
    fn disk_full_during_append_keeps_prior_ack_and_poisons_uncertain_tail() {
        let generation = Uuid::new_v4().to_string();
        let mut s = Session::start(b"abc", &generation, FaultLog::default()).unwrap();
        s.append(batch(&generation, 1, 0, 1, "A")).unwrap();
        s.flush().unwrap();
        s.log.fail_after = Some(s.log.bytes.position() + 20);
        assert!(matches!(
            s.append(batch(&generation, 2, 1, 2, "B")),
            Err(JournalError::Io(_))
        ));
        assert_eq!(output(s.source()), b"Abc");
        assert_eq!((s.received_revision(), s.durable_revision()), (1, 1));
        assert!(matches!(s.flush(), Err(JournalError::Poisoned)));
    }
    #[test]
    fn disk_full_during_flush_never_discards_received_edits_or_claims_durability() {
        let generation = Uuid::new_v4().to_string();
        let mut s = Session::start(b"abc", &generation, FaultLog::default()).unwrap();
        s.append(batch(&generation, 1, 0, 1, "A")).unwrap();
        s.log.fail_sync = true;
        assert!(matches!(s.flush(), Err(JournalError::Io(_))));
        assert!(matches!(s.capture_save(1), Err(JournalError::Io(_))));
        assert_eq!(s.durable_revision(), 0);
        assert_eq!(output(s.source()), b"Abc");
        s.log.fail_sync = false;
        assert_eq!(s.flush().unwrap().durable_revision, 1);
    }
    // A parallel process-spawn test can briefly inherit CLOEXEC descriptors
    // before exec closes them. Only retry after this test has dropped its owner;
    // the separate live-lock refusal test still requires immediate denial.
    fn recover_after_drop(root: &Path, id: &str) -> Session<File> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        loop {
            match Session::recover(root, id) {
                Ok(session) => return session,
                Err(JournalError::Io(error))
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(error) => panic!("recovery after drop failed: {error:?}"),
            }
        }
    }
    #[test]
    fn complete_recovery_syncs_without_touching_journal_identity() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, mut session) = Session::create(root.path(), b"a\r\nb\n").unwrap();
        session.append(batch(&id, 1, 0, 1, "A")).unwrap();
        session.flush().unwrap();
        drop(session);
        let log = root.path().join(&id).join("journal.jsonl");
        let identity = crate::atomic_save::Identity::of(&fs::metadata(&log).unwrap());
        let recovered = recover_after_drop(root.path(), &id);
        assert_eq!(output(recovered.source()), b"A\r\nb\n");
        assert_eq!(recovered.durable_revision(), 1);
        assert_eq!(
            crate::atomic_save::Identity::of(&fs::metadata(log).unwrap()),
            identity
        );
    }
    #[test]
    fn recovers_private_base_inverse_provenance_and_torn_tail_then_accepts_new_edits() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, mut s) = Session::create(root.path(), b"a\r\nb\nc\r").unwrap();
        let undo = s.append(batch(&id, 1, 1, 5, "X")).unwrap().inverse;
        s.flush().unwrap();
        drop(s);
        let log_path = root.path().join(&id).join("journal.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap()
            .write_all(b"{\"batch\":")
            .unwrap();
        let mut s = recover_after_drop(root.path(), &id);
        let mut inverse = batch(&id, 2, 0, 0, "");
        inverse.changes = undo;
        s.append(inverse).unwrap();
        s.flush().unwrap();
        drop(s);
        let s = recover_after_drop(root.path(), &id);
        assert_eq!(output(s.source()), b"a\r\nb\nc\r");
        assert_eq!(s.durable_revision(), 2);
        assert_eq!(
            fs::metadata(root.path().join(&id).join("base.bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn refuses_tampered_base_complete_frame_and_path_shaped_recovery_ids() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, mut s) = Session::create(root.path(), b"abc").unwrap();
        s.append(batch(&id, 1, 0, 1, "A")).unwrap();
        s.flush().unwrap();
        drop(s);
        assert!(matches!(
            Session::recover(root.path(), "../elsewhere"),
            Err(JournalError::Format)
        ));
        let base = root.path().join(&id).join("base.bin");
        fs::write(&base, b"tampered").unwrap();
        assert!(matches!(
            Session::recover(root.path(), &id),
            Err(JournalError::Base)
        ));
        fs::write(&base, b"abc").unwrap();
        let log_path = root.path().join(&id).join("journal.jsonl");
        let bytes = fs::read(&log_path).unwrap();
        let bad = String::from_utf8(bytes).unwrap().replace("\"A\"", "\"Z\"");
        fs::write(&log_path, bad).unwrap();
        assert!(matches!(
            Session::recover(root.path(), &id),
            Err(JournalError::Format)
        ));
    }
    #[test]
    fn recovery_cannot_open_a_live_session_in_another_app_instance() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, mut live) = Session::create(root.path(), b"abc").unwrap();
        live.append(batch(&id, 1, 0, 1, "A")).unwrap();
        assert!(matches!(Session::recover(root.path(), &id),
            Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock));
        live.flush().unwrap();
        drop(live);
        let recovered = Session::recover(root.path(), &id).unwrap();
        assert_eq!(output(recovered.source()), b"Abc");
    }
    #[test]
    fn duplicate_log_description_does_not_extend_the_closed_sessions_lease() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, live) = Session::create(root.path(), b"abc").unwrap();
        let duplicate = live.log.try_clone().unwrap();
        drop(live);
        let recovered = Session::recover(root.path(), &id)
            .expect("Only the logical session owns the journal lease");
        drop(duplicate);
        assert!(
            matches!(Session::recover(root.path(), &id), Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        drop(recovered);
        assert!(Session::recover(root.path(), &id).is_ok());
    }
    #[test]
    fn journal_lease_survives_repair_failure_success_and_checkpoint_publication() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (id, mut live) = Session::create(root.path(), b"abc").unwrap();
        live.append(batch(&id, 1, 0, 1, "A")).unwrap();
        live.flush().unwrap();
        let path = root.path().join(&id).join("journal.jsonl");
        let valid = fs::read(&path).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"invalid complete frame\n")
            .unwrap();
        assert!(live.repair(root.path(), &id).is_err());
        assert!(
            matches!(Session::recover(root.path(), &id), Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        fs::write(&path, valid).unwrap();
        live.repair(root.path(), &id).unwrap();
        assert!(
            matches!(Session::recover(root.path(), &id), Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        let old_description = live.log.try_clone().unwrap();
        live.checkpoint(root.path(), &id).unwrap();
        drop(old_description);
        assert!(
            matches!(Session::recover(root.path(), &id), Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        let current_description = live.log.try_clone().unwrap();
        drop(live);
        let recovered = Session::recover(root.path(), &id).unwrap();
        drop(current_description);
        assert!(
            matches!(Session::recover(root.path(), &id), Err(JournalError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        assert_eq!(output(recovered.source()), b"Abc");
    }
}

#[cfg(test)]
mod zero_history_catalogue_tests {
    use super::*;
    fn private() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root
    }
    #[test]
    fn streaming_zero_history_matches_full_replay_at_utf8_boundaries_without_changing_metadata() {
        let mut boundary = vec![b'a'; 65535];
        boundary.extend_from_slice("🦀文\r\n".as_bytes());
        boundary.extend_from_slice("文🦀\rnext\n".repeat(10000).as_bytes());
        for bytes in [
            vec![],
            "\u{feff}\u{feff}a\r\n文🦀\rnext\n".as_bytes().to_vec(),
            boundary,
        ] {
            let root = private();
            let (id, session) = Session::create(root.path(), &bytes).unwrap();
            drop(session);
            let folder = root.path().join(&id);
            let before = ["base.bin", "journal.jsonl"].map(|name| {
                crate::atomic_save::Identity::of(&fs::metadata(folder.join(name)).unwrap())
            });
            let streamed = Session::zero_history_summary(root.path(), &id)
                .unwrap()
                .unwrap();
            let restored = Session::recover(root.path(), &id).unwrap();
            assert_eq!(restored.received_revision(), 0);
            assert_eq!(
                streamed,
                crate::atomic_save::source_hash(&restored.source().snapshot()).unwrap()
            );
            assert_eq!(streamed, hash(&bytes));
            drop(restored);
            let after = ["base.bin", "journal.jsonl"].map(|name| {
                crate::atomic_save::Identity::of(&fs::metadata(folder.join(name)).unwrap())
            });
            assert_eq!(before, after);
        }
    }
    #[test]
    fn streaming_refuses_corrupt_utf8_nul_checksums_symlinks_and_oversized_bases() {
        for invalid in [vec![0], vec![255], vec![240, 159, 166]] {
            let root = private();
            let (id, session) = Session::create(root.path(), b"original").unwrap();
            drop(session);
            let folder = root.path().join(&id);
            let log = folder.join("journal.jsonl");
            let mut header: Header = serde_json::from_slice(&fs::read(&log).unwrap()).unwrap();
            header.base_hash = hash(&invalid);
            fs::write(&log, encode(&header).unwrap()).unwrap();
            fs::write(folder.join("base.bin"), &invalid).unwrap();
            assert!(matches!(
                Session::zero_history_summary(root.path(), &id),
                Err(JournalError::Edits(EditError::Encoding))
            ));
            assert!(matches!(
                Session::recover(root.path(), &id),
                Err(JournalError::Edits(EditError::Encoding))
            ));
            assert_eq!(fs::read(folder.join("base.bin")).unwrap(), invalid);
        }
        let root = private();
        let (id, session) = Session::create(root.path(), b"original").unwrap();
        drop(session);
        let base = root.path().join(&id).join("base.bin");
        fs::write(&base, b"tampered").unwrap();
        assert!(matches!(
            Session::zero_history_summary(root.path(), &id),
            Err(JournalError::Base)
        ));
        assert!(matches!(
            Session::recover(root.path(), &id),
            Err(JournalError::Base)
        ));
        fs::OpenOptions::new()
            .write(true)
            .open(&base)
            .unwrap()
            .set_len(MAX_BASE as u64 + 1)
            .unwrap();
        assert!(matches!(
            Session::zero_history_summary(root.path(), &id),
            Err(JournalError::Capacity)
        ));
        fs::remove_file(&base).unwrap();
        let secret = root.path().join("secret");
        fs::write(&secret, b"keep secret").unwrap();
        std::os::unix::fs::symlink(&secret, &base).unwrap();
        assert!(Session::zero_history_summary(root.path(), &id).is_err());
        assert_eq!(fs::read(&secret).unwrap(), b"keep secret");
    }
    #[test]
    fn streaming_defers_receipts_checkpoints_and_torn_frames_to_full_replay() {
        for case in ["receipt", "checkpoint", "torn"] {
            let root = private();
            let (id, mut session) = Session::create(root.path(), b"original\r\n").unwrap();
            if case == "receipt" {
                session
                    .append(Batch {
                        schema: SCHEMA,
                        generation: id.clone(),
                        operation_id: "edit".into(),
                        base_revision: 0,
                        new_revision: 1,
                        changes: vec![Change {
                            from: 0,
                            to: 1,
                            insert: super::super::Insert::Text { text: "O".into() },
                        }],
                    })
                    .unwrap();
                session.flush().unwrap();
            } else if case == "checkpoint" {
                session.checkpoint(root.path(), &id).unwrap();
            }
            drop(session);
            let log = root.path().join(&id).join("journal.jsonl");
            if case == "torn" {
                OpenOptions::new()
                    .append(true)
                    .open(&log)
                    .unwrap()
                    .write_all(b"{\"partial\":")
                    .unwrap();
            }
            let before = fs::read(&log).unwrap();
            assert!(
                Session::zero_history_summary(root.path(), &id)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(fs::read(&log).unwrap(), before);
            let restored = Session::recover(root.path(), &id).unwrap();
            assert_eq!(restored.received_revision(), u64::from(case == "receipt"));
        }
    }
}
