//! Explicitly selected, local-only Git. Renderer input never becomes a patch command.
use crate::{
    atomic_save,
    document::Provenance,
    files::FileError,
    private_store,
    scoped_file::{ScopedFile, path_key},
};
use git2::{ObjectType, Oid, Repository};
use serde::Serialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::{CString, OsString},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
const FILE_LIMIT: usize = 4 * 1024 * 1024;
const INDEX_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
fn error(code: &'static str, text: &str) -> FileError {
    FileError::new(code, text)
}
fn stale() -> FileError {
    error(
        "git-stale",
        "Repository, index or disk bytes changed. Refresh the comparison before acting.",
    )
}
fn unsupported(text: &str) -> FileError {
    error("git-unsupported", text)
}
fn ge(e: git2::Error) -> FileError {
    error("git", &format!("Local Git: {}", e.message()))
}
fn uid() -> String {
    Uuid::new_v4().to_string()
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn display_path(path: &[u8]) -> String {
    String::from_utf8_lossy(path)
        .chars()
        .flat_map(char::escape_debug)
        .collect()
}
fn oid(b: &[u8]) -> Result<Oid, FileError> {
    Oid::hash_object(ObjectType::Blob, b).map_err(ge)
}
fn stamp(m: &fs::Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
fn dir(path: &Path) -> Result<File, FileError> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}
fn subdir(parent: &File, name: &std::ffi::OsStr) -> Result<File, FileError> {
    let name = CString::new(name.as_bytes()).map_err(|_| stale())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn components(path: &[u8]) -> Result<Vec<OsString>, FileError> {
    if path.is_empty() || path.len() > 16384 || path.contains(&0) || path[0] == b'/' {
        return Err(stale());
    }
    path.split(|b| *b == b'/')
        .map(|p| {
            if p.is_empty() || p == b"." || p == b".." || p.eq_ignore_ascii_case(b".git") {
                Err(unsupported(
                    "Unsafe repository path; no traversal or Git-metadata file action is allowed.",
                ))
            } else {
                Ok(OsString::from_vec(p.to_vec()))
            }
        })
        .collect()
}
#[derive(Clone)]
struct Entry {
    path: Vec<u8>,
    mode: u32,
    oid: Oid,
    raw: Vec<u8>,
    stage: u16,
}
#[derive(Clone)]
struct Index {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
    mutable: bool,
    reason: String,
}
fn be(b: &[u8], p: usize) -> Result<u32, FileError> {
    Ok(u32::from_be_bytes(
        b.get(p..p + 4)
            .ok_or_else(stale)?
            .try_into()
            .map_err(|_| stale())?,
    ))
}
impl Index {
    fn parse(bytes: Vec<u8>) -> Result<Self, FileError> {
        if bytes.is_empty() {
            return Ok(Self {
                bytes,
                entries: vec![],
                mutable: true,
                reason: String::new(),
            });
        }
        if bytes.len() < 32
            || &bytes[..4] != b"DIRC"
            || Sha1::digest(&bytes[..bytes.len() - 20]).as_slice() != &bytes[bytes.len() - 20..]
        {
            return Err(unsupported(
                "Invalid or non-SHA1 index; no mutation is allowed.",
            ));
        }
        let version = be(&bytes, 4)?;
        if ![2, 3, 4].contains(&version) {
            return Err(unsupported(
                "Unknown Git index format is not admitted. Git CLI can convert an ordinary index to v2.",
            ));
        }
        let count = be(&bytes, 8)? as usize;
        if count > MAX_FILES {
            return Err(error("capacity", "Git index exceeds 20,000 entries."));
        }
        let mut entries = vec![];
        let mut at = 12;
        let mut mutable = version == 2;
        let mut reason = String::new();
        for _ in 0..count {
            let start = at;
            let mode = be(&bytes, at + 24)?;
            let id = Oid::from_bytes(bytes.get(at + 40..at + 60).ok_or_else(stale)?).map_err(ge)?;
            let f = u16::from_be_bytes(
                bytes
                    .get(at + 60..at + 62)
                    .ok_or_else(stale)?
                    .try_into()
                    .map_err(|_| stale())?,
            );
            at += 62;
            if f & 0x4000 != 0 {
                at += 2;
                mutable = false;
                reason =
                    "Extended index flags (including sparse/intent-to-add) are read-only.".into();
            }
            if f & 0xb000 != 0 {
                mutable = false;
                reason = "Unmerged stages or assume-unchanged flags are read-only.".into();
            }
            let mut previous = Vec::new();
            if version == 4 {
                let mut value = 0usize;
                loop {
                    let byte = *bytes.get(at).ok_or_else(stale)?;
                    at += 1;
                    value = value.checked_add((byte & 127) as usize).ok_or_else(stale)?;
                    if byte & 128 == 0 {
                        break;
                    }
                    value = value
                        .checked_add(1)
                        .and_then(|n| n.checked_mul(128))
                        .ok_or_else(stale)?;
                }
                if let Some(e) = entries.last() {
                    let e: &Entry = e;
                    previous = e.path.clone();
                }
                if value > previous.len() {
                    return Err(stale());
                }
                previous.truncate(previous.len() - value);
            }
            let end = bytes
                .get(at..bytes.len() - 20)
                .ok_or_else(stale)?
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(stale)?
                + at;
            previous.extend_from_slice(&bytes[at..end]);
            let path = previous;
            components(&path)?;
            at = if version == 4 {
                end + 1
            } else {
                start + (end + 1 - start).div_ceil(8) * 8
            };
            if at > bytes.len() - 20 {
                return Err(stale());
            }
            if entries.last().is_some_and(|e: &Entry| {
                e.path > path || e.path == path && e.stage >= (f >> 12) & 3
            }) {
                return Err(unsupported(
                    "Duplicate or unsorted index paths are read-only.",
                ));
            }
            entries.push(Entry {
                path,
                mode,
                oid: id,
                raw: bytes[start..at].to_vec(),
                stage: (f >> 12) & 3,
            });
        }
        while at < bytes.len() - 20 {
            let name = bytes.get(at..at + 4).ok_or_else(stale)?;
            let len = be(&bytes, at + 4)? as usize;
            at += 8;
            if name != b"TREE" {
                mutable = false;
                reason="Index extensions other than the cache tree are read-only (including split, sparse, fsmonitor and untracked caches).".into();
            }
            at = at.checked_add(len).ok_or_else(stale)?;
            if at > bytes.len() - 20 {
                return Err(stale());
            }
        }
        if version != 2 {
            reason = "Index v3/v4 is read-only; mutation admits v2 only.".into();
        }
        Ok(Self {
            bytes,
            entries,
            mutable,
            reason,
        })
    }
    fn get(&self, path: &[u8]) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }
    fn updated(&self, path: &[u8], value: Option<(Oid, usize, u32)>) -> Result<Vec<u8>, FileError> {
        if !self.mutable {
            return Err(unsupported(&self.reason));
        }
        let mut entries = self.entries.clone();
        if let Some((id, len, mode)) = value {
            let mut raw = if let Some(e) = self.get(path) {
                e.raw.clone()
            } else {
                let mut b = vec![0; 62];
                b[60..62].copy_from_slice(&(path.len().min(4095) as u16).to_be_bytes());
                b.extend_from_slice(path);
                b.push(0);
                while !b.len().is_multiple_of(8) {
                    b.push(0);
                }
                b
            };
            // A zero stat cache forces future Git refresh; never claim disk equals this partial blob.
            raw[..40].fill(0);
            raw[24..28].copy_from_slice(&mode.to_be_bytes());
            raw[36..40].copy_from_slice(&(len as u32).to_be_bytes());
            raw[40..60].copy_from_slice(id.as_bytes());
            entries.retain(|e| e.path != path);
            entries.push(Entry {
                path: path.to_vec(),
                mode,
                oid: id,
                raw,
                stage: 0,
            });
        } else {
            entries.retain(|e| e.path != path);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = b"DIRC".to_vec();
        out.extend(2u32.to_be_bytes());
        out.extend((entries.len() as u32).to_be_bytes());
        for e in entries {
            out.extend(e.raw);
        }
        // TREE is a disposable cache of the old tree, deliberately invalidated.
        let checksum = Sha1::digest(&out);
        out.extend(checksum);
        Ok(out)
    }
}
#[derive(Clone)]
struct Policy {
    config: Vec<u8>,
    autocrlf: String,
    mutation: Option<String>,
    ignorecase: bool,
    external_ignores: bool,
}
#[cfg(not(test))]
static CONFIG_FILES: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
fn config_files() -> Vec<PathBuf> {
    #[cfg(test)]
    {
        vec![]
    }
    #[cfg(not(test))]
    {
        CONFIG_FILES
            .get_or_init(|| {
                let mut files = vec![
                    git2::Config::find_system().unwrap_or_else(|_| PathBuf::from("/etc/gitconfig")),
                ];
                if let Some(home) = std::env::var_os("HOME") {
                    let home = PathBuf::from(home);
                    let xdg = std::env::var_os("XDG_CONFIG_HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| home.join(".config"));
                    files.push(xdg.join("git/config"));
                    files.push(home.join(".gitconfig"));
                }
                files
            })
            .clone()
    }
}
mod config;
fn policy(root: &Path) -> Result<Policy, FileError> {
    config::read(root, config_files())
}
fn metadata_safe(path: &Path, count: &mut usize, depth: usize) -> Result<(), FileError> {
    *count += 1;
    if *count > 50_000 || depth > 64 {
        return Err(error("capacity", "Git metadata exceeds 50,000 entries."));
    }
    let m = fs::symlink_metadata(path)?;
    if m.file_type().is_symlink() || !m.is_dir() && !m.is_file() {
        return Err(unsupported(
            "Symlinked or special Git metadata is not admitted.",
        ));
    }
    if m.is_dir() {
        for e in fs::read_dir(path)? {
            metadata_safe(&e?.path(), count, depth + 1)?;
        }
    }
    Ok(())
}
struct Grant {
    id: String,
    path: PathBuf,
    root: File,
    git: File,
    commits: HashMap<String, Oid>,
    files: HashMap<String, Vec<u8>>,
    rows: Vec<Change>,
    branch: String,
    head: Option<Oid>,
    statuses: HashMap<Vec<u8>, String>,
}
impl Grant {
    fn validate(&self) -> Result<(), FileError> {
        if stamp(&fs::symlink_metadata(&self.path)?) != stamp(&self.root.metadata()?)
            || stamp(&fs::symlink_metadata(self.path.join(".git"))?) != stamp(&self.git.metadata()?)
        {
            return Err(stale());
        }
        metadata_safe(&self.path.join(".git"), &mut 0, 0)?;
        for p in [
            "objects/info/alternates",
            "objects/info/http-alternates",
            "commondir",
            "shallow",
            "info/grafts",
        ] {
            if self.path.join(".git").join(p).exists() {
                return Err(unsupported(
                    "Alternates, shallow/grafted repositories and linked worktrees are not admitted.",
                ));
            }
        }
        Ok(())
    }
    fn repo(&self) -> Result<(Repository, Policy), FileError> {
        self.validate()?;
        let p = policy(&self.path)?;
        let repo = Repository::open_ext(
            self.path.join(".git"),
            git2::RepositoryOpenFlags::BARE
                | git2::RepositoryOpenFlags::NO_SEARCH
                | git2::RepositoryOpenFlags::NO_DOTGIT,
            [] as [&std::ffi::OsStr; 0],
        )
        .map_err(ge)?;
        repo.set_config(&git2::Config::new().map_err(ge)?)
            .map_err(ge)?;
        Ok((repo, p))
    }
    fn scoped(&self, path: &[u8]) -> Result<ScopedFile, FileError> {
        let parts = components(path)?;
        let mut parent = self.root.try_clone()?;
        let mut parent_path = self.path.clone();
        for part in &parts[..parts.len() - 1] {
            parent = subdir(&parent, part)?;
            parent_path.push(part);
        }
        ScopedFile::from_parent(parent, parent_path, parts.last().ok_or_else(stale)?.clone())
    }
    fn bytes(&self, path: &[u8]) -> Result<Option<(Vec<u8>, u32)>, FileError> {
        let mut dest = self.path.clone();
        for p in components(path)? {
            dest.push(p);
        }
        match fs::symlink_metadata(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
            Ok(m) if m.is_file() => {
                let (mut f, _, _) = self.scoped(path)?.snapshot_bounded(FILE_LIMIT as u64)?;
                let mut b = vec![];
                f.read_to_end(&mut b)?;
                Ok(Some((
                    b,
                    if m.mode() & 0o111 != 0 {
                        0o100755
                    } else {
                        0o100644
                    },
                )))
            }
            _ => Err(unsupported(
                "Symlinks, submodules and nonregular worktree entries are read-only.",
            )),
        }
    }
    fn index(&self) -> Result<Index, FileError> {
        match private_store::read(&self.path.join(".git/index"), INDEX_LIMIT) {
            Ok(b) => Index::parse(b),
            Err(_)
                if fs::symlink_metadata(self.path.join(".git/index"))
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Index::parse(vec![])
            }
            Err(e) => Err(e),
        }
    }
    fn attrs(&self, path: &[u8]) -> Result<Vec<u8>, FileError> {
        let mut data = vec![];
        let parts = components(path)?;
        let mut rel = Vec::new();
        for i in 0..parts.len() {
            if i > 0 {
                if !rel.is_empty() {
                    rel.push(b'/');
                }
                rel.extend_from_slice(parts[i - 1].as_bytes());
            }
            let mut a = rel.clone();
            if !a.is_empty() {
                a.push(b'/');
            }
            a.extend_from_slice(b".gitattributes");
            if let Some((b, _)) = self.bytes(&a)? {
                data.extend(b);
            }
        }
        if self.path.join(".git/info/attributes").exists() {
            data.extend(private_store::read(
                &self.path.join(".git/info/attributes"),
                65536,
            )?);
        }
        Ok(data)
    }
}
fn head(repo: &Repository) -> Result<Option<Oid>, FileError> {
    match repo.head() {
        Ok(h) => Ok(h.target()),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(e) => Err(ge(e)),
    }
}

fn tree_files(
    repo: &Repository,
    id: Option<Oid>,
) -> Result<BTreeMap<Vec<u8>, (Oid, u32)>, FileError> {
    let mut out = BTreeMap::new();
    if let Some(id) = id {
        let tree = repo.find_commit(id).map_err(ge)?.tree().map_err(ge)?;
        let mut too_many = false;
        tree.walk(git2::TreeWalkMode::PreOrder, |root, e| {
            if out.len() >= MAX_FILES || root.len() > 16384 {
                too_many = true;
                return git2::TreeWalkResult::Abort;
            }
            if e.kind() != Some(ObjectType::Tree) {
                let mut p = root.as_bytes().to_vec();
                p.extend(e.name_bytes());
                out.insert(p, (e.id(), e.filemode() as u32));
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(ge)?;
        if too_many {
            return Err(error("capacity", "Git tree exceeds its entry/path budget."));
        }
    }
    Ok(out)
}
fn blob(repo: &Repository, id: Option<Oid>) -> Result<Vec<u8>, FileError> {
    if let Some(id) = id {
        if repo.odb().map_err(ge)?.read_header(id).map_err(ge)?.0 > FILE_LIMIT {
            return Err(error(
                "capacity",
                "Git blob exceeds 4 MiB before decompression.",
            ));
        }
        let b = repo.find_blob(id).map_err(ge)?;
        if b.size() > FILE_LIMIT {
            return Err(error(
                "capacity",
                "Git file/blob exceeds the 4 MiB diff admission.",
            ));
        }
        Ok(b.content().to_vec())
    } else {
        Ok(vec![])
    }
}
fn clean(b: &[u8], p: &Policy) -> Vec<u8> {
    if p.autocrlf == "true" || p.autocrlf == "input" {
        b.split(|c| *c == b'\n')
            .enumerate()
            .fold(Vec::new(), |mut o, (i, l)| {
                if i > 0 {
                    if o.last() == Some(&b'\r') {
                        o.pop();
                    }
                    o.push(b'\n');
                }
                o.extend(l);
                o
            })
    } else {
        b.to_vec()
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub path: String,
    pub staged: String,
    pub working: String,
    pub warning: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Commit {
    pub id: String,
    pub short: String,
    pub summary: String,
    pub time: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub head: Option<String>,
    pub files: Vec<Change>,
    pub total: usize,
    pub history: Vec<Commit>,
    pub warning: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub id: String,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub unified_row: usize,
    pub split_row: usize,
    pub unified_full_row: usize,
    pub split_full_row: usize,
}
#[derive(Clone)]
struct RowRef {
    old: Option<usize>,
    new: Option<usize>,
    kind: &'static str,
    hunk: Option<String>,
    hidden: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub kind: String,
    pub hunk: Option<String>,
    pub hidden: usize,
    pub clipped: bool,
    pub old_ending: String,
    pub new_ending: String,
    pub source_line: Option<usize>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub id: String,
    pub path: String,
    pub before: String,
    pub after: String,
    pub hunks: Vec<Hunk>,
    pub unified_rows: usize,
    pub split_rows: usize,
    pub unified_full_rows: usize,
    pub split_full_rows: usize,
    pub mutation: Option<String>,
    pub conversion: String,
}
#[derive(Serialize)]
pub struct HistoryFiles {
    pub files: Vec<Change>,
    pub total: usize,
}
struct Preview {
    id: String,
    path: Vec<u8>,
    before: String,
    after: String,
    left: Vec<u8>,
    right: Vec<u8>,
    old: Vec<String>,
    new: Vec<String>,
    raw_work: Option<Vec<u8>>,
    index: Index,
    head: Option<Oid>,
    policy: Policy,
    attrs: Vec<u8>,
    hunks: Vec<Hunk>,
    rows: [Vec<RowRef>; 4],
    mode: u32,
    head_mode: Option<u32>,
    mutation: Option<String>,
    created: Instant,
    source_lines: Vec<usize>,
}
fn text_lines(b: &[u8]) -> Result<Vec<String>, FileError> {
    let t = std::str::from_utf8(b).map_err(|_| {
        unsupported(
            "Binary/non-UTF-8 content has status only; text diff and mutation are unavailable.",
        )
    })?;
    if t.contains('\0') {
        return Err(unsupported("Binary content has status only."));
    }
    let l: Vec<_> = t.split_inclusive('\n').map(str::to_owned).collect();
    if l.len() > 50_000 {
        return Err(error("capacity", "Diff exceeds 50,000 lines per side."));
    }
    Ok(l)
}
fn display_line(text: &str) -> String {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let mut out = String::new();
    for c in text.chars() {
        let piece = match c {
            '\r' => "␍".into(),
            '\u{feff}' => "⟪BOM⟫".into(),
            c if c.is_control() && c != '\t' => format!("\\u{{{:x}}}", c as u32),
            c => c.to_string(),
        };
        if out.len() + piece.len() > 1024 {
            break;
        }
        out.push_str(&piece);
    }
    out
}
fn ending(text: Option<&String>) -> String {
    text.map(|t| {
        if t.ends_with("\r\n") {
            "CRLF"
        } else if t.ends_with('\n') {
            "LF"
        } else {
            "no newline"
        }
    })
    .unwrap_or("")
    .into()
}
fn source_lines(bytes: &[u8]) -> Vec<usize> {
    let mut out = vec![1];
    let mut line = 1;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\r' {
            line += 1;
        }
        if *b == b'\n' {
            if i == 0 || bytes[i - 1] != b'\r' {
                line += 1;
            }
            out.push(line);
        }
    }
    out
}
pub(crate) fn native_hunks(left: &[u8], right: &[u8]) -> Result<Vec<Hunk>, FileError> {
    buffer_hunks(left, right, 3)
}
pub(crate) fn native_review_hunks(left: &[u8], right: &[u8]) -> Result<Vec<Hunk>, FileError> {
    buffer_hunks(left, right, 0)
}
fn buffer_hunks(left: &[u8], right: &[u8], context: u32) -> Result<Vec<Hunk>, FileError> {
    let mut options = git2::DiffOptions::new();
    options
        .context_lines(context)
        .interhunk_lines(0)
        .force_text(true)
        .minimal(true);
    let patch =
        git2::Patch::from_buffers(left, None, right, None, Some(&mut options)).map_err(ge)?;
    if patch.num_hunks() > 1000 {
        return Err(error("capacity", "Diff exceeds 1,000 hunks."));
    }
    (0..patch.num_hunks())
        .map(|i| {
            let (h, _) = patch.hunk(i).map_err(ge)?;
            Ok(Hunk {
                id: uid(),
                old_start: if h.old_start() == 0 {
                    0
                } else {
                    h.old_start() as usize - 1
                },
                old_lines: h.old_lines() as usize,
                new_start: if h.new_start() == 0 {
                    0
                } else {
                    h.new_start() as usize - 1
                },
                new_lines: h.new_lines() as usize,
                unified_row: 0,
                split_row: 0,
                unified_full_row: 0,
                split_full_row: 0,
            })
        })
        .collect()
}
fn rows(old: &[String], new: &[String], hunks: &mut [Hunk]) -> Result<[Vec<RowRef>; 4], FileError> {
    let mut out: [Vec<RowRef>; 4] = std::array::from_fn(|_| vec![]);
    let (mut a, mut b) = (0, 0);
    for h in hunks {
        let gap = h.old_start.saturating_sub(a);
        for (n, rows) in out.iter_mut().enumerate() {
            if n < 2 {
                if gap > 0 {
                    rows.push(RowRef {
                        old: Some(a),
                        new: Some(b),
                        kind: "gap",
                        hunk: None,
                        hidden: gap,
                    });
                }
            } else {
                for i in 0..gap {
                    rows.push(RowRef {
                        old: Some(a + i),
                        new: Some(b + i),
                        kind: "context",
                        hunk: None,
                        hidden: 0,
                    });
                }
            }
        }
        h.unified_row = out[0].len();
        h.split_row = out[1].len();
        h.unified_full_row = out[2].len();
        h.split_full_row = out[3].len();
        let l = old
            .get(h.old_start..h.old_start + h.old_lines)
            .ok_or_else(stale)?
            .concat();
        let r = new
            .get(h.new_start..h.new_start + h.new_lines)
            .ok_or_else(stale)?
            .concat();
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(u32::MAX).force_text(true);
        let patch =
            git2::Patch::from_buffers(l.as_bytes(), None, r.as_bytes(), None, Some(&mut opts))
                .map_err(ge)?;
        let (mut x, mut y) = (h.old_start, h.new_start);
        let mut minus = vec![];
        let mut plus = vec![];
        let flush = |out: &mut [Vec<RowRef>; 4], minus: &mut Vec<usize>, plus: &mut Vec<usize>| {
            for i in 0..minus.len().max(plus.len()) {
                let row = RowRef {
                    old: minus.get(i).copied(),
                    new: plus.get(i).copied(),
                    kind: "change",
                    hunk: Some(h.id.clone()),
                    hidden: 0,
                };
                out[1].push(row.clone());
                out[3].push(row);
            }
            minus.clear();
            plus.clear();
        };
        for k in 0..patch.num_hunks() {
            let (_, len) = patch.hunk(k).map_err(ge)?;
            for j in 0..len {
                let line = patch.line_in_hunk(k, j).map_err(ge)?;
                let row = match line.origin() {
                    ' ' => {
                        flush(&mut out, &mut minus, &mut plus);
                        let r = RowRef {
                            old: Some(x),
                            new: Some(y),
                            kind: "context",
                            hunk: Some(h.id.clone()),
                            hidden: 0,
                        };
                        x += 1;
                        y += 1;
                        out[1].push(r.clone());
                        out[3].push(r.clone());
                        r
                    }
                    '-' => {
                        minus.push(x);
                        let r = RowRef {
                            old: Some(x),
                            new: None,
                            kind: "delete",
                            hunk: Some(h.id.clone()),
                            hidden: 0,
                        };
                        x += 1;
                        r
                    }
                    '+' => {
                        plus.push(y);
                        let r = RowRef {
                            old: None,
                            new: Some(y),
                            kind: "add",
                            hunk: Some(h.id.clone()),
                            hidden: 0,
                        };
                        y += 1;
                        r
                    }
                    _ => continue,
                };
                out[0].push(row.clone());
                out[2].push(row);
            }
        }
        flush(&mut out, &mut minus, &mut plus);
        a = h.old_start + h.old_lines;
        b = h.new_start + h.new_lines;
    }
    let gap = old.len().saturating_sub(a);
    for (n, r) in out.iter_mut().enumerate() {
        if n < 2 {
            if gap > 0 {
                r.push(RowRef {
                    old: Some(a),
                    new: Some(b),
                    kind: "gap",
                    hunk: None,
                    hidden: gap,
                });
            }
        } else {
            for i in 0..gap {
                r.push(RowRef {
                    old: Some(a + i),
                    new: Some(b + i),
                    kind: "context",
                    hunk: None,
                    hidden: 0,
                });
            }
        }
    }
    Ok(out)
}
struct IndexLock {
    dir: File,
    file: File,
    owned: bool,
}
impl IndexLock {
    fn acquire(git: &File) -> Result<Self, FileError> {
        let fd = unsafe {
            libc::openat(
                git.as_raw_fd(),
                c"index.lock".as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(error(
                "git-lock",
                "Git index is locked. Wait for the other Git operation; its lock is never removed.",
            ));
        }
        let lock = Self {
            dir: git.try_clone()?,
            file: unsafe { File::from_raw_fd(fd) },
            owned: true,
        };
        let existing = unsafe {
            libc::openat(
                git.as_raw_fd(),
                c"index".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if existing >= 0 {
            let source = unsafe { File::from_raw_fd(existing) };
            crate::atomic_save::metadata(&source,&lock.file).map_err(|_|unsupported("Index ownership, links, read-only bits or metadata cannot be preserved; use external Git."))?;
        } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
            return Err(stale());
        }
        Ok(lock)
    }
    fn verify(&self) -> Result<(), FileError> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                c"index.lock".as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
            || (st.st_dev as u64, st.st_ino as u64) != stamp(&self.file.metadata()?)
        {
            return Err(stale());
        }
        Ok(())
    }
    fn publish(
        mut self,
        b: &[u8],
        validate: impl FnOnce() -> Result<(), FileError>,
    ) -> Result<(), FileError> {
        self.file.write_all(b)?;
        private_store::sync(&self.file)?;
        validate()?;
        self.verify()?;
        if unsafe {
            libc::renameat(
                self.dir.as_raw_fd(),
                c"index.lock".as_ptr(),
                self.dir.as_raw_fd(),
                c"index".as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        self.owned = false;
        private_store::sync(&self.dir).map_err(|_|error("git-uncertain","Index was published, but parent sync failed. Refresh and inspect it before retrying."))?;
        Ok(())
    }
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.owned && self.verify().is_ok() {
            unsafe {
                libc::unlinkat(self.dir.as_raw_fd(), c"index.lock".as_ptr(), 0);
            }
        }
    }
}
#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    schema: u8,
    pub id: String,
    pub path: String,
    pub created: u64,
    repo: String,
    hash: String,
}
#[derive(Serialize)]
pub struct Mutated {
    pub message: String,
    pub backup: Option<String>,
}
pub struct Workbench {
    grant: Option<Grant>,
    preview: Option<Preview>,
    store: PathBuf,
    protected: Vec<PathBuf>,
}
impl Workbench {
    pub fn new(store: PathBuf, protected: Vec<PathBuf>) -> Self {
        Self {
            grant: None,
            preview: None,
            store,
            protected,
        }
    }
    pub fn choose(&mut self, path: &Path) -> Result<State, FileError> {
        if self.store.as_os_str().is_empty() {
            return Err(error(
                "private-storage",
                "App-private storage must be available before opening Git.",
            ));
        }
        let path = fs::canonicalize(path)?;
        if self.protected.iter().any(|p| path.starts_with(p)) {
            return Err(unsupported(
                "App-owned/private resources cannot be a repository grant.",
            ));
        }
        let root = dir(&path)?;
        let git = subdir(&root, std::ffi::OsStr::new(".git"))?;
        let g = Grant {
            id: uid(),
            path,
            root,
            git,
            commits: HashMap::new(),
            files: HashMap::new(),
            rows: vec![],
            branch: String::new(),
            head: None,
            statuses: HashMap::new(),
        };
        g.repo()?;
        self.grant = Some(g);
        self.preview = None;
        let id = self.grant.as_ref().unwrap().id.clone();
        self.refresh(&id, 0)
    }
    fn grant(&self, id: &str) -> Result<&Grant, FileError> {
        self.grant.as_ref().filter(|g| g.id == id).ok_or_else(|| {
            error(
                "git-grant",
                "Select the repository again; this native grant is unavailable.",
            )
        })
    }
    pub fn selected(&self) -> bool {
        self.grant.is_some()
    }
    pub fn badges(&self, files: Vec<(String, PathBuf)>) -> HashMap<String, String> {
        let Some(g) = &self.grant else {
            return HashMap::new();
        };
        if fs::symlink_metadata(&g.path).ok().map(|m| stamp(&m))
            != g.root.metadata().ok().map(|m| stamp(&m))
        {
            return HashMap::new();
        }
        files
            .into_iter()
            .filter_map(|(id, p)| {
                p.strip_prefix(&g.path)
                    .ok()
                    .and_then(|p| g.statuses.get(p.as_os_str().as_bytes()))
                    .map(|status| (id, status.clone()))
            })
            .collect()
    }
    pub fn revoke(&mut self, id: &str) -> Result<(), FileError> {
        self.grant(id)?;
        self.grant = None;
        self.preview = None;
        Ok(())
    }
    pub fn refresh(&mut self, id: &str, start: usize) -> Result<State, FileError> {
        let g = self.grant(id)?;
        let (repo, p) = g.repo()?;
        let index = g.index()?;
        let head = head(&repo)?;
        let tree = tree_files(&repo, head)?;
        let mut paths: BTreeMap<Vec<u8>, ()> = tree.keys().cloned().map(|p| (p, ())).collect();
        for e in &index.entries {
            paths.insert(e.path.clone(), ());
        }
        let mut builder = ignore::gitignore::GitignoreBuilder::new(&g.path);
        builder
            .case_insensitive(p.ignorecase)
            .map_err(|e| error("git-ignore", &e.to_string()))?;
        if g.path.join(".git/info/exclude").exists() {
            let bytes = private_store::read(&g.path.join(".git/info/exclude"), 65536)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| unsupported("Info exclude must be UTF-8."))?;
            for line in text.lines() {
                builder
                    .add_line(None, line)
                    .map_err(|e| error("git-ignore", &e.to_string()))?;
            }
        }
        let base_ignore = builder
            .build()
            .map_err(|e| error("git-ignore", &e.to_string()))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut todo = vec![(g.path.clone(), vec![base_ignore])];
        let mut scanned = 0;
        let mut truncated = false;
        while let Some((folder, mut ignores)) = todo.pop() {
            let mut relative = folder
                .strip_prefix(&g.path)
                .map_err(|_| stale())?
                .as_os_str()
                .as_bytes()
                .to_vec();
            if !relative.is_empty() {
                relative.push(b'/');
            }
            relative.extend_from_slice(b".gitignore");
            if let Some((bytes, _)) = g.bytes(&relative)? {
                if bytes.len() > 65536 {
                    return Err(error("capacity", "Ignore file exceeds 64 KiB."));
                }
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| unsupported("Ignore files must be UTF-8."))?;
                let mut builder = ignore::gitignore::GitignoreBuilder::new(&folder);
                builder
                    .case_insensitive(p.ignorecase)
                    .map_err(|e| error("git-ignore", &e.to_string()))?;
                for line in text.lines() {
                    builder
                        .add_line(Some(folder.join(".gitignore")), line)
                        .map_err(|e| error("git-ignore", &e.to_string()))?;
                }
                ignores.push(
                    builder
                        .build()
                        .map_err(|e| error("git-ignore", &e.to_string()))?,
                );
            }
            for e in fs::read_dir(&folder)? {
                let e = e?;
                scanned += 1;
                if scanned > MAX_FILES || Instant::now() > deadline {
                    truncated = true;
                    break;
                }
                let path = e.path();
                let rel = path.strip_prefix(&g.path).map_err(|_| stale())?;
                if rel.components().any(|c| c.as_os_str() == ".git")
                    || self.protected.iter().any(|p| path.starts_with(p))
                {
                    continue;
                }
                let m = e.file_type()?;
                if ignores
                    .iter()
                    .rev()
                    .map(|i| i.matched_path_or_any_parents(&path, m.is_dir()))
                    .find(|m| !m.is_none())
                    .is_some_and(|m| m.is_ignore())
                {
                    continue;
                }
                if m.is_dir() {
                    if rel.components().count() < 32 {
                        todo.push((path, ignores.clone()));
                    } else {
                        truncated = true;
                    }
                } else {
                    paths.insert(rel.as_os_str().as_bytes().to_vec(), ());
                }
            }
            if truncated {
                break;
            }
        }
        let mut rows = vec![];
        let mut files = HashMap::new();
        let mut work_budget = 0;
        let mut adds = false;
        let mut deletes = false;
        let has_attrs = paths
            .keys()
            .any(|p| p.rsplit(|b| *b == b'/').next() == Some(b".gitattributes"))
            || g.path.join(".git/info/attributes").exists();
        for path in paths.keys() {
            if rows.len() >= MAX_FILES || Instant::now() > deadline {
                truncated = true;
                break;
            }
            let ie = index.get(path);
            let he = tree.get(path);
            let staged = if index
                .entries
                .iter()
                .any(|e| e.path == *path && e.stage != 0)
            {
                "conflicted"
            } else {
                match (he, ie) {
                    (None, Some(_)) => "added",
                    (Some(_), None) => "deleted",
                    (Some(h), Some(i)) if h.0 != i.oid || h.1 != i.mode => "modified",
                    _ => "clean",
                }
            };
            let (mut warning, working) = if self
                .protected
                .iter()
                .any(|p| g.path.join(OsString::from_vec(path.clone())).starts_with(p))
            {
                (
                    Some("Protected app/private file; unavailable.".into()),
                    "unavailable",
                )
            } else if ie.is_some_and(|e| ![0o100644, 0o100755].contains(&e.mode))
                || he.is_some_and(|e| ![0o100644, 0o100755].contains(&e.1))
            {
                (
                    Some("Symlink/submodule/type change; status only.".into()),
                    "unsupported",
                )
            } else if work_budget > 256 * 1024 * 1024 {
                truncated = true;
                (
                    Some("Status content budget reached; refresh a smaller repository.".into()),
                    "unavailable",
                )
            } else {
                match g.bytes(path) {
                    Ok(Some((b, mode))) => {
                        work_budget += b.len();
                        let binary = std::str::from_utf8(&b).is_err() || b.contains(&0);
                        let canonical_index = if p.autocrlf != "false" {
                            blob(&repo, ie.map(|e| e.oid))
                                .map(|b| b.contains(&b'\r'))
                                .unwrap_or(true)
                        } else {
                            false
                        };
                        let uncertain = has_attrs || p.mutation.is_some() || canonical_index;
                        let bytes = if uncertain || binary {
                            b.clone()
                        } else {
                            clean(&b, &p)
                        };
                        let w = match ie {
                            None => "untracked",
                            Some(i) if uncertain => {
                                if oid(&bytes)? == i.oid {
                                    "raw-equal"
                                } else {
                                    "raw-diff"
                                }
                            }
                            Some(i) if oid(&bytes)? != i.oid || mode != i.mode => "modified",
                            _ => "clean",
                        };
                        let warn = if uncertain {
                            p.mutation.clone().or_else(||Some("Raw saved-byte comparison only; attributes or noncanonical EOL prevent safe transformation.".into()))
                        } else if ie.is_none() && p.external_ignores {
                            Some("Untracked candidate: external ignore policy is not followed. Staging requires external Git.".into())
                        } else if binary {
                            Some("Binary/non-UTF-8; status only.".into())
                        } else if ie.is_some_and(|i| mode != i.mode) {
                            Some("Mode changes are read-only.".into())
                        } else {
                            None
                        };
                        (warn, w)
                    }
                    Ok(None) => (None, if ie.is_some() { "deleted" } else { "clean" }),
                    Err(e) => (Some(e.message), "unavailable"),
                }
            };
            if staged == "clean" && working == "clean" {
                continue;
            }
            adds |= working == "untracked" || staged == "added";
            deletes |= working == "deleted" || staged == "deleted";
            if warning.is_none() && p.mutation.is_some() {
                warning = p.mutation.clone();
            }
            let entry_id = uid();
            files.insert(entry_id.clone(), path.clone());
            rows.push(Change {
                id: entry_id,
                path: display_path(path),
                staged: staged.into(),
                working: working.into(),
                warning,
            });
        }
        if adds && deletes {
            for row in &mut rows {
                if row.working == "untracked"
                    || row.working == "deleted"
                    || row.staged == "added"
                    || row.staged == "deleted"
                {
                    row.warning=Some("Possible rename/delete-add pair; inspect raw comparisons. Mutations for these paths are refused.".into());
                }
            }
        }
        let mut commits = HashMap::new();
        let mut history = vec![];
        let mut current = head;
        for _ in 0..50 {
            let Some(id) = current else { break };
            let commit = repo.find_commit(id).map_err(ge)?;
            let key = uid();
            commits.insert(key.clone(), id);
            history.push(Commit {
                id: key,
                short: id.to_string()[..8].into(),
                summary: commit
                    .summary()
                    .ok()
                    .flatten()
                    .unwrap_or("Untitled commit")
                    .chars()
                    .take(240)
                    .collect(),
                time: commit.time().seconds(),
            });
            current = commit.parent_id(0).ok();
        }
        let head_text = private_store::read(&g.path.join(".git/HEAD"), 65536)?;
        let branch = String::from_utf8_lossy(&head_text)
            .trim()
            .strip_prefix("ref: refs/heads/")
            .map(str::to_owned)
            .unwrap_or_else(|| "Detached HEAD".into());
        let total = rows.len();
        let name = g
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let g = self.grant.as_mut().unwrap();
        g.statuses = rows
            .iter()
            .filter_map(|r| {
                files.get(&r.id).map(|p| {
                    (
                        p.clone(),
                        format!("Index {} / Disk {}", r.staged, r.working),
                    )
                })
            })
            .collect();
        g.files = files;
        g.rows = rows;
        g.commits = commits;
        g.branch = branch.clone();
        g.head = head;
        self.preview = None;
        Ok(State {
            id: id.into(),
            name,
            branch,
            head: head.map(|h| h.to_string()),
            files: g
                .rows
                .iter()
                .skip(start.min(total))
                .take(100)
                .cloned()
                .collect(),
            total,
            history,
            warning: if truncated {
                Some("Status scan reached its entry/time/content budget; this list is incomplete. Mutations require fresh exact file/index captures.".into())
            } else if !index.mutable {
                Some(index.reason)
            } else {
                None
            },
        })
    }
    pub fn history_files(
        &mut self,
        id: &str,
        commit: &str,
        start: usize,
    ) -> Result<HistoryFiles, FileError> {
        let g = self.grant(id)?;
        let (repo, _) = g.repo()?;
        let oid = *g.commits.get(commit).ok_or_else(stale)?;
        let files = tree_files(&repo, Some(oid))?;
        let mut handles = HashMap::new();
        let rows: Vec<Change> = files
            .into_keys()
            .map(|path| {
                let key = uid();
                handles.insert(key.clone(), path.clone());
                Change {
                    id: key,
                    path: display_path(&path),
                    staged: "history".into(),
                    working: "current".into(),
                    warning: Some("Historical source comparison; mutation is read-only.".into()),
                }
            })
            .collect();
        let total = rows.len();
        let visible = rows
            .iter()
            .skip(start.min(total))
            .take(100)
            .cloned()
            .collect();
        let g = self.grant.as_mut().ok_or_else(stale)?;
        g.files = handles;
        g.rows = rows;
        self.preview = None;
        Ok(HistoryFiles {
            files: visible,
            total,
        })
    }
    pub fn scoped(&self, id: &str, file: &str) -> Result<ScopedFile, FileError> {
        let g = self.grant(id)?;
        g.validate()?;
        let p = g.files.get(file).ok_or_else(stale)?;
        let path = g.path.join(OsString::from_vec(p.clone()));
        if self.protected.iter().any(|x| path.starts_with(x)) {
            return Err(unsupported("Protected file."));
        }
        g.scoped(p)
    }
    pub fn compare(
        &mut self,
        id: &str,
        file: &str,
        before: &str,
        after: &str,
    ) -> Result<Diff, FileError> {
        let g = self.grant(id)?;
        let (repo, p) = g.repo()?;
        let path = g.files.get(file).ok_or_else(stale)?.clone();
        if self
            .protected
            .iter()
            .any(|p| g.path.join(OsString::from_vec(path.clone())).starts_with(p))
        {
            return Err(unsupported("Protected app/private file."));
        }
        let index = g.index()?;
        let head = head(&repo)?;
        if [before, after].contains(&"index")
            && index.entries.iter().any(|e| e.path == path && e.stage != 0)
        {
            return Err(unsupported(
                "The index has unresolved stages; use HEAD → Disk to inspect source without choosing an invented stage.",
            ));
        }
        let attrs = g.attrs(&path)?;
        let ie = index.get(&path);
        let head_tree = tree_files(&repo, head)?;
        let he = head_tree.get(&path);
        let raw_work = g.bytes(&path)?;
        let mode = ie
            .map(|e| e.mode)
            .or(raw_work.as_ref().map(|w| w.1))
            .or(he.map(|e| e.1))
            .unwrap_or(0o100644);
        if ![0o100644, 0o100755].contains(&mode) {
            return Err(unsupported("Symlink/submodule/type change: status only."));
        }
        let noncanonical =
            p.autocrlf != "false" && blob(&repo, ie.map(|e| e.oid))?.contains(&b'\r');
        let raw_compare = noncanonical
            || p.mutation.is_some()
            || g.rows
                .iter()
                .find(|r| r.id == file)
                .and_then(|r| r.warning.as_ref())
                .is_some_and(|s| s.starts_with("Raw saved-byte"))
            || !attrs.iter().all(u8::is_ascii_whitespace);
        let side = |name: &str| -> Result<Vec<u8>, FileError> {
            match name {
                "index" => blob(&repo, ie.map(|e| e.oid)),
                "head" => blob(&repo, he.map(|e| e.0)),
                "worktree" => {
                    let b = raw_work.as_ref().map(|w| w.0.clone()).unwrap_or_default();
                    Ok(if raw_compare { b } else { clean(&b, &p) })
                }
                key => {
                    let commit = g.commits.get(key).ok_or_else(|| {
                        unsupported("Choose a commit from the bounded native history.")
                    })?;
                    let tree = tree_files(&repo, Some(*commit))?;
                    blob(&repo, tree.get(&path).map(|e| e.0))
                }
            }
        };
        let left = side(before)?;
        let right = side(after)?;
        let old = text_lines(&left)?;
        let new = text_lines(&right)?;
        let mut hunks = native_hunks(&left, &right)?;
        if hunks.is_empty()
            && ((before == "index" && ie.is_none() && raw_work.is_some())
                || (before == "head" && he.is_none() && ie.is_some()))
        {
            hunks.push(Hunk {
                id: uid(),
                old_start: 0,
                old_lines: old.len(),
                new_start: 0,
                new_lines: new.len(),
                unified_row: 0,
                split_row: 0,
                unified_full_row: 0,
                split_full_row: 0,
            });
        }
        let rows = rows(&old, &new, &mut hunks)?;
        let mut mutation = if !index.mutable {
            Some(index.reason.clone())
        } else {
            p.mutation.clone()
        };
        if !attrs.iter().all(u8::is_ascii_whitespace) {
            mutation=Some("Applicable .gitattributes/info attributes are read-only; no filter, encoding or EOL attribute is executed or guessed.".into());
        }
        if let Some(w) = g
            .rows
            .iter()
            .find(|r| r.id == file)
            .and_then(|r| r.warning.clone())
        {
            mutation = Some(w);
        }
        if raw_work.as_ref().is_some_and(|w| w.1 != mode) {
            mutation = Some("Mode changes are read-only.".into());
        }
        if noncanonical {
            mutation=Some("Index has noncanonical CR bytes under autocrlf; raw comparison only, use external Git.".into());
        }
        if ie.is_none() && p.external_ignores {
            mutation=Some("External ignore policy is not followed for untracked candidates; use external Git to add this path.".into());
        }
        if std::str::from_utf8(&path).is_err() {
            mutation =
                Some("Non-UTF-8 path display is lossy; mutations require external Git.".into());
        }
        if ie.is_none()
            && index.entries.iter().any(|e| {
                String::from_utf8_lossy(&e.path).to_lowercase()
                    == String::from_utf8_lossy(&path).to_lowercase()
            })
        {
            mutation = Some("Case-alias index paths require external Git.".into());
        }
        if !((before == "index" && after == "worktree") || (before == "head" && after == "index")) {
            mutation=Some("This baseline is read-only. Use Index → Disk for stage/discard or HEAD → Index for unstage.".into());
        }
        let result = Diff {
            id: uid(),
            path: display_path(&path),
            before: before.into(),
            after: after.into(),
            hunks: hunks.clone(),
            unified_rows: rows[0].len(),
            split_rows: rows[1].len(),
            unified_full_rows: rows[2].len(),
            split_full_rows: rows[3].len(),
            mutation: mutation.clone(),
            conversion: if p.autocrlf == "false" || raw_compare {
                "Raw saved bytes; no configured filters".into()
            } else {
                format!(
                    "Saved bytes with built-in CRLF → LF clean (core.autocrlf={}); no external filters",
                    p.autocrlf
                )
            },
        };
        let work_lines = source_lines(
            raw_work
                .as_ref()
                .map(|w| w.0.as_slice())
                .unwrap_or_default(),
        );
        self.preview = Some(Preview {
            id: result.id.clone(),
            path,
            before: before.into(),
            after: after.into(),
            left,
            right,
            old,
            new,
            raw_work: raw_work.map(|w| w.0),
            index,
            head,
            policy: p,
            attrs,
            hunks,
            rows,
            mode,
            head_mode: he.map(|e| e.1),
            mutation,
            created: Instant::now(),
            source_lines: work_lines,
        });
        Ok(result)
    }
    pub fn page(
        &self,
        id: &str,
        preview: &str,
        start: usize,
        split: bool,
        expanded: bool,
    ) -> Result<Vec<Row>, FileError> {
        self.grant(id)?;
        let p = self
            .preview
            .as_ref()
            .filter(|p| p.id == preview)
            .ok_or_else(stale)?;
        let rows = &p.rows[usize::from(split) + usize::from(expanded) * 2];
        Ok(rows
            .iter()
            .skip(start.min(rows.len()))
            .take(200)
            .map(|r| {
                let old = r.old.and_then(|n| p.old.get(n));
                let new = r.new.and_then(|n| p.new.get(n));
                Row {
                    old_number: r.old.map(|n| n + 1),
                    new_number: r.new.map(|n| n + 1),
                    old_text: old.map(|t| display_line(t)),
                    new_text: new.map(|t| display_line(t)),
                    kind: r.kind.into(),
                    hunk: r.hunk.clone(),
                    hidden: r.hidden,
                    clipped: old.is_some_and(|t| t.len() > 1024)
                        || new.is_some_and(|t| t.len() > 1024),
                    old_ending: ending(old),
                    new_ending: ending(new),
                    source_line: {
                        let side = if p.after == "worktree" {
                            r.new.or_else(|| {
                                r.hunk.as_ref().and_then(|id| {
                                    p.hunks.iter().find(|h| &h.id == id).map(|h| h.new_start)
                                })
                            })
                        } else if p.before == "worktree" {
                            r.old.or_else(|| {
                                r.hunk.as_ref().and_then(|id| {
                                    p.hunks.iter().find(|h| &h.id == id).map(|h| h.old_start)
                                })
                            })
                        } else {
                            None
                        };
                        let map = &p.source_lines;
                        side.map(|n| map[n.min(map.len() - 1)])
                    },
                }
            })
            .collect())
    }
    pub fn mutate(
        &mut self,
        id: &str,
        preview: &str,
        action: &str,
        selected: &[String],
    ) -> Result<Mutated, FileError> {
        self.mutate_hook(id, preview, action, selected, || {})
    }
    fn mutate_hook(
        &mut self,
        id: &str,
        preview: &str,
        action: &str,
        selected: &[String],
        hook: impl FnOnce(),
    ) -> Result<Mutated, FileError> {
        let g = self.grant(id)?;
        let p = self
            .preview
            .as_ref()
            .filter(|p| p.id == preview && p.created.elapsed() < Duration::from_secs(300))
            .ok_or_else(stale)?;
        if let Some(reason) = &p.mutation {
            return Err(unsupported(reason));
        }
        let valid = if action == "stage" || action == "discard" {
            p.before == "index" && p.after == "worktree"
        } else {
            action == "unstage" && p.before == "head" && p.after == "index"
        };
        if !valid {
            return Err(unsupported("Action does not match this captured baseline."));
        }
        if selected.is_empty() || selected.len() > 1000 {
            return Err(unsupported("Select at least one captured whole hunk."));
        }
        let wanted: HashSet<_> = selected.iter().collect();
        if wanted.len() != selected.len()
            || selected
                .iter()
                .any(|id| !p.hunks.iter().any(|h| &h.id == id))
        {
            return Err(stale());
        }
        if action == "discard" && selected.len() != p.hunks.len() {
            return Err(unsupported(
                "Discard is whole-file only and requires confirmation plus a durable recovery copy.",
            ));
        }
        let lock = IndexLock::acquire(&g.git)?;
        hook();
        let (repo, current_policy) = g.repo()?;
        if g.index()?.bytes != p.index.bytes
            || head(&repo)? != p.head
            || current_policy.config != p.policy.config
            || g.attrs(&p.path)? != p.attrs
        {
            return Err(stale());
        }
        let current = g.bytes(&p.path)?;
        if current.as_ref().map(|w| &w.0) != p.raw_work.as_ref() {
            return Err(stale());
        }
        if action == "discard" {
            if p.index.get(&p.path).is_none() {
                return Err(unsupported(
                    "Untracked files are never deleted by Git discard; use reviewed Library operations.",
                ));
            }
            let raw=p.raw_work.as_ref().ok_or_else(||unsupported("Restoring a deleted path requires an explicit file restore outside this discard slice."))?;
            let mut replacement = p.left.clone();
            if p.policy.autocrlf == "true" {
                if raw
                    .windows(1)
                    .enumerate()
                    .any(|(i, b)| b == b"\n" && (i == 0 || raw[i - 1] != b'\r'))
                {
                    return Err(unsupported(
                        "Mixed/LF worktree EOL with autocrlf=true is read-only for discard.",
                    ));
                }
                replacement = replacement
                    .into_iter()
                    .flat_map(|b| {
                        if b == b'\n' {
                            vec![b'\r', b'\n']
                        } else {
                            vec![b]
                        }
                    })
                    .collect();
            }
            let backup = self.backup(g, &p.path, raw)?;
            let target = g.scoped(&p.path)?;
            let file = target.open()?;
            let identity = atomic_save::Identity::of(&file.metadata()?);
            let source = Provenance::new(&replacement)
                .map_err(|_| unsupported("Discard replacement is not supported UTF-8."))?;
            if g.index()?.bytes != p.index.bytes
                || head(&repo)? != p.head
                || policy(&g.path)?.config != p.policy.config
                || g.attrs(&p.path)? != p.attrs
            {
                return Err(stale());
            }
            atomic_save::save(
                &target,
                &source.snapshot(),
                &hash(raw),
                &identity,
                |_, _| Err(stale()),
            )?;
            drop(lock);
            self.preview = None;
            return Ok(Mutated{message:"Discarded saved file changes. The exact previous bytes are retained in Git recovery; export an independent copy to restore them.".into(),backup:Some(backup)});
        }
        let reverse = action == "unstage";
        let mut result = if reverse {
            p.right.clone()
        } else {
            p.left.clone()
        };
        let base_lines = if reverse { &p.new } else { &p.old };
        let replacements = if reverse { &p.old } else { &p.new };
        for h in p.hunks.iter().rev().filter(|h| wanted.contains(&h.id)) {
            let (start, count, other, other_count) = if reverse {
                (h.new_start, h.new_lines, h.old_start, h.old_lines)
            } else {
                (h.old_start, h.old_lines, h.new_start, h.new_lines)
            };
            let from = base_lines[..start].iter().map(String::len).sum::<usize>();
            let to = from
                + base_lines[start..start + count]
                    .iter()
                    .map(String::len)
                    .sum::<usize>();
            result.splice(
                from..to,
                replacements[other..other + other_count].concat().bytes(),
            );
        }
        let remove = selected.len() == p.hunks.len()
            && if reverse {
                p.head_mode.is_none()
            } else {
                p.raw_work.is_none()
            };
        let value = if remove {
            None
        } else {
            let id = repo.blob(&result).map_err(ge)?;
            Some((
                id,
                result.len(),
                if reverse {
                    p.head_mode.unwrap_or(p.mode)
                } else {
                    p.mode
                },
            ))
        };
        let bytes = p.index.updated(&p.path, value)?;
        // Fence again immediately before publication; no live Index::write is nested here.
        if g.index()?.bytes != p.index.bytes
            || head(&repo)? != p.head
            || policy(&g.path)?.config != p.policy.config
            || g.attrs(&p.path)? != p.attrs
            || g.bytes(&p.path)?.as_ref().map(|w| &w.0) != p.raw_work.as_ref()
        {
            return Err(stale());
        }
        g.validate()?;
        lock.publish(&bytes, || {
            if g.index()?.bytes != p.index.bytes
                || head(&repo)? != p.head
                || policy(&g.path)?.config != p.policy.config
                || g.attrs(&p.path)? != p.attrs
                || g.bytes(&p.path)?.as_ref().map(|w| &w.0) != p.raw_work.as_ref()
            {
                return Err(stale());
            }
            g.validate()
        })?;
        self.preview = None;
        Ok(Mutated {
            message: format!(
                "{} {} captured hunk(s). Refresh to inspect the index.",
                if reverse { "Unstaged" } else { "Staged" },
                selected.len()
            ),
            backup: None,
        })
    }
    fn backup(&self, g: &Grant, path: &[u8], bytes: &[u8]) -> Result<String, FileError> {
        let folder = self.store.join("git-discard");
        private_store::directory(&folder)?;
        if private_store::usage(&self.store)?.saturating_add(bytes.len() as u64 + 65536)
            > private_store::STORE_QUOTA
        {
            return Err(error(
                "quota",
                "Git recovery quota is full. Export/delete an old Git recovery copy before discarding.",
            ));
        }
        if fs::read_dir(&folder)?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
            >= 100
        {
            return Err(error(
                "capacity",
                "Git recovery contains 100 copies. Review/export/delete an old copy first.",
            ));
        }
        let id = uid();
        let record = Backup {
            schema: 1,
            id: id.clone(),
            path: display_path(path),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            repo: path_key(&g.path),
            hash: hash(bytes),
        };
        private_store::atomic(&folder.join(format!("{id}.bin")), bytes)?;
        private_store::atomic(
            &folder.join(format!("{id}.json")),
            &serde_json::to_vec(&record).map_err(|_| stale())?,
        )?;
        Ok(id)
    }
    pub fn backups(&self, id: &str) -> Result<Vec<Backup>, FileError> {
        let g = self.grant(id)?;
        let folder = self.store.join("git-discard");
        if !folder.exists() {
            return Ok(vec![]);
        }
        let mut result = vec![];
        for e in fs::read_dir(folder)?.take(201) {
            let e = e?;
            if e.path().extension().is_none_or(|s| s != "json") {
                continue;
            }
            let b: Backup = serde_json::from_slice(&private_store::read(&e.path(), 65536)?)
                .map_err(|_| {
                    unsupported("Git recovery metadata is damaged; original records are kept.")
                })?;
            if b.schema == 1 && b.repo == path_key(&g.path) && Uuid::parse_str(&b.id).is_ok() {
                result.push(b);
            }
        }
        result.sort_by_key(|r| std::cmp::Reverse(r.created));
        Ok(result)
    }
    pub fn backup_bytes(&self, id: &str, backup: &str) -> Result<Vec<u8>, FileError> {
        let b = self
            .backups(id)?
            .into_iter()
            .find(|b| b.id == backup)
            .ok_or_else(stale)?;
        let bytes = private_store::read(
            &self.store.join("git-discard").join(format!("{}.bin", b.id)),
            FILE_LIMIT as u64,
        )?;
        if hash(&bytes) != b.hash {
            return Err(unsupported(
                "Git recovery checksum failed. Original bytes are kept.",
            ));
        }
        Ok(bytes)
    }
    pub fn forget_backup(&self, id: &str, backup: &str) -> Result<(), FileError> {
        self.backup_bytes(id, backup)?;
        let folder = self.store.join("git-discard");
        fs::remove_file(folder.join(format!("{backup}.json")))?;
        fs::remove_file(folder.join(format!("{backup}.bin")))?;
        private_store::sync(&dir(&folder)?)?;
        Ok(())
    }
}
/// Called once by the single-threaded app entry before any Git service operation.
pub fn initialize() -> Result<(), FileError> {
    let _ = config_files();
    for level in [
        git2::ConfigLevel::System,
        git2::ConfigLevel::Global,
        git2::ConfigLevel::XDG,
        git2::ConfigLevel::ProgramData,
    ] {
        unsafe {
            git2::opts::set_search_path(level, "").map_err(ge)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
