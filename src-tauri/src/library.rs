//! Native selected-root capabilities. No caller-supplied path is accepted by this service.
use crate::{files::FileError, private_store, scoped_file::ScopedFile};
use base64::{Engine, engine::general_purpose::STANDARD};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    ffi::{CStr, CString, OsString},
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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;
#[cfg(target_os = "linux")]
#[path = "linux_trash.rs"]
mod linux_trash;
const MAX_ENTRIES: usize = 100_000;
const PAGE: usize = 100;
fn err(code: &'static str, text: &str) -> FileError {
    FileError::new(code, text)
}
fn stale() -> FileError {
    err(
        "stale",
        "The folder or entry changed. Refresh it and prepare the operation again.",
    )
}
fn c(name: &std::ffi::OsStr) -> Result<CString, FileError> {
    CString::new(name.as_bytes()).map_err(|_| stale())
}
fn uid() -> String {
    Uuid::new_v4().to_string()
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteCapture {
    pub item: Item,
    pub path: String,
    pub text: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteCatalogue {
    pub root: Folder,
    pub active: String,
    pub notes: Vec<NoteCapture>,
    pub complete: bool,
    pub skipped: usize,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp(u64, u64, u64, i64, i64, i64, i64);
fn stamp(m: &fs::Metadata) -> Stamp {
    Stamp(
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
fn open_dir_at(parent: &File, name: &std::ffi::OsStr) -> Result<File, FileError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c(name)?.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
struct Stream(*mut libc::DIR);
// Sole owner, serialized by the library mutex; never used concurrently.
unsafe impl Send for Stream {}
impl Stream {
    fn new(dir: &File) -> Result<Self, FileError> {
        let file = open_dir_at(dir, std::ffi::OsStr::new("."))?;
        let fd = std::os::fd::IntoRawFd::into_raw_fd(file);
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(stream))
    }
    fn next(&mut self) -> Result<Option<OsString>, FileError> {
        loop {
            #[cfg(target_os = "macos")]
            let errno = unsafe { libc::__error() };
            #[cfg(target_os = "linux")]
            let errno = unsafe { libc::__errno_location() };
            unsafe {
                *errno = 0;
            }
            let entry = unsafe { libc::readdir(self.0) };
            if entry.is_null() {
                return if unsafe { *errno } == 0 {
                    Ok(None)
                } else {
                    Err(std::io::Error::last_os_error().into())
                };
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                return Ok(Some(OsString::from_vec(name.to_vec())));
            }
        }
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
struct Root {
    file: File,
    path: PathBuf,
    selected: PathBuf,
    revision: Arc<AtomicU64>,
    _watcher: Option<RecommendedWatcher>,
    ignore: ignore::gitignore::Gitignore,
}
#[derive(Clone)]
struct Dir {
    root: String,
    parts: Vec<OsString>,
    identity: (u64, u64),
}
#[derive(Clone)]
struct Entry {
    dir: String,
    name: OsString,
    folded_name: Box<str>,
    kind: String,
    stamp: Stamp,
}
impl Entry {
    fn new(dir: String, name: OsString, kind: &str, stamp: Stamp) -> Self {
        let folded_name = name.to_string_lossy().to_lowercase().into_boxed_str();
        Self {
            dir,
            name,
            folded_name,
            kind: kind.into(),
            stamp,
        }
    }
}
struct Listing {
    stream: Stream,
    entries: Vec<String>,
    complete: bool,
    scanned: usize,
    revision: u64,
    stamp: Stamp,
    ignored: bool,
    _watcher: Option<RecommendedWatcher>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub bytes: u64,
    pub modified: i64,
    pub location: Option<String>,
    pub directory: Option<String>,
    pub parent: Folder,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub root: String,
    pub label: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Item>,
    pub total: usize,
    pub scanned: usize,
    pub complete: bool,
    pub revision: u64,
}
#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Record {
    id: String,
    path: String,
    name: String,
    pinned: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Display {
    pub offset: usize,
    pub sort: String,
    pub hidden: bool,
    pub ignored: bool,
    pub filter: String,
}
impl Default for Display {
    fn default() -> Self {
        Self {
            offset: 0,
            sort: "name".into(),
            hidden: false,
            ignored: false,
            filter: "all".into(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Data {
    schema: u32,
    roots: Vec<Record>,
    recent: Vec<Record>,
    sidebar: bool,
    view: String,
    tabs: Vec<String>,
    active: usize,
    last_operation: Vec<Outcome>,
    location: String,
    display: Display,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            schema: 1,
            roots: vec![],
            recent: vec![],
            sidebar: false,
            view: "list".into(),
            tabs: vec![],
            active: 0,
            last_operation: vec![],
            location: String::new(),
            display: Display::default(),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    pub id: String,
    pub name: String,
    pub context: String,
    pub pinned: bool,
    pub authorized: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub roots: Vec<Folder>,
    pub root_hints: Vec<Hint>,
    pub recent: Vec<Hint>,
    pub can_clear_recent: bool,
    pub sidebar: bool,
    pub view: String,
    pub warning: Option<String>,
    pub session: Vec<Hint>,
    pub active: usize,
    pub last_operation: Vec<Outcome>,
    pub display: Display,
    pub job: Option<String>,
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub running: bool,
    pub scanned: usize,
    pub skipped: usize,
    pub bytes: u64,
    pub truncated: bool,
    pub canceled: bool,
    pub results: Vec<Hit>,
    pub outcomes: Vec<Outcome>,
    pub selection: Vec<Item>,
    pub error: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub item: Item,
    pub context: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub kind: String,
    pub entries: Vec<String>,
    pub destination: Option<String>,
    pub name: Option<String>,
}
#[derive(Clone)]
struct Plan {
    request: Request,
    entries: Vec<(String, Entry)>,
    destination: Option<Dir>,
    at: Instant,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: String,
    pub kind: String,
    pub sources: Vec<String>,
    pub destination: String,
    pub warning: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Outcome {
    pub name: String,
    pub status: String,
    pub message: String,
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropOffer {
    pub id: String,
    pub name: String,
    pub kind: String,
}
pub struct Library {
    roots: HashMap<String, Root>,
    dirs: HashMap<String, Dir>,
    entries: HashMap<String, Entry>,
    lookup: HashMap<(String, OsString), String>,
    lists: HashMap<String, Listing>,
    plans: HashMap<String, Plan>,
    job: Option<Job>,
    store: PathBuf,
    protected: Vec<PathBuf>,
    data: Data,
    previous: Vec<String>,
    previous_active: usize,
    warning: Option<String>,
    blocked: bool,
    drops: HashMap<String, (PathBuf, Instant)>,
}
impl Library {
    pub fn new(store: PathBuf, protected: Vec<PathBuf>) -> Self {
        let store = fs::canonicalize(&store).unwrap_or(store);
        let protected = protected
            .into_iter()
            .map(|p| fs::canonicalize(&p).unwrap_or(p))
            .collect::<Vec<_>>();
        let mut warning = None;
        let mut blocked = false;
        let decode = |bytes: Vec<u8>| -> Option<Data> {
            serde_json::from_slice::<Data>(&bytes).ok().filter(|d| {
                d.schema == 1
                    && d.roots.len() <= 32
                    && d.recent.len() <= 200
                    && d.tabs.len() <= crate::limits::MAX_TABS
                    && d.last_operation.len() <= 128
                    && d.roots
                        .iter()
                        .chain(d.recent.iter())
                        .all(|r| r.path.len() <= 32768 && r.name.len() <= 1024)
                    && d.tabs.iter().all(|p| p.len() <= 32768)
                    && d.location.len() <= 32768
            })
        };
        let primary = private_store::read(&store.join("library.json"), 2 * 1024 * 1024);
        let data = match primary {
            Ok(bytes) => match decode(bytes) {
                Some(d) => d,
                None => {
                    blocked = true;
                    warning=Some("Library records are unsupported or damaged; originals preserved. New records are session-only.".into());
                    private_store::read(&store.join("library.bak"), 2 * 1024 * 1024)
                        .ok()
                        .and_then(decode)
                        .unwrap_or_default()
                }
            },
            Err(error) => {
                let fallback = private_store::read(&store.join("library.bak"), 2 * 1024 * 1024)
                    .ok()
                    .and_then(decode);
                if fallback.is_some() {
                    warning = Some("Library records restored from the last-good backup.".into());
                }
                if store.join("library.json").exists() {
                    blocked = true;
                    warning = Some(error.message);
                }
                fallback.unwrap_or_default()
            }
        };
        Self {
            roots: HashMap::new(),
            dirs: HashMap::new(),
            entries: HashMap::new(),
            lookup: HashMap::new(),
            lists: HashMap::new(),
            plans: HashMap::new(),
            job: None,
            store,
            protected,
            previous: data.tabs.clone(),
            previous_active: data.active,
            data,
            warning,
            blocked,
            drops: HashMap::new(),
        }
    }
    fn persist(&mut self) -> Result<(), FileError> {
        if self.store.as_os_str().is_empty() {
            return Err(err("storage", "Private storage is unavailable."));
        }
        if self.blocked {
            return Err(err(
                "library-storage",
                "Library records are read-only; originals were preserved.",
            ));
        }
        let bytes = serde_json::to_vec(&self.data).map_err(|_| stale())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(err("capacity", "Library records exceed their budget."));
        }
        if private_store::usage(&self.store)?.saturating_add(bytes.len() as u64 * 2)
            > private_store::STORE_QUOTA
        {
            return Err(err(
                "quota",
                "Private storage is full. Library changes remain session-only; recovery was preserved.",
            ));
        }
        if let Ok(previous) = private_store::read(&self.store.join("library.json"), 2 * 1024 * 1024)
        {
            private_store::atomic(&self.store.join("library.bak"), &previous)?;
        }
        private_store::atomic(&self.store.join("library.json"), &bytes)
    }
    fn persist_hint(&mut self) {
        if let Err(e) = self.persist() {
            self.warning = Some(e.message);
        }
    }
    fn allowed(&self, path: &Path) -> bool {
        !self.protected.iter().any(|p| path.starts_with(p))
            && !path
                .components()
                .any(|p| p.as_os_str().as_bytes().eq_ignore_ascii_case(b".git"))
    }
    pub fn offer(&mut self, paths: &[PathBuf]) -> Vec<DropOffer> {
        self.drops
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(120));
        let mut offers = vec![];
        for path in paths.iter().take(64) {
            if self.drops.len() >= 64 {
                break;
            }
            let Ok(canonical) = fs::canonicalize(path) else {
                continue;
            };
            if !self.allowed(&canonical) {
                continue;
            }
            let Ok(meta) = fs::metadata(&canonical) else {
                continue;
            };
            if !meta.is_file() && !meta.is_dir() {
                continue;
            }
            let id = uid();
            offers.push(DropOffer {
                id: id.clone(),
                name: canonical
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                kind: if meta.is_dir() {
                    "folder".into()
                } else {
                    "file".into()
                },
            });
            self.drops.insert(id, (path.clone(), Instant::now()));
        }
        offers
    }
    pub fn accept_drop(&mut self, id: &str) -> Result<PathBuf, FileError> {
        let (path, at) = self
            .drops
            .remove(id)
            .ok_or_else(|| err("unauthorized", "Unknown native drop offer."))?;
        if at.elapsed() > Duration::from_secs(120) {
            return Err(err("expired", "The drop expired; drop the item again."));
        }
        let canonical = fs::canonicalize(&path)?;
        if !self.allowed(&canonical) {
            return Err(stale());
        }
        Ok(path)
    }
    pub fn select(&mut self, selected: &Path) -> Result<Folder, FileError> {
        if self.store.as_os_str().is_empty() {
            return Err(err(
                "storage",
                "Library unavailable while private storage is unavailable.",
            ));
        }
        let path = fs::canonicalize(selected)?;
        if !self.allowed(&path) {
            return Err(err(
                "protected",
                "Application resources and private data cannot be library roots.",
            ));
        }
        if let Some((id, _)) = self.roots.iter().find(|(_, r)| r.path == path) {
            return self.folder(id);
        }
        if self.roots.len() >= 8 {
            return Err(err(
                "capacity",
                "Remove a root before adding another (limit 8).",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let metadata = file.metadata()?;
        let revision = Arc::new(AtomicU64::new(1));
        let event_revision = revision.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if !matches!(event,Ok(ref e) if matches!(e.kind,notify::EventKind::Access(_))) {
                event_revision.fetch_add(1, Ordering::Relaxed);
            }
        })
        .ok()
        .and_then(|mut w| {
            w.watch(&path, RecursiveMode::NonRecursive).ok()?;
            Some(w)
        });
        let ignore = load_ignore(&file, &path)?;
        let id = uid();
        self.dirs.insert(
            id.clone(),
            Dir {
                root: id.clone(),
                parts: vec![],
                identity: (metadata.dev(), metadata.ino()),
            },
        );
        self.roots.insert(
            id.clone(),
            Root {
                file,
                path: path.clone(),
                selected: selected.into(),
                revision,
                _watcher: watcher,
                ignore,
            },
        );
        let encoded = STANDARD.encode(path.as_os_str().as_bytes());
        self.data.roots.retain(|r| r.path != encoded);
        self.data.roots.insert(
            0,
            Record {
                id: uid(),
                path: encoded,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                pinned: false,
            },
        );
        self.data.roots.truncate(32);
        self.persist_hint();
        if let Some(previous) = decode_path(&self.data.location)
            && previous.starts_with(&path)
            && previous != path
            && let Ok(entry) = self.resolve_path(&previous)
            && let Some(dir) = self.item(&entry)?.directory
        {
            return self.folder(&dir);
        }
        self.folder(&id)
    }
    fn folder(&self, id: &str) -> Result<Folder, FileError> {
        let d = self.dirs.get(id).ok_or_else(stale)?;
        let root = self.roots.get(&d.root).ok_or_else(stale)?;
        Ok(Folder {
            id: id.into(),
            root: d.root.clone(),
            label: d
                .parts
                .iter()
                .fold(root.path.clone(), |path, part| path.join(part))
                .to_string_lossy()
                .into(),
            name: if d.parts.is_empty() {
                root.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            } else {
                d.parts.last().unwrap().to_string_lossy().into()
            },
        })
    }
    fn dir(&self, id: &str) -> Result<(File, PathBuf), FileError> {
        let d = self
            .dirs
            .get(id)
            .ok_or_else(|| err("unauthorized", "Unknown folder handle."))?;
        let r = self.roots.get(&d.root).ok_or_else(stale)?;
        if fs::canonicalize(&r.selected)? != r.path {
            return Err(stale());
        }
        let m = fs::metadata(&r.path)?;
        let pin = r.file.metadata()?;
        if (m.dev(), m.ino()) != (pin.dev(), pin.ino()) {
            return Err(stale());
        }
        let mut file = r.file.try_clone()?;
        let mut path = r.path.clone();
        for part in &d.parts {
            path.push(part);
            if !self.allowed(&path) {
                return Err(err("protected", "This directory is excluded."));
            }
            file = open_dir_at(&file, part)?;
        }
        let m = file.metadata()?;
        if (m.dev(), m.ino()) != d.identity {
            return Err(stale());
        }
        Ok((file, path))
    }
    fn entry(&self, id: &str) -> Result<(Entry, File, PathBuf), FileError> {
        let e = self
            .entries
            .get(id)
            .ok_or_else(|| err("unauthorized", "Unknown entry handle."))?
            .clone();
        let (parent, path) = self.dir(&e.dir)?;
        if !self.allowed(&path.join(&e.name)) {
            return Err(err("protected", "This entry is excluded."));
        }
        let stat = stat_at(&parent, &e.name)?;
        if stamp(&stat) != e.stamp {
            return Err(stale());
        }
        Ok((e, parent, path))
    }
    fn intern(&mut self, entry: Entry) -> Result<String, FileError> {
        let key = (entry.dir.clone(), entry.name.clone());
        if let Some(id) = self.lookup.get(&key) {
            if self
                .entries
                .get(id)
                .is_some_and(|e| e.stamp == entry.stamp && e.kind == entry.kind)
            {
                return Ok(id.clone());
            }
            self.entries.remove(id);
            self.lists.remove(&entry.dir);
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(err(
                "capacity",
                "Library entry budget reached. Remove a root to release its handles.",
            ));
        }
        let id = uid();
        self.lookup.insert(key, id.clone());
        self.entries.insert(id.clone(), entry);
        Ok(id)
    }
    fn item(&mut self, id: &str) -> Result<Item, FileError> {
        let e = self.entries.get(id).ok_or_else(stale)?.clone();
        let parent = self.folder(&e.dir)?;
        let d = self.dirs.get(&e.dir).ok_or_else(stale)?.clone();
        let location = (e.kind == "file").then(|| {
            let path = d
                .parts
                .iter()
                .fold(self.roots[&d.root].path.clone(), |path, part| {
                    path.join(part)
                });
            crate::scoped_file::path_key(&path.join(&e.name))
        });
        let directory = if e.kind == "folder" {
            let mut parts = d.parts;
            parts.push(e.name.clone());
            let existing = self
                .dirs
                .iter()
                .find(|(_, dir)| {
                    dir.root == d.root
                        && dir.parts == parts
                        && dir.identity == (e.stamp.0, e.stamp.1)
                })
                .map(|(id, _)| id.clone());
            Some(if let Some(id) = existing {
                id
            } else {
                if self.dirs.len() >= 10_000 {
                    return Err(err(
                        "capacity",
                        "Directory handle limit reached. Remove and reselect this root.",
                    ));
                }
                let id = uid();
                self.dirs.insert(
                    id.clone(),
                    Dir {
                        root: d.root,
                        parts,
                        identity: (e.stamp.0, e.stamp.1),
                    },
                );
                id
            })
        } else {
            None
        };
        Ok(Item {
            id: id.into(),
            name: e.name.to_string_lossy().into(),
            kind: e.kind,
            bytes: e.stamp.2,
            modified: e.stamp.3,
            location,
            directory,
            parent,
        })
    }
    #[allow(clippy::too_many_arguments)] // Independent paging/filter controls.
    pub fn page(
        &mut self,
        id: &str,
        offset: usize,
        sort: &str,
        hidden: bool,
        filter: &str,
        refresh: bool,
        ignored: bool,
    ) -> Result<Page, FileError> {
        self.page_matching(id, offset, sort, hidden, filter, refresh, ignored, "")
    }

    #[allow(clippy::too_many_arguments)] // Reuses the bounded folder listing with a name filter.
    pub fn page_matching(
        &mut self,
        id: &str,
        offset: usize,
        sort: &str,
        hidden: bool,
        filter: &str,
        refresh: bool,
        ignored: bool,
        query: &str,
    ) -> Result<Page, FileError> {
        if query.len() > 512 {
            return Err(err("query", "Folder search is limited to 512 bytes."));
        }
        let query = query.trim().to_lowercase();
        let (dir, path) = self.dir(id)?;
        if refresh {
            let root = &self.dirs[id].root;
            let r = self.roots.get_mut(root).ok_or_else(stale)?;
            r.ignore = load_ignore(&r.file, &r.path)?;
        }
        let d = &self.dirs[id];
        let revision = self.roots[&d.root].revision.load(Ordering::Relaxed);
        let now = stamp(&dir.metadata()?);
        if refresh
            || self
                .lists
                .get(id)
                .is_some_and(|l| l.revision != revision || l.stamp != now || l.ignored != ignored)
        {
            self.lists.remove(id);
        }
        if !self.lists.contains_key(id) {
            if self.lists.len() >= 32
                && let Some(old) = self.lists.keys().next().cloned()
            {
                self.lists.remove(&old);
            }
            let event_revision = self.roots[&self.dirs[id].root].revision.clone();
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if !matches!(event,Ok(ref e) if matches!(e.kind,notify::EventKind::Access(_))) {
                        event_revision.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .ok()
                .and_then(|mut w| {
                    w.watch(&path, RecursiveMode::NonRecursive).ok()?;
                    Some(w)
                });
            self.lists.insert(
                id.into(),
                Listing {
                    stream: Stream::new(&dir)?,
                    entries: vec![],
                    complete: false,
                    scanned: 0,
                    revision,
                    stamp: now,
                    ignored,
                    _watcher: watcher,
                },
            );
        }
        let mut list = self.lists.remove(id).unwrap();
        let was_complete = list.complete;
        for _ in 0..512 {
            if list.complete {
                break;
            }
            let Some(name) = list.stream.next()? else {
                list.complete = true;
                break;
            };
            list.scanned += 1;
            if list.scanned > 50_000 {
                self.lists.insert(id.into(), list);
                return Err(err(
                    "capacity",
                    "Directory scan budget reached (50,000 names, including excluded entries). Results are partial.",
                ));
            }
            if !self.allowed(&path.join(&name)) {
                continue;
            }
            let Ok(metadata) = stat_at(&dir, &name) else {
                continue;
            };
            if !ignored
                && self.roots[&self.dirs[id].root]
                    .ignore
                    .matched_path_or_any_parents(path.join(&name), metadata.is_dir())
                    .is_ignore()
            {
                continue;
            }
            let kind = if metadata.file_type().is_symlink() {
                "link"
            } else if metadata.is_dir() {
                "folder"
            } else if metadata.is_file() {
                "file"
            } else {
                "special"
            };
            if list.entries.len() >= 50_000 {
                self.lists.insert(id.into(), list);
                return Err(err(
                    "capacity",
                    "Library handle budget reached (100,000 entries or 50,000 per folder). Remove and reselect roots to release it.",
                ));
            }
            let key = self.intern(Entry::new(id.into(), name, kind, stamp(&metadata)))?;
            list.entries.push(key);
        }
        if list.complete && !was_complete {
            let live = list
                .entries
                .iter()
                .collect::<std::collections::HashSet<_>>();
            let removed = self
                .lookup
                .iter()
                .filter(|((dir, _), key)| dir == id && !live.contains(key))
                .map(|(key, id)| (key.clone(), id.clone()))
                .collect::<Vec<_>>();
            for (key, id) in removed {
                self.lookup.remove(&key);
                self.entries.remove(&id);
            }
        }
        let mut ids = list
            .entries
            .iter()
            .map(|key| (key, &self.entries[key]))
            .filter(|(_, e)| {
                (hidden || !e.name.as_bytes().starts_with(b"."))
                    && (query.is_empty() || e.folded_name.contains(&query))
                    && match filter {
                        "folders" => e.kind == "folder",
                        "notes" => e.kind == "file" && is_text(&e.name),
                        "all" => true,
                        _ if e.kind == "folder" => true,
                        _ => match filter {
                            "markdown" => e.kind == "file" && is_markdown(&e.name),
                            "text" => is_text(&e.name),
                            "images" => is_image(&e.name),
                            _ => false,
                        },
                    }
            })
            .collect::<Vec<_>>();
        ids.sort_by(|(_, a), (_, b)| {
            (a.kind != "folder")
                .cmp(&(b.kind != "folder"))
                .then_with(|| match sort {
                    "size" => b.stamp.2.cmp(&a.stamp.2),
                    "modified" => b.stamp.3.cmp(&a.stamp.3),
                    "type" => Path::new(&a.name)
                        .extension()
                        .cmp(&Path::new(&b.name).extension()),
                    _ => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.folded_name.cmp(&b.folded_name))
        });
        let total = ids.len();
        let scanned = list.scanned;
        let complete = list.complete;
        let page_ids = ids
            .into_iter()
            .skip(offset.min(total))
            .take(PAGE)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        self.lists.insert(id.into(), list);
        let items = page_ids
            .iter()
            .map(|id| self.item(id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Page {
            items,
            total,
            scanned,
            complete,
            revision,
        })
    }
    pub fn scoped(&self, id: &str) -> Result<ScopedFile, FileError> {
        let (e, parent, path) = self.entry(id)?;
        if e.kind != "file" {
            return Err(err(
                "type",
                "Only regular files can be opened; links are not followed.",
            ));
        }
        ScopedFile::from_parent(parent, path, e.name)
    }
    pub fn remember(&mut self, id: &str) -> Result<(), FileError> {
        let (e, _, path) = self.entry(id)?;
        self.remember_path(&path.join(e.name));
        Ok(())
    }
    pub fn state(&self) -> State {
        State {
            roots: self
                .roots
                .keys()
                .filter_map(|id| self.folder(id).ok())
                .collect(),
            root_hints: self.data.roots.iter().map(|r| self.hint(r)).collect(),
            recent: self
                .data
                .recent
                .iter()
                .filter(|r| self.recent_available(r))
                .map(|r| self.hint(r))
                .collect(),
            can_clear_recent: self.data.recent.iter().any(|r| !r.pinned),
            sidebar: self.data.sidebar,
            view: self.data.view.clone(),
            warning: self.warning.clone(),
            session: self
                .previous
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    decode_path(p).map(|path| {
                        self.hint(&Record {
                            id: format!("session-{i}"),
                            path: p.clone(),
                            name: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into(),
                            pinned: false,
                        })
                    })
                })
                .collect(),
            active: self.previous_active,
            last_operation: self.data.last_operation.clone(),
            display: self.data.display.clone(),
            job: self.job.as_ref().and_then(|j| {
                j.progress
                    .lock()
                    .ok()
                    .filter(|p| p.running)
                    .map(|p| p.id.clone())
            }),
        }
    }
    fn recent_available(&self, record: &Record) -> bool {
        let Some(path) = decode_path(&record.path) else {
            return false;
        };
        self.root_for(&path).is_some_and(|(id, _)| {
            self.allowed(&path)
                && self.dir(id).is_ok()
                && fs::canonicalize(&path).is_ok_and(|resolved| resolved == path)
                && fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file())
        })
    }
    fn hint(&self, r: &Record) -> Hint {
        let path = decode_path(&r.path);
        Hint {
            id: r.id.clone(),
            name: r.name.clone(),
            context: path
                .as_deref()
                .and_then(Path::parent)
                .map(display_context)
                .unwrap_or_default(),
            pinned: r.pinned,
            authorized: path
                .is_some_and(|p| self.roots.values().any(|root| p.starts_with(&root.path))),
        }
    }
    pub fn location(&mut self, id: &str, display: Display) -> Result<(), FileError> {
        if display.offset > 50_000
            || !["name", "modified", "size", "type"].contains(&display.sort.as_str())
            || !["all", "text", "images"].contains(&display.filter.as_str())
        {
            return Err(stale());
        }
        let (_, path) = self.dir(id)?;
        self.data.location = STANDARD.encode(path.as_os_str().as_bytes());
        self.data.display = display;
        self.persist()
    }
    pub fn configure(&mut self, sidebar: bool, view: &str) -> Result<(), FileError> {
        if !["list", "compact", "grid"].contains(&view) {
            return Err(stale());
        }
        self.data.sidebar = sidebar;
        self.data.view = view.into();
        self.persist()
    }
    pub fn clear_recent(&mut self) -> Result<(), FileError> {
        let previous = std::mem::take(&mut self.data.recent);
        self.data.recent = previous.iter().filter(|r| r.pinned).cloned().collect();
        if let Err(error) = self.persist() {
            self.data.recent = previous;
            return Err(error);
        }
        Ok(())
    }
    fn trim_recent(&mut self) {
        let mut unpinned = 0;
        self.data.recent.retain(|r| {
            if r.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= 100
        });
    }
    pub fn pin(&mut self, id: &str) -> Result<(), FileError> {
        let count = self.data.recent.iter().filter(|r| r.pinned).count();
        let r = self
            .data
            .recent
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(stale)?;
        if !r.pinned && count >= 100 {
            return Err(err(
                "capacity",
                "Bookmark limit reached (100). Unpin a file first.",
            ));
        }
        r.pinned = !r.pinned;
        self.trim_recent();
        self.persist()
    }
    pub fn session(&mut self, paths: Vec<PathBuf>, active: usize) -> Result<(), FileError> {
        if paths.len() > crate::limits::MAX_TABS {
            return Err(stale());
        }
        self.data.tabs = paths
            .iter()
            .map(|p| STANDARD.encode(p.as_os_str().as_bytes()))
            .collect();
        self.data.active = active.min(paths.len().saturating_sub(1));
        self.persist()
    }
    pub fn resolve_hint(&mut self, id: &str) -> Result<String, FileError> {
        let path = if let Some(index) = id
            .strip_prefix("session-")
            .and_then(|i| i.parse::<usize>().ok())
        {
            self.previous.get(index).cloned().ok_or_else(stale)?
        } else {
            self.data
                .recent
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.path.clone())
                .ok_or_else(stale)?
        };
        self.resolve_path(&decode_path(&path).ok_or_else(stale)?)
    }
    pub fn note_catalogue(&mut self, base: &Path) -> Result<NoteCatalogue, FileError> {
        let (root_id, root_path) = self
            .roots
            .iter()
            .filter(|(_, root)| base.starts_with(&root.path))
            .max_by_key(|(_, root)| root.path.components().count())
            .map(|(id, root)| (id.clone(), root.path.clone()))
            .ok_or_else(|| {
                err(
                    "unauthorized",
                    "Open the containing folder in Library to explore note links.",
                )
            })?;
        let active = base
            .strip_prefix(&root_path)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| err("encoding", "Note links require UTF-8 filenames."))?
            .to_owned();
        let root = self.folder(&root_id)?;
        let mut result = NoteCatalogue {
            root,
            active,
            notes: vec![],
            complete: true,
            skipped: 0,
        };
        let mut queue = VecDeque::from([root_id]);
        let mut seen = std::collections::HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut steps = 0;
        while let Some(dir) = queue.pop_front() {
            let mut offset = 0;
            let mut refresh = true;
            loop {
                steps += 1;
                if steps > 1000 || Instant::now() > deadline || result.notes.len() >= 200 {
                    result.complete = false;
                    return Ok(result);
                }
                let page = self.page(&dir, offset, "name", false, "all", refresh, false)?;
                refresh = false;
                for item in page.items {
                    if !seen.insert(item.id.clone()) {
                        continue;
                    }
                    if let Some(child) = &item.directory {
                        if self.dirs[child].parts.len() < 64 {
                            queue.push_back(child.clone());
                        } else {
                            result.complete = false;
                        }
                        continue;
                    }
                    if item.kind != "file"
                        || !["md", "markdown"].contains(
                            &Path::new(&item.name)
                                .extension()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase()
                                .as_str(),
                        )
                    {
                        continue;
                    }
                    if result.notes.len() >= 200 {
                        result.complete = false;
                        return Ok(result);
                    }
                    let (_, parent) = self.dir(&item.parent.id)?;
                    let relative = parent
                        .join(&item.name)
                        .strip_prefix(&root_path)
                        .ok()
                        .and_then(Path::to_str)
                        .map(str::to_owned);
                    let Some(path) = relative else {
                        result.skipped += 1;
                        continue;
                    };
                    let text = if item.bytes <= 65536 {
                        self.scoped(&item.id)
                            .and_then(|file| file.snapshot_bounded(65536))
                            .and_then(|(mut file, _, _)| {
                                let mut text = String::new();
                                file.read_to_string(&mut text)?;
                                if text.contains('\0') {
                                    return Err(err("encoding", "Not a UTF-8 Markdown note."));
                                }
                                Ok(text)
                            })
                            .ok()
                    } else {
                        None
                    };
                    if text.is_none() {
                        result.skipped += 1;
                    }
                    result.notes.push(NoteCapture { item, path, text });
                }
                offset += PAGE;
                if offset >= page.total {
                    if page.complete {
                        break;
                    }
                    offset = 0;
                }
            }
        }
        Ok(result)
    }
    pub fn resolve_link(
        &mut self,
        base: &Path,
        link: &str,
    ) -> Result<Option<ScopedFile>, FileError> {
        let raw = link.split('#').next().unwrap_or("");
        if raw.contains(':') || raw.contains('?') || raw.starts_with('/') || raw.contains('\\') {
            return Ok(None);
        }
        let bytes = percent_decode(raw)?;
        let relative = PathBuf::from(OsString::from_vec(bytes));
        let mut path = base.parent().ok_or_else(stale)?.to_owned();
        for p in relative.components() {
            match p {
                std::path::Component::Normal(n) => path.push(n),
                std::path::Component::ParentDir => {
                    path.pop();
                }
                std::path::Component::CurDir => {}
                _ => return Ok(None),
            }
        }
        match self.resolve_path(&path) {
            Ok(id) => self.scoped(&id).map(Some),
            Err(e) if e.code == "unauthorized" => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn root_for(&self, path: &Path) -> Option<(&str, &Path)> {
        self.roots
            .iter()
            .filter(|(_, root)| path.starts_with(&root.path))
            .max_by_key(|(_, root)| root.path.components().count())
            .map(|(id, root)| (id.as_str(), root.path.as_path()))
    }
    pub(crate) fn resolve_path(&mut self, path: &Path) -> Result<String, FileError> {
        let (root_id, root_path) = self.root_for(path).ok_or_else(|| {
            err(
                "unauthorized",
                "Reselect a containing folder before reopening this location.",
            )
        })?;
        let parts = path
            .strip_prefix(root_path)
            .map_err(|_| stale())?
            .components()
            .map(|p| p.as_os_str().to_owned())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return Err(stale());
        }
        let mut dir = root_id.to_owned();
        for (index, part) in parts.iter().enumerate() {
            let (parent, path) = self.dir(&dir)?;
            if !self.allowed(&path.join(part)) {
                return Err(stale());
            }
            let m = stat_at(&parent, part)?;
            let kind = if m.is_dir() {
                "folder"
            } else if m.is_file() {
                "file"
            } else {
                "link"
            };
            let id = self.intern(Entry::new(dir.clone(), part.clone(), kind, stamp(&m)))?;
            if index + 1 == parts.len() {
                return Ok(id);
            }
            if kind != "folder" {
                return Err(stale());
            }
            dir = self.item(&id)?.directory.ok_or_else(stale)?;
        }
        Err(stale())
    }
    pub fn remove_root(&mut self, id: &str) -> Result<(), FileError> {
        self.roots.remove(id).ok_or_else(stale)?;
        let dirs = self
            .dirs
            .iter()
            .filter(|(_, d)| d.root == id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in dirs {
            self.dirs.remove(&id);
            self.lists.remove(&id);
            self.entries.retain(|_, e| e.dir != id);
        }
        self.lookup
            .retain(|(dir, _), _| self.dirs.contains_key(dir));
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn preview(&self, id: &str) -> Result<String, FileError> {
        let scoped = self.scoped(id)?;
        let mut file = scoped.open()?;
        let before = stamp(&file.metadata()?);
        let mut bytes = vec![];
        Read::by_ref(&mut file)
            .take(65536)
            .read_to_end(&mut bytes)?;
        if before != stamp(&file.metadata()?) {
            return Err(stale());
        }
        if bytes.contains(&0) {
            return Err(err("encoding", "Binary file cannot be previewed."));
        }
        while std::str::from_utf8(&bytes).is_err() && bytes.len() > 65532 {
            bytes.pop();
        }
        String::from_utf8(bytes).map_err(|_| err("encoding", "Preview requires UTF-8 text."))
    }
    pub fn start(
        shared: Arc<Mutex<Self>>,
        query: String,
        content: bool,
        hidden: bool,
        ignored: bool,
    ) -> Result<String, FileError> {
        if query.len() > 256 || (content && query.is_empty()) {
            return Err(err("query", "Use a search phrase of 1–256 bytes."));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let id = uid();
        let progress = Arc::new(Mutex::new(Progress {
            id: id.clone(),
            running: true,
            ..Progress::default()
        }));
        {
            let mut lib = shared.lock().map_err(|_| stale())?;
            if lib
                .job
                .as_ref()
                .is_some_and(|j| j.progress.lock().is_ok_and(|p| p.running))
            {
                return Err(err("busy", "Cancel the current library job first."));
            }
            lib.job = Some(Job {
                cancel: cancel.clone(),
                progress: progress.clone(),
            });
        }
        std::thread::spawn(move || {
            let result = (|| -> Result<(), FileError> {
                let roots = shared
                    .lock()
                    .map_err(|_| stale())?
                    .roots
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut queue = VecDeque::from(roots);
                let mut dirs = 0;
                let mut seen = std::collections::HashSet::new();
                let mut steps = 0usize;
                let deadline = Instant::now() + Duration::from_secs(120);
                while let Some(dir) = queue.pop_front() {
                    dirs += 1;
                    if dirs > 10_000 {
                        progress.lock().unwrap().truncated = true;
                        break;
                    }
                    let mut offset = 0;
                    loop {
                        steps += 1;
                        if steps > 5000 || Instant::now() > deadline {
                            progress.lock().unwrap().truncated = true;
                            return Ok(());
                        }
                        if cancel.load(Ordering::Relaxed) {
                            progress.lock().unwrap().canceled = true;
                            return Ok(());
                        }
                        let page = shared
                            .lock()
                            .map_err(|_| stale())?
                            .page(&dir, offset, "name", hidden, "all", false, ignored)?;
                        for item in page.items {
                            if !seen.insert(item.id.clone()) {
                                continue;
                            }
                            if cancel.load(Ordering::Relaxed) {
                                progress.lock().unwrap().canceled = true;
                                return Ok(());
                            }
                            let mut p = progress.lock().unwrap();
                            p.scanned += 1;
                            if p.scanned > MAX_ENTRIES {
                                p.truncated = true;
                                return Ok(());
                            }
                            drop(p);
                            if let Some(child) = &item.directory {
                                let depth = shared.lock().unwrap().dirs[child].parts.len();
                                if depth < 64 {
                                    queue.push_back(child.clone());
                                } else {
                                    progress.lock().unwrap().skipped += 1;
                                }
                                continue;
                            }
                            if item.kind != "file" {
                                progress.lock().unwrap().skipped += 1;
                                continue;
                            }
                            let context = if content {
                                if !is_text(std::ffi::OsStr::new(&item.name))
                                    || item.bytes > 8 * 1024 * 1024
                                {
                                    progress.lock().unwrap().skipped += 1;
                                    continue;
                                }
                                let mut p = progress.lock().unwrap();
                                if p.bytes + item.bytes > 256 * 1024 * 1024 {
                                    p.truncated = true;
                                    return Ok(());
                                }
                                p.bytes += item.bytes;
                                drop(p);
                                let scoped = shared.lock().unwrap().scoped(&item.id)?;
                                search_file(scoped, &query, &cancel)?
                            } else if fuzzy(&item.name, &query) {
                                Some(String::new())
                            } else {
                                None
                            };
                            if let Some(context) = context {
                                let mut p = progress.lock().unwrap();
                                if p.results.len() < 200 {
                                    p.results.push(Hit { item, context });
                                } else {
                                    p.truncated = true;
                                    return Ok(());
                                }
                            }
                        }
                        offset += PAGE;
                        if offset >= page.total {
                            if page.complete {
                                break;
                            }
                            offset = 0;
                        }
                    }
                }
                Ok(())
            })();
            let mut p = progress.lock().unwrap();
            p.running = false;
            if let Err(e) = result {
                p.error = Some(e.message);
            }
        });
        Ok(id)
    }
    pub fn progress(&self, id: &str, cancel: bool) -> Result<Progress, FileError> {
        let j = self.job.as_ref().ok_or_else(stale)?;
        let p = j.progress.lock().map_err(|_| stale())?;
        if p.id != id {
            return Err(stale());
        }
        if cancel {
            j.cancel.store(true, Ordering::Relaxed);
        }
        Ok(p.clone())
    }
    pub fn prepare(&mut self, request: Request) -> Result<Review, FileError> {
        if ![
            "createFile",
            "createFolder",
            "rename",
            "copy",
            "move",
            "trash",
        ]
        .contains(&request.kind.as_str())
            || request.entries.len() > 100
            || request
                .entries
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != request.entries.len()
        {
            return Err(err(
                "operation",
                "Invalid operation or selection (maximum 100 entries).",
            ));
        }
        if request.kind.starts_with("create") {
            if !request.entries.is_empty() {
                return Err(stale());
            }
            validate_name(request.name.as_deref().unwrap_or(""))?;
        } else if request.entries.is_empty() {
            return Err(stale());
        }
        if request.kind == "rename" {
            if request.entries.len() != 1 {
                return Err(stale());
            }
            validate_name(request.name.as_deref().unwrap_or(""))?;
        }
        let mut entries = vec![];
        for id in &request.entries {
            let (entry, _, parent) = self.entry(id)?;
            if self
                .protected
                .iter()
                .any(|p| p.starts_with(parent.join(&entry.name)))
            {
                return Err(err(
                    "protected",
                    "This selection contains application resources or private storage.",
                ));
            }
            entries.push((id.clone(), entry));
        }
        let destination = if request.kind == "trash" {
            None
        } else {
            let id = request.destination.as_deref().ok_or_else(stale)?;
            self.dir(id)?;
            Some(self.dirs[id].clone())
        };
        let mut dest_name = request
            .destination
            .as_deref()
            .map(|id| self.folder(id).map(|d| d.label))
            .transpose()?
            .unwrap_or_else(|| "System Trash".into());
        if let Some(name) = &request.name {
            dest_name.push('/');
            dest_name.push_str(name);
        }
        if self.plans.len() >= 16 {
            self.plans.clear();
        }
        let id = uid();
        self.plans.insert(
            id.clone(),
            Plan {
                request: request.clone(),
                entries: entries.clone(),
                destination,
                at: Instant::now(),
            },
        );
        Ok(Review{id,kind:request.kind,sources:entries.iter().map(|(_,e)|e.name.to_string_lossy().into()).collect(),destination:dest_name,warning:"No existing destination is overwritten. Source sessions must be closed. Links are not rewritten. Each item reports its own outcome; cancellation retains completed items. Directory copies are verified per file, not a simultaneous folder snapshot. Copies verify bytes and preserve permissions and timestamps; macOS also copies ACLs and extended attributes. New copies remain owned by the current user. Unsupported metadata can leave a partial destination for review. Symlinks and special files cannot be copied.".into()})
    }
    pub fn take_plan(&mut self, id: &str) -> Result<Mutation, FileError> {
        let plan = self.plans.remove(id).ok_or_else(stale)?;
        if plan.at.elapsed() > Duration::from_secs(120) {
            return Err(err("expired", "The preflight expired. Review it again."));
        }
        if self
            .job
            .as_ref()
            .is_some_and(|j| j.progress.lock().is_ok_and(|p| p.running))
        {
            return Err(err("busy", "Cancel the library job before changing files."));
        }
        let destination = if let Some(expected) = plan.destination {
            let id = plan.request.destination.as_deref().unwrap();
            let current = self.dirs.get(id).ok_or_else(stale)?;
            if current.root != expected.root
                || current.parts != expected.parts
                || current.identity != expected.identity
            {
                return Err(stale());
            }
            Some(self.dir(id)?)
        } else {
            None
        };
        let mut items = vec![];
        for (id, expected) in &plan.entries {
            let (e, parent, path) = self.entry(id)?;
            if e.stamp != expected.stamp {
                return Err(stale());
            }
            items.push((e, parent, path));
        }
        let names = if plan.entries.is_empty() {
            vec![
                plan.request
                    .name
                    .clone()
                    .unwrap_or_else(|| "New entry".into()),
            ]
        } else {
            plan.entries
                .iter()
                .map(|(_, e)| e.name.to_string_lossy().into_owned())
                .collect()
        };
        self.data.last_operation=names.into_iter().map(|name|Outcome{name,status:"pending".into(),message:format!("Prepared {}. If Marktub exited before reporting a result, inspect source and destination before retrying. No rollback is claimed.",plan.request.kind)}).collect();
        self.persist()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress {
            id: id.into(),
            running: true,
            ..Progress::default()
        }));
        self.job = Some(Job {
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        Ok(Mutation {
            request: plan.request,
            items,
            destination,
            cancel,
            progress,
            protected: self.protected.clone(),
        })
    }
    pub fn select_moved(&mut self, path: &Path) -> Result<(), FileError> {
        let id = self.resolve_path(path)?;
        let item = self.item(&id)?;
        if let Some(job) = &self.job {
            let mut progress = job.progress.lock().map_err(|_| stale())?;
            if progress.selection.len() < 100 {
                progress.selection.push(item);
            }
        }
        Ok(())
    }
    pub fn relocate(&mut self, old: &Path, new: &Path) -> Vec<(PathBuf, PathBuf)> {
        let mut pairs = vec![(old.into(), new.into())];
        for record in self
            .data
            .recent
            .iter_mut()
            .chain(self.data.roots.iter_mut())
        {
            if let Some(path) = decode_path(&record.path)
                && let Ok(suffix) = path.strip_prefix(old)
            {
                let dest = new.join(suffix);
                pairs.push((path, dest.clone()));
                record.path = STANDARD.encode(dest.as_os_str().as_bytes());
                record.name = dest
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
            }
        }
        for encoded in self.data.tabs.iter_mut().chain(self.previous.iter_mut()) {
            if let Some(path) = decode_path(encoded)
                && let Ok(suffix) = path.strip_prefix(old)
            {
                *encoded = STANDARD.encode(new.join(suffix).as_os_str().as_bytes());
            }
        }
        if let Some(path) = decode_path(&self.data.location)
            && let Ok(suffix) = path.strip_prefix(old)
        {
            self.data.location = STANDARD.encode(new.join(suffix).as_os_str().as_bytes());
        }
        for root in self.roots.values_mut() {
            if let Ok(suffix) = root.path.strip_prefix(old) {
                let dest = new.join(suffix);
                if let Ok(meta) = fs::metadata(&dest)
                    && root
                        .file
                        .metadata()
                        .is_ok_and(|pin| (pin.dev(), pin.ino()) == (meta.dev(), meta.ino()))
                {
                    root.path = dest.clone();
                    root.selected = dest;
                    root.revision.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        self.persist_hint();
        pairs
    }
    pub fn remember_path(&mut self, path: &Path) -> String {
        let encoded = STANDARD.encode(path.as_os_str().as_bytes());
        let previous = self.data.recent.iter().find(|r| r.path == encoded).cloned();
        self.data.recent.retain(|r| r.path != encoded);
        let id = previous.as_ref().map_or_else(uid, |r| r.id.clone());
        self.data.recent.insert(
            0,
            Record {
                id: id.clone(),
                path: encoded,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                pinned: previous.is_some_and(|r| r.pinned),
            },
        );
        self.trim_recent();
        self.persist_hint();
        id
    }
    pub fn warn(&mut self, text: String) {
        self.warning = Some(text.clone());
        if let Some(job) = &self.job
            && let Ok(mut p) = job.progress.lock()
        {
            if p.outcomes.iter().any(|o| o.status == "warning") {
                return;
            }
            p.outcomes.push(Outcome {
                name: "Saved library metadata".into(),
                status: "warning".into(),
                message: text,
            });
        }
    }
    pub fn workspace(&self, path: &Path) -> Option<PathBuf> {
        self.roots
            .iter()
            .filter(|(_, r)| path.starts_with(&r.path))
            .filter(|(id, _)| self.dir(id).is_ok())
            .max_by_key(|(_, r)| r.path.components().count())
            .map(|(_, r)| r.path.clone())
    }
    pub fn is_running(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| j.progress.lock().is_ok_and(|p| p.running))
    }
    pub fn finish_mutation(&mut self, failed: bool) {
        if let Some(job) = &self.job {
            let mut p = job.progress.lock().unwrap_or_else(|e| e.into_inner());
            if failed {
                p.error=Some("The operation was interrupted. Sources and any partial destinations must be reviewed.".into());
            }
            self.data.last_operation = p.outcomes.clone();
            if failed {
                self.data.last_operation.push(Outcome{name:"Interrupted file operation".into(),status:"uncertain".into(),message:"Inspect the prepared sources and destinations before retrying. Partial copies may remain; no rollback is claimed.".into()});
            }
            p.running = false;
        }
        self.persist_hint();
        self.invalidate();
    }
    pub fn invalidate(&mut self) {
        self.lists.clear();
        for r in self.roots.values() {
            r.revision.fetch_add(1, Ordering::Relaxed);
        }
    }
}
// Display metadata only; this string is never an authority or path argument.
fn display_context(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect();
    parts[parts.len().saturating_sub(2)..].join(" / ").chars()
        .filter(|c| !c.is_control() && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .take(120).collect()
}
fn decode_path(s: &str) -> Option<PathBuf> {
    STANDARD
        .decode(s)
        .ok()
        .map(|b| PathBuf::from(OsString::from_vec(b)))
}
fn percent_decode(s: &str) -> Result<Vec<u8>, FileError> {
    let mut out = vec![];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b
                .get(i + 1..i + 3)
                .and_then(|x| std::str::from_utf8(x).ok())
                .and_then(|x| u8::from_str_radix(x, 16).ok())
                .ok_or_else(stale)?;
            if h == 0 {
                return Err(stale());
            }
            out.push(h);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(out)
}
fn is_markdown(name: &std::ffi::OsStr) -> bool {
    Path::new(name).extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
    })
}
fn is_text(n: &std::ffi::OsStr) -> bool {
    matches!(
        Path::new(n)
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt")
    )
}
fn is_image(n: &std::ffi::OsStr) -> bool {
    matches!(
        Path::new(n)
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}
fn fuzzy(name: &str, query: &str) -> bool {
    let mut chars = query.chars().flat_map(char::to_lowercase);
    let mut next = chars.next();
    for ch in name.chars().flat_map(char::to_lowercase) {
        if Some(ch) == next {
            next = chars.next();
        }
    }
    next.is_none()
}
fn search_file(
    scoped: ScopedFile,
    query: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>, FileError> {
    let mut file = scoped.open()?;
    let before = stamp(&file.metadata()?);
    let mut tail = vec![];
    let needle = query.as_bytes();
    let mut found = None;
    let mut utf8 = vec![];
    let mut input = Read::by_ref(&mut file).take(before.2.saturating_add(1));
    let mut captured = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut buf = [0; 65536];
        let n = input.read(&mut buf)?;
        captured += n as u64;
        if captured > before.2 {
            return Err(stale());
        }
        if n == 0 {
            break;
        }
        utf8.extend_from_slice(&buf[..n]);
        match std::str::from_utf8(&utf8) {
            Ok(_) => utf8.clear(),
            Err(error) if error.error_len().is_none() => {
                utf8.drain(..error.valid_up_to());
            }
            Err(_) => return Ok(None),
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.contains(&0) {
            return Ok(None);
        }
        if found.is_none()
            && let Some(at) = tail.windows(needle.len()).position(|s| s == needle)
        {
            found = Some(
                String::from_utf8_lossy(
                    &tail[at.saturating_sub(60)..(at + needle.len() + 100).min(tail.len())],
                )
                .into(),
            );
        }
        let keep = needle.len() + 4;
        if tail.len() > keep {
            tail.drain(..tail.len() - keep);
        }
    }
    if !utf8.is_empty() {
        return Ok(None);
    }
    if stamp(&file.metadata()?) != before {
        return Err(stale());
    }
    Ok(found)
}
fn stat_at(parent: &File, name: &std::ffi::OsStr) -> Result<fs::Metadata, FileError> {
    // openat with O_SYMLINK on macOS, O_PATH on Linux, permits metadata without following links.
    #[cfg(target_os = "macos")]
    let flags = libc::O_SYMLINK | libc::O_EVTONLY | libc::O_CLOEXEC | libc::O_NONBLOCK;
    #[cfg(target_os = "linux")]
    let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(parent.as_raw_fd(), c(name)?.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    unsafe { File::from_raw_fd(fd) }
        .metadata()
        .map_err(Into::into)
}
fn validate_name(name: &str) -> Result<(), FileError> {
    if name.is_empty()
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.eq_ignore_ascii_case(".git")
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err(err(
            "name",
            "Use a single filename without separators, control characters, trailing spaces or dots (up to 240 bytes).",
        ));
    }
    Ok(())
}
fn exclusive_rename(
    from: &File,
    name: &std::ffi::OsStr,
    to: &File,
    new: &std::ffi::OsStr,
) -> Result<(), FileError> {
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            from.as_raw_fd(),
            c(name)?.as_ptr(),
            to.as_raw_fd(),
            c(new)?.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            c(name)?.as_ptr(),
            to.as_raw_fd(),
            c(new)?.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    private_store::sync(to).and_then(|_|private_store::sync(from)).map_err(|_|err("durability","Rename committed, but parent synchronization failed. Check both locations before retrying; no rollback is claimed."))?;
    Ok(())
}
pub struct Mutation {
    request: Request,
    items: Vec<(Entry, File, PathBuf)>,
    destination: Option<(File, PathBuf)>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    protected: Vec<PathBuf>,
}
impl Mutation {
    pub fn run(self) -> Vec<(PathBuf, PathBuf)> {
        let mut moved = vec![];
        let mut outcomes = vec![];
        let mut copy = CopyJob {
            cancel: &self.cancel,
            progress: &self.progress,
            protected: &self.protected,
            entries: 0,
            bytes: 0,
        };
        if self.request.kind.starts_with("create") {
            let result = (|| {
                let (dir, path) = self.destination.as_ref().ok_or_else(stale)?;
                let name = std::ffi::OsStr::new(self.request.name.as_deref().ok_or_else(stale)?);
                if self
                    .protected
                    .iter()
                    .any(|p| path.join(name).starts_with(p))
                {
                    return Err(stale());
                }
                validate_parent_path(dir, path)?;
                create(dir, name, self.request.kind == "createFolder", &self.cancel)
            })();
            outcomes.push(outcome(
                self.request.name.as_deref().unwrap_or("New entry"),
                result,
            ));
        }
        for (e, parent, path) in &self.items {
            if self.cancel.load(Ordering::Relaxed) {
                outcomes.push(Outcome {
                    name: e.name.to_string_lossy().into(),
                    status: "canceled".into(),
                    message: "Source retained; earlier completed items remain.".into(),
                });
                break;
            }
            let result = (|| -> Result<String, FileError> {
                validate_parent_path(parent, path)?;
                if self
                    .protected
                    .iter()
                    .any(|p| p.starts_with(path.join(&e.name)))
                {
                    return Err(err(
                        "protected",
                        "Selection contains protected application data.",
                    ));
                }
                if stamp(&stat_at(parent, &e.name)?) != e.stamp {
                    return Err(stale());
                }
                if self.request.kind == "trash" {
                    trash(parent, &e.name, &e.stamp)?;
                    return Ok("Moved to System Trash. Restore using the system file manager; no permanent deletion was attempted.".into());
                }
                let (dest, dest_path) = self.destination.as_ref().ok_or_else(stale)?;
                validate_parent_path(dest, dest_path)?;
                let name = if self.request.kind == "rename" {
                    std::ffi::OsStr::new(self.request.name.as_deref().unwrap())
                } else {
                    &e.name
                };
                if dest_path.starts_with(path.join(&e.name))
                    || self
                        .protected
                        .iter()
                        .any(|p| dest_path.join(name).starts_with(p))
                {
                    return Err(err(
                        "destination",
                        "Destination cannot be inside the source or protected data.",
                    ));
                }
                ensure_available(dest, name, &self.cancel)?;
                if self.request.kind == "copy" {
                    copy_tree(parent, &e.name, path, dest, name, dest_path, &mut copy)?;
                    return Ok("Copied and verified. Source retained.".into());
                }
                if parent.metadata()?.dev() == dest.metadata()?.dev() {
                    let result = exclusive_rename(parent, &e.name, dest, name);
                    if result.is_ok() || result.as_ref().is_err_and(|e| e.code == "durability") {
                        moved.push((path.join(&e.name), dest_path.join(name)));
                    }
                    result?;
                    return Ok("Moved. Open read snapshots remain available; reopen the moved file from its destination. Relative links were not rewritten.".into());
                }
                copy_tree(parent, &e.name, path, dest, name, dest_path, &mut copy)?;
                if self.cancel.load(Ordering::Relaxed)
                    || stamp(&stat_at(parent, &e.name)?) != e.stamp
                {
                    return Err(err(
                        "partial",
                        "Verified copy completed; source retained because it changed or cancellation arrived. Review both locations.",
                    ));
                }
                if e.kind == "folder" {
                    return Err(err(
                        "partial",
                        "Verified directory copy completed; source retained for review. Cross-volume directory removal is refused because child files may have changed.",
                    ));
                }
                match trash(parent, &e.name, &e.stamp) {
                    Ok(()) => {
                        moved.push((path.join(&e.name), dest_path.join(name)));
                        Ok("Verified copy completed; original retained in System Trash.".into())
                    }
                    Err(error) if error.code == "durability" => {
                        moved.push((path.join(&e.name), dest_path.join(name)));
                        Err(error)
                    }
                    Err(_) => Err(err(
                        "partial",
                        "Verified copy completed; source retained because Trash on its volume is unavailable. Review both locations.",
                    )),
                }
            })();
            let result=result.map_err(|mut error| { if ["copy","move"].contains(&self.request.kind.as_str()){error.message.push_str(" Source was retained unless explicitly reported otherwise. Review any partial destination before retrying.");} error });
            outcomes.push(outcome(&e.name.to_string_lossy(), result));
            self.progress.lock().unwrap().outcomes = outcomes.clone();
        }
        let mut p = self.progress.lock().unwrap();
        p.outcomes = outcomes;
        // The app finishes the job after grant/metadata relocation and the durable result log.
        p.canceled = self.cancel.load(Ordering::Relaxed);
        moved
    }
}
fn outcome(name: &str, result: Result<String, FileError>) -> Outcome {
    match result {
        Ok(message) => Outcome {
            name: name.into(),
            status: "completed".into(),
            message,
        },
        Err(e) => Outcome {
            name: name.into(),
            status: match e.code {
                "partial" => "partial",
                "durability" => "uncertain",
                "canceled" => "canceled",
                _ => "failed",
            }
            .into(),
            message: e.message,
        },
    }
}
fn ensure_available(
    dir: &File,
    name: &std::ffi::OsStr,
    cancel: &AtomicBool,
) -> Result<(), FileError> {
    let mut stream = Stream::new(dir)?;
    let folded = name.to_string_lossy().to_lowercase();
    let mut scanned = 0;
    while let Some(existing) = stream.next()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(err(
                "canceled",
                "Operation canceled before destination creation. Completed earlier items remain.",
            ));
        }
        scanned += 1;
        if scanned > 50_000 {
            return Err(err(
                "capacity",
                "Destination collision-scan budget reached; no new entry was created.",
            ));
        }
        if existing.to_string_lossy().to_lowercase() == folded {
            return Err(err(
                "collision",
                "A destination with this name already exists (including case variants).",
            ));
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(err(
            "canceled",
            "Operation canceled before destination creation.",
        ));
    }
    Ok(())
}
fn create(
    dir: &File,
    name: &std::ffi::OsStr,
    folder: bool,
    cancel: &AtomicBool,
) -> Result<String, FileError> {
    ensure_available(dir, name, cancel)?;
    if folder {
        if unsafe { libc::mkdirat(dir.as_raw_fd(), c(name)?.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    } else {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c(name)?.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        private_store::sync(&unsafe { File::from_raw_fd(fd) })?;
    }
    private_store::sync(dir)?;
    Ok("Created exclusively.".into())
}
struct CopyJob<'a> {
    cancel: &'a AtomicBool,
    progress: &'a Mutex<Progress>,
    protected: &'a [PathBuf],
    entries: usize,
    bytes: u64,
}
fn copy_tree(
    from: &File,
    name: &std::ffi::OsStr,
    from_path: &Path,
    to: &File,
    new: &std::ffi::OsStr,
    to_path: &Path,
    job: &mut CopyJob<'_>,
) -> Result<(), FileError> {
    if job.cancel.load(Ordering::Relaxed) {
        return Err(err(
            "canceled",
            "Copy canceled. Source retained; an incomplete destination may remain for review.",
        ));
    }
    validate_parent_path(from, from_path)?;
    validate_parent_path(to, to_path)?;
    job.entries += 1;
    job.progress.lock().map_err(|_| stale())?.scanned = job.entries;
    if job.entries > 10_000 {
        return Err(err(
            "capacity",
            "Copy entry budget exceeded. Source retained; review the partial destination.",
        ));
    }
    let m = stat_at(from, name)?;
    if job
        .protected
        .iter()
        .any(|p| from_path.join(name).starts_with(p))
        || name.as_bytes().eq_ignore_ascii_case(b".git")
    {
        return Err(err(
            "protected",
            "Copy contains excluded data; source retained.",
        ));
    }
    if m.is_dir() {
        if from_path.components().count() > 128 {
            return Err(err("capacity", "Copy depth limit reached."));
        }
        create(to, new, true, job.cancel)?;
        let src = open_dir_at(from, name)?;
        if stamp(&src.metadata()?) != stamp(&m) {
            return Err(stale());
        }
        let dst = open_dir_at(to, new)?;
        let mut stream = Stream::new(&src)?;
        while let Some(child) = stream.next()? {
            copy_tree(
                &src,
                &child,
                &from_path.join(name),
                &dst,
                &child,
                &to_path.join(new),
                job,
            )?;
        }
        if stamp(&stat_at(from, name)?) != stamp(&m) {
            return Err(stale());
        }
        copy_metadata(&src, &dst)?;
        private_store::sync(&dst)?;
        return Ok(());
    }
    if !m.is_file() {
        return Err(err(
            "type",
            "Copy refuses symlinks and special files. Source retained.",
        ));
    }
    job.bytes = job.bytes.saturating_add(m.len());
    if job.bytes > 1024 * 1024 * 1024 {
        return Err(err(
            "capacity",
            "Copy byte budget (1 GiB) exceeded. Source retained.",
        ));
    }
    let scoped = ScopedFile::from_parent(from.try_clone()?, from_path.into(), name.to_owned())?;
    let mut input = scoped.open()?;
    let before = stamp(&input.metadata()?);
    if before != stamp(&m) {
        return Err(stale());
    }
    let fd = unsafe {
        libc::openat(
            to.as_raw_fd(),
            c(new)?.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut output = unsafe { File::from_raw_fd(fd) };
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut bounded = Read::by_ref(&mut input).take(m.len().saturating_add(1));
    let mut captured = 0;
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            return Err(err(
                "canceled",
                "Copy canceled; source retained. Review the partial destination.",
            ));
        }
        let mut bytes = [0; 65536];
        let n = bounded.read(&mut bytes)?;
        captured += n as u64;
        if captured > m.len() {
            return Err(stale());
        }
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
        output.write_all(&bytes[..n])?;
        job.progress.lock().map_err(|_| stale())?.bytes += n as u64;
    }
    if before != stamp(&input.metadata()?) || before != stamp(&stat_at(from, name)?) {
        return Err(stale());
    }
    private_store::sync(&output)?;
    let (actual, _) = crate::scoped_file::fingerprint(&mut output, None)?;
    let expected = format!("{:x}", hash.finalize());
    if actual != expected {
        return Err(err("verify", "Copy verification failed. Source retained."));
    }
    copy_metadata(&input, &output)?;
    private_store::sync(&output)?;
    private_store::sync(to)?;
    Ok(())
}
fn trash(parent: &File, name: &std::ffi::OsStr, expected: &Stamp) -> Result<(), FileError> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| err("trash", "System Trash unavailable; source retained."))?;
        let path = home.join(".Trash");
        let dir=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|_|err("trash","System Trash unavailable; source retained. No permanent deletion was attempted."))?;
        let meta = dir.metadata()?;
        if meta.dev() != expected.0
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o022 != 0
        {
            return Err(err(
                "trash",
                "Trash on this volume is unavailable; source retained.",
            ));
        }
        let new = OsString::from(format!("{}-{}", name.to_string_lossy(), uid()));
        if stamp(&stat_at(parent, name)?) != *expected {
            return Err(stale());
        }
        exclusive_rename(parent, name, &dir, &new)
    }
    #[cfg(target_os = "linux")]
    {
        linux_trash::trash(parent, name, expected)
    }
}

fn load_ignore(root: &File, path: &Path) -> Result<ignore::gitignore::Gitignore, FileError> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(path);
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let exists = unsafe {
        libc::fstatat(
            root.as_raw_fd(),
            c(std::ffi::OsStr::new(".gitignore"))?.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if exists != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error.into());
        }
    } else {
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(err(
                "ignore",
                "Root .gitignore must be a regular file, not a link or special file.",
            ));
        }
        let scoped =
            ScopedFile::from_parent(root.try_clone()?, path.into(), OsString::from(".gitignore"))?;
        let (file, _, _) = scoped.snapshot_bounded(65536)?;
        let mut text = String::new();
        file.take(65537).read_to_string(&mut text)?;
        if text.len() > 65536 {
            return Err(err("ignore", "Root .gitignore exceeds the 64 KiB budget."));
        }
        for line in text.lines() {
            builder
                .add_line(None, line)
                .map_err(|_| err("ignore", "Root .gitignore has an unsupported pattern."))?;
        }
    }
    builder
        .build()
        .map_err(|_| err("ignore", "Root .gitignore could not be compiled."))
}

fn validate_parent_path(file: &File, path: &Path) -> Result<(), FileError> {
    if fs::canonicalize(path)? != path {
        return Err(stale());
    }
    let current = fs::metadata(path)?;
    let pinned = file.metadata()?;
    if (current.dev(), current.ino()) != (pinned.dev(), pinned.ino()) {
        return Err(stale());
    }
    Ok(())
}

fn copy_metadata(input: &File, output: &File) -> Result<(), FileError> {
    let m = input.metadata()?;
    #[cfg(target_os = "macos")]
    if unsafe {
        libc::fcopyfile(
            input.as_raw_fd(),
            output.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        )
    } != 0
    {
        return Err(err(
            "metadata",
            "Copy metadata could not be preserved. Source retained; review the partial destination.",
        ));
    }
    let times = [
        libc::timespec {
            tv_sec: m.atime(),
            tv_nsec: m.atime_nsec() as _,
        },
        libc::timespec {
            tv_sec: m.mtime(),
            tv_nsec: m.mtime_nsec() as _,
        },
    ];
    if unsafe { libc::futimens(output.as_raw_fd(), times.as_ptr()) } != 0
        || unsafe { libc::fchmod(output.as_raw_fd(), (m.mode() & 0o777) as libc::mode_t) } != 0
    {
        return Err(err(
            "metadata",
            "Copy metadata could not be preserved. Source retained; review the partial destination.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_context_is_bounded_display_only() {
        assert_eq!(
            display_context(Path::new("/private/alpha/notes")),
            "alpha / notes"
        );
        assert_ne!(
            display_context(Path::new("/a/alpha")),
            display_context(Path::new("/a/beta"))
        );
        assert_eq!(display_context(Path::new("/")), "");
        assert_eq!(
            display_context(Path::new("/a/evil\n\u{202e}name")),
            "a / evilname"
        );
        assert_eq!(
            display_context(Path::new(&format!("/a/{}", "文".repeat(200))))
                .chars()
                .count(),
            120
        );
    }
    fn setup() -> (tempfile::TempDir, Library, Folder) {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("root")).unwrap();
        let store = temp.path().join("private");
        private_store::directory(&store).unwrap();
        let mut l = Library::new(store.clone(), vec![store]);
        let root = l.select(&temp.path().join("root")).unwrap();
        (temp, l, root)
    }
    fn all(l: &mut Library, dir: &str) -> Page {
        loop {
            let p = l.page(dir, 0, "name", true, "all", false, true).unwrap();
            if p.complete {
                return p;
            }
        }
    }
    fn request(
        kind: &str,
        entries: Vec<String>,
        destination: Option<String>,
        name: Option<&str>,
    ) -> Request {
        Request {
            kind: kind.into(),
            entries,
            destination,
            name: name.map(str::to_owned),
        }
    }
    fn execute(l: &mut Library, r: Request) -> Progress {
        let review = l.prepare(r).unwrap();
        let moved = l.take_plan(&review.id).unwrap().run();
        for (old, new) in moved {
            l.relocate(&old, &new);
            l.select_moved(&new).unwrap();
        }
        l.finish_mutation(false);
        let p = l.progress(&review.id, false).unwrap();
        l.invalidate();
        p
    }
    #[test]
    fn paging_and_filters_do_not_send_entire_tree() {
        let (t, mut l, r) = setup();
        for i in 0..1250 {
            fs::write(t.path().join(format!("root/{i:04}.txt")), b"x").unwrap();
        }
        let p = l
            .page(&r.id, 0, "name", false, "all", false, false)
            .unwrap();
        assert_eq!(p.items.len(), 100);
        assert!(!p.complete);
        let p = all(&mut l, &r.id);
        assert_eq!(p.total, 1250);
        let p = l
            .page(&r.id, 1200, "name", false, "all", false, true)
            .unwrap();
        assert_eq!(p.items.len(), 50);
        assert_eq!(p.items[0].name, "1200.txt");
    }
    #[test]
    fn note_queries_filter_before_paging_and_preserve_location_across_revisions() {
        let (t, mut library, root) = setup();
        let folder = t.path().join("root");
        for i in 0..520 {
            fs::write(folder.join(format!("note-{i:03}.md")), b"note").unwrap();
        }
        for name in ["Écrit العربية.TXT", ".Écrit.md", "Écrit.png"] {
            fs::write(folder.join(name), b"first").unwrap();
        }
        fs::create_dir(folder.join("Écrit")).unwrap();

        let query = |library: &mut Library, hidden, filter, mut refresh| loop {
            let page = library
                .page_matching(&root.id, 0, "name", hidden, filter, refresh, false, "ÉCRIT")
                .unwrap();
            if page.complete {
                break page;
            }
            refresh = false;
        };
        let found = query(&mut library, false, "notes", false);
        assert_eq!(found.total, 1);
        assert_eq!(found.items[0].name, "Écrit العربية.TXT");
        let original_id = &found.items[0].id;
        let original_location = found.items[0].location.as_deref();
        assert_eq!(
            original_location,
            Some(crate::scoped_file::path_key(&folder.join("Écrit العربية.TXT")).as_str())
        );
        assert_eq!(query(&mut library, true, "notes", false).total, 2);
        let folders = query(&mut library, false, "folders", false);
        assert_eq!(folders.total, 1);
        assert!(folders.items[0].directory.is_some());
        fs::write(folder.join("Écrit العربية.TXT"), b"a different revision").unwrap();
        let refreshed = query(&mut library, false, "notes", true);
        assert_ne!(&refreshed.items[0].id, original_id);
        assert_eq!(refreshed.items[0].location.as_deref(), original_location);
        assert!(library.scoped(original_id).is_err());
    }
    #[test]
    fn symlinks_specials_and_protected_trees_do_not_grant_reads() {
        let (t, mut l, r) = setup();
        std::os::unix::fs::symlink(t.path(), t.path().join("root/loop")).unwrap();
        fs::create_dir(t.path().join("root/.GiT")).unwrap();
        fs::write(t.path().join("secret.txt"), b"secret").unwrap();
        std::os::unix::fs::symlink(t.path().join("secret.txt"), t.path().join("root/leak.txt"))
            .unwrap();
        let p = all(&mut l, &r.id);
        assert_eq!(p.total, 2);
        assert!(
            p.items
                .iter()
                .all(|i| i.kind == "link" && i.directory.is_none())
        );
        for i in p.items {
            assert!(l.scoped(&i.id).is_err());
        }
        assert!(l.page("../", 0, "name", true, "all", false, true).is_err());
    }
    #[test]
    fn root_and_directory_swaps_invalidate_handles() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/a.txt"), b"x").unwrap();
        let p = all(&mut l, &r.id);
        fs::rename(t.path().join("root"), t.path().join("old")).unwrap();
        fs::create_dir(t.path().join("root")).unwrap();
        assert!(l.scoped(&p.items[0].id).is_err());
        assert!(l.page(&r.id, 0, "name", false, "all", false, true).is_err());
    }
    #[test]
    fn stale_preflight_never_mutates_replaced_source() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/a.txt"), b"before").unwrap();
        let p = all(&mut l, &r.id);
        let review = l
            .prepare(request(
                "rename",
                vec![p.items[0].id.clone()],
                Some(r.id.clone()),
                Some("b.txt"),
            ))
            .unwrap();
        fs::write(t.path().join("root/a.txt"), b"after").unwrap();
        assert!(l.take_plan(&review.id).is_err());
        assert!(!t.path().join("root/b.txt").exists());
    }
    #[test]
    fn exclusive_copy_and_rename_preserve_mixed_bytes() {
        let (t, mut l, r) = setup();
        let bytes = b"\xef\xbb\xbf# Hello\r\n\nlast\r";
        fs::write(t.path().join("root/a.txt"), bytes).unwrap();
        fs::create_dir(t.path().join("root/dest")).unwrap();
        let p = all(&mut l, &r.id);
        let file = p
            .items
            .iter()
            .find(|i| i.name == "a.txt")
            .unwrap()
            .id
            .clone();
        let dest = p
            .items
            .iter()
            .find(|i| i.name == "dest")
            .unwrap()
            .directory
            .clone()
            .unwrap();
        let p = execute(
            &mut l,
            request("copy", vec![file.clone()], Some(dest), None),
        );
        assert_eq!(p.outcomes[0].status, "completed");
        assert_eq!(fs::read(t.path().join("root/dest/a.txt")).unwrap(), bytes);
        let p = execute(
            &mut l,
            request("rename", vec![file], Some(r.id.clone()), Some("new.txt")),
        );
        assert_eq!(p.outcomes[0].status, "completed");
        assert_eq!(fs::read(t.path().join("root/new.txt")).unwrap(), bytes);
        assert!(!t.path().join("root/a.txt").exists());
        assert_eq!(p.selection.len(), 1);
        assert_eq!(p.selection[0].name, "new.txt");
        assert!(l.scoped(&p.selection[0].id).is_ok());
    }
    #[test]
    fn collision_and_cancel_preserve_source_and_existing_destination() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/A.txt"), b"a").unwrap();
        fs::write(t.path().join("root/B.txt"), b"b").unwrap();
        let p = all(&mut l, &r.id);
        let a = p
            .items
            .iter()
            .find(|i| i.name == "A.txt")
            .unwrap()
            .id
            .clone();
        let result = execute(
            &mut l,
            request("rename", vec![a.clone()], Some(r.id.clone()), Some("b.txt")),
        );
        assert_eq!(result.outcomes[0].status, "failed");
        assert_eq!(fs::read(t.path().join("root/B.txt")).unwrap(), b"b");
        let review = l
            .prepare(request(
                "rename",
                vec![a],
                Some(r.id.clone()),
                Some("c.txt"),
            ))
            .unwrap();
        let mutation = l.take_plan(&review.id).unwrap();
        l.progress(&review.id, true).unwrap();
        mutation.run();
        assert!(l.progress(&review.id, false).unwrap().canceled);
        assert!(t.path().join("root/A.txt").exists());
        assert!(!t.path().join("root/c.txt").exists());
    }
    #[test]
    fn sixteen_session_hints_survive_restart_without_restoring_root_authority() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("notes");
        fs::create_dir(&root).unwrap();
        let private = dir.path().join("private");
        private_store::directory(&private).unwrap();
        let paths = (0..16)
            .map(|index| {
                let path = root.join(format!("note-{index}.md"));
                fs::write(&path, "# Note").unwrap();
                path
            })
            .collect::<Vec<_>>();
        let mut library = Library::new(private.clone(), vec![]);
        library.select(&root).unwrap();
        library.session(paths.clone(), 15).unwrap();
        let mut overflow = paths;
        overflow.push(root.join("overflow.md"));
        assert!(library.session(overflow, 16).is_err());
        drop(library);
        let mut restored = Library::new(private, vec![]);
        assert_eq!(restored.state().session.len(), 16);
        assert_eq!(restored.state().active, 15);
        assert!(restored.state().session.iter().all(|hint| !hint.authorized));
        restored.select(&root).unwrap();
        assert!(restored.state().session.iter().all(|hint| hint.authorized));
    }
    #[test]
    fn note_catalogue_reads_only_selected_visible_markdown_and_bounds_captures() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("notes");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("First.md"), "# First\n[[Second]]").unwrap();
        fs::write(root.join("Second.md"), "# Second").unwrap();
        fs::write(root.join("large.md"), "x".repeat(65537)).unwrap();
        fs::write(root.join(".hidden.md"), "hidden").unwrap();
        fs::write(root.join("private.txt"), "not indexed").unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("escape")).unwrap();
        let mut library = Library::new(dir.path().join("private"), vec![]);
        assert!(library.note_catalogue(&root.join("First.md")).is_err());
        let folder = library.select(&root).unwrap();
        let catalogue = library.note_catalogue(&root.join("First.md")).unwrap();
        assert_eq!(catalogue.root.id, folder.id);
        assert_eq!(catalogue.active, "First.md");
        assert!(catalogue.complete);
        assert_eq!(catalogue.notes.len(), 3);
        assert_eq!(catalogue.skipped, 1);
        assert!(
            catalogue
                .notes
                .iter()
                .find(|n| n.path == "large.md")
                .unwrap()
                .text
                .is_none()
        );
        assert!(
            catalogue
                .notes
                .iter()
                .find(|n| n.path == "First.md")
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .contains("[[Second]]")
        );
        library.remove_root(&folder.id).unwrap();
        assert!(library.note_catalogue(&root.join("First.md")).is_err());
    }
    #[test]
    fn create_rejects_paths_and_never_overwrites() {
        let (t, mut l, r) = setup();
        for bad in [
            "../escape",
            "/tmp/oops",
            ".git",
            "a/b",
            "a\\b",
            "x\n",
            "trail.",
            ".",
            "",
        ] {
            assert!(
                l.prepare(request("createFile", vec![], Some(r.id.clone()), Some(bad)))
                    .is_err()
            );
        }
        let p = execute(
            &mut l,
            request("createFile", vec![], Some(r.id.clone()), Some("new.txt")),
        );
        assert_eq!(p.outcomes[0].status, "completed");
        fs::write(t.path().join("root/new.txt"), b"keep").unwrap();
        let p = execute(
            &mut l,
            request("createFile", vec![], Some(r.id.clone()), Some("new.txt")),
        );
        assert_eq!(p.outcomes[0].status, "failed");
        assert_eq!(fs::read(t.path().join("root/new.txt")).unwrap(), b"keep");
    }
    #[test]
    fn persisted_locations_do_not_restore_authorization() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/a.txt"), b"text").unwrap();
        let p = all(&mut l, &r.id);
        l.remember(&p.items[0].id).unwrap();
        let hint = l.state().recent[0].id.clone();
        let mut restored = Library::new(t.path().join("private"), vec![]);
        assert!(restored.state().roots.is_empty());
        assert!(restored.state().recent.is_empty());
        assert!(restored.resolve_hint(&hint).is_err());
        restored.select(&t.path().join("root")).unwrap();
        assert!(restored.resolve_hint(&hint).is_ok());
        assert_eq!(restored.state().recent[0].id, hint);
        fs::remove_file(t.path().join("root/a.txt")).unwrap();
        assert!(restored.state().recent.is_empty());
    }
    #[test]
    fn ignore_and_hidden_policy_is_explicit() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/.gitignore"), "*.txt\n!keep.txt\n").unwrap();
        fs::write(t.path().join("root/a.txt"), b"x").unwrap();
        fs::write(t.path().join("root/keep.txt"), b"x").unwrap();
        l.remove_root(&r.id).unwrap();
        let r = l.select(&t.path().join("root")).unwrap();
        let p = l.page(&r.id, 0, "name", false, "all", true, false).unwrap();
        assert_eq!(
            p.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["keep.txt"]
        );
        let p = l.page(&r.id, 0, "name", true, "all", true, true).unwrap();
        assert_eq!(p.total, 3);
    }
    #[test]
    fn search_crosses_chunks_and_cancellation_is_checked() {
        let (t, l, _) = setup();
        let mut bytes = vec![b'a'; 65534];
        bytes.extend_from_slice(b"needle\n");
        fs::write(t.path().join("root/a.txt"), bytes).unwrap();
        let file = ScopedFile::selected(&t.path().join("root/a.txt")).unwrap();
        assert!(
            search_file(file, "needle", &AtomicBool::new(false))
                .unwrap()
                .is_some()
        );
        let shared = Arc::new(Mutex::new(l));
        let id = Library::start(shared.clone(), "needle".into(), true, false, false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let p = shared.lock().unwrap().progress(&id, false).unwrap();
            if !p.running {
                assert_eq!(p.results.len(), 1);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn relative_resolution_stays_in_fresh_root() {
        let (t, mut l, _) = setup();
        fs::write(t.path().join("root/other file.txt"), b"text").unwrap();
        assert!(
            l.resolve_link(
                &fs::canonicalize(t.path().join("root"))
                    .unwrap()
                    .join("base.txt"),
                "other%20file.txt#h"
            )
            .unwrap()
            .is_some()
        );
        assert!(
            l.resolve_link(
                &fs::canonicalize(t.path().join("root"))
                    .unwrap()
                    .join("base.txt"),
                "../secret.txt"
            )
            .unwrap()
            .is_none()
        );
        assert!(
            l.resolve_link(
                &fs::canonicalize(t.path().join("root"))
                    .unwrap()
                    .join("base.txt"),
                "https://host/a"
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn folder_and_session_hints_restore_only_after_a_fresh_root_grant() {
        let (t, mut l, r) = setup();
        fs::create_dir(t.path().join("root/nested")).unwrap();
        fs::write(t.path().join("root/nested/a.txt"), b"x").unwrap();
        let child = all(&mut l, &r.id).items[0].directory.clone().unwrap();
        l.location(
            &child,
            Display {
                offset: 200,
                sort: "size".into(),
                hidden: true,
                ignored: false,
                filter: "text".into(),
            },
        )
        .unwrap();
        l.session(
            vec![fs::canonicalize(t.path().join("root/nested/a.txt")).unwrap()],
            0,
        )
        .unwrap();
        let mut restored = Library::new(t.path().join("private"), vec![]);
        assert!(restored.state().roots.is_empty());
        assert!(!restored.state().session[0].authorized);
        let folder = restored.select(&t.path().join("root")).unwrap();
        assert_eq!(folder.name, "nested");
        assert_eq!(restored.state().display.offset, 200);
        assert_eq!(restored.state().display.sort, "size");
        assert!(restored.state().session[0].authorized);
    }
    #[test]
    fn protected_ancestors_cannot_be_trashed_or_moved() {
        let (t, mut l, r) = setup();
        let protected = t.path().join("root/container/private");
        fs::create_dir_all(&protected).unwrap();
        l.protected.push(fs::canonicalize(&protected).unwrap());
        let item = all(&mut l, &r.id)
            .items
            .into_iter()
            .find(|i| i.name == "container")
            .unwrap();
        assert!(
            l.prepare(request("trash", vec![item.id.clone()], None, None))
                .is_err()
        );
        assert!(
            l.prepare(request("rename", vec![item.id], Some(r.id), Some("new")))
                .is_err()
        );
        assert!(protected.exists());
    }
    #[test]
    fn copy_partial_results_keep_sources_and_never_follow_nested_links() {
        let (t, mut l, r) = setup();
        fs::create_dir_all(t.path().join("root/source")).unwrap();
        fs::create_dir(t.path().join("root/dest")).unwrap();
        fs::write(t.path().join("root/source/a.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(t.path(), t.path().join("root/source/link")).unwrap();
        let items = all(&mut l, &r.id).items;
        let source = items
            .iter()
            .find(|i| i.name == "source")
            .unwrap()
            .id
            .clone();
        let dest = items
            .iter()
            .find(|i| i.name == "dest")
            .unwrap()
            .directory
            .clone()
            .unwrap();
        let result = execute(&mut l, request("copy", vec![source], Some(dest), None));
        assert_eq!(result.outcomes[0].status, "failed");
        assert_eq!(
            fs::read(t.path().join("root/source/a.txt")).unwrap(),
            b"keep"
        );
        assert!(!t.path().join("root/dest/source/link").exists());
    }
    #[test]
    fn unavailable_trash_retains_source_and_never_deletes() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/a.txt"), b"keep").unwrap();
        let item = all(&mut l, &r.id).items[0].id.clone();
        let (e, parent, _) = l.entry(&item).unwrap();
        let mut wrong = e.stamp;
        wrong.0 = wrong.0.wrapping_add(1);
        assert!(trash(&parent, &e.name, &wrong).is_err());
        assert_eq!(fs::read(t.path().join("root/a.txt")).unwrap(), b"keep");
    }
    #[test]
    fn unusual_names_are_kept_in_native_capabilities() {
        let (t, mut l, r) = setup();
        let raw = OsString::from_vec(b"raw-\xff.txt".to_vec());
        match fs::write(t.path().join("root").join(raw), b"exact") {
            Ok(()) => {}
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(libc::EILSEQ) => {
                eprintln!(
                    "RAW_FILENAME_NOT_RUN: macOS filesystem refuses invalid UTF-8 (EILSEQ); testing Arabic/emoji instead."
                );
                fs::write(t.path().join("root/ملاحظات 🦀.txt"), b"exact").unwrap();
            }
            Err(error) => panic!("native filename fixture creation failed: {error}"),
        }
        let item = all(&mut l, &r.id).items[0].id.clone();
        assert_eq!(l.preview(&item).unwrap(), "exact");
        let result = execute(
            &mut l,
            request("rename", vec![item], Some(r.id), Some("renamed.txt")),
        );
        assert_eq!(result.outcomes[0].status, "completed");
        assert_eq!(
            fs::read(t.path().join("root/renamed.txt")).unwrap(),
            b"exact"
        );
    }
    #[test]
    fn content_search_rejects_invalid_utf8_after_a_match() {
        let (t, _, _) = setup();
        let path = fs::canonicalize(t.path().join("root"))
            .unwrap()
            .join("a.txt");
        fs::write(&path, b"needle\xff").unwrap();
        assert!(
            search_file(
                ScopedFile::selected(&path).unwrap(),
                "needle",
                &AtomicBool::new(false)
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn unchanged_refresh_reuses_handles_and_deleted_files_release_them() {
        let (t, mut l, r) = setup();
        fs::write(t.path().join("root/a.txt"), b"x").unwrap();
        let first = all(&mut l, &r.id).items[0].id.clone();
        for _ in 0..40 {
            let page = l.page(&r.id, 0, "name", true, "all", true, true).unwrap();
            assert_eq!(page.items[0].id, first);
        }
        assert_eq!(l.entries.len(), 1);
        fs::remove_file(t.path().join("root/a.txt")).unwrap();
        l.page(&r.id, 0, "name", true, "all", true, true).unwrap();
        assert!(l.entries.is_empty());
        assert!(l.lookup.is_empty());
    }
    #[test]
    fn previous_session_tokens_survive_opening_the_first_restored_tab() {
        let (t, mut l, r) = setup();
        let root = fs::canonicalize(t.path().join("root")).unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join("b.txt"), b"b").unwrap();
        l.session(vec![root.join("a.txt"), root.join("b.txt")], 1)
            .unwrap();
        l.remove_root(&r.id).unwrap();
        let mut restored = Library::new(t.path().join("private"), vec![]);
        restored.select(&root).unwrap();
        let hints = restored.state().session;
        assert_eq!(restored.state().active, 1);
        restored.session(vec![root.join("a.txt")], 0).unwrap();
        assert!(restored.resolve_hint(&hints[1].id).is_ok());
    }
    #[test]
    fn recency_does_not_evict_bookmarks_and_pending_operations_are_durable() {
        let (t, mut l, r) = setup();
        let root = fs::canonicalize(t.path().join("root")).unwrap();
        let star = root.join("star.txt");
        fs::write(&star, "bookmark").unwrap();
        let id = l.remember_path(&star);
        l.pin(&id).unwrap();
        for i in 0..130 {
            let path = root.join(format!("recent-{i}.txt"));
            fs::write(&path, "recent").unwrap();
            l.remember_path(&path);
        }
        assert!(l.state().recent.iter().any(|r| r.id == id && r.pinned));
        assert_eq!(l.state().recent.len(), 101);
        assert!(l.state().can_clear_recent);
        l.clear_recent().unwrap();
        assert!(!l.state().can_clear_recent);
        assert_eq!(l.state().recent.len(), 1);
        assert_eq!(l.state().recent[0].id, id);
        let review = l
            .prepare(request(
                "createFile",
                vec![],
                Some(r.id),
                Some("pending.txt"),
            ))
            .unwrap();
        let task = l.take_plan(&review.id).unwrap();
        let restored = Library::new(t.path().join("private"), vec![]);
        assert_eq!(restored.state().last_operation[0].status, "pending");
        assert!(!root.join("pending.txt").exists());
        drop(task);
    }
    #[test]
    fn unknown_library_schema_is_preserved() {
        let (t, _, _) = setup();
        let path = t.path().join("private/library.json");
        let bytes = b"{\"schema\":99}";
        fs::write(&path, bytes).unwrap();
        let mut l = Library::new(t.path().join("private"), vec![]);
        assert!(l.configure(true, "grid").is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    #[ignore = "requires an explicitly mounted disposable second filesystem; scripts/check-cross-volume.py"]
    fn cross_volume_copy_and_directory_move_preserve_exact_sources() {
        let mount = PathBuf::from(
            std::env::var_os("MARKTUB_CROSS_VOLUME_ROOT")
                .expect("explicit mounted test filesystem required"),
        );
        let (t, mut l, root) = setup();
        let destination = tempfile::tempdir_in(&mount).unwrap();
        assert_ne!(
            fs::metadata(t.path()).unwrap().dev(),
            fs::metadata(destination.path()).unwrap().dev(),
            "must exercise real EXDEV branch"
        );
        let dest = l.select(destination.path()).unwrap();
        let bytes = b"\xef\xbb\xbf# Cross volume\r\n\nlast\r";
        fs::write(t.path().join("root/mixed.txt"), bytes).unwrap();
        fs::create_dir(t.path().join("root/tree")).unwrap();
        fs::write(t.path().join("root/tree/child.txt"), bytes).unwrap();
        let entries = all(&mut l, &root.id);
        let file = entries
            .items
            .iter()
            .find(|e| e.name == "mixed.txt")
            .unwrap()
            .id
            .clone();
        let folder = entries
            .items
            .iter()
            .find(|e| e.name == "tree")
            .unwrap()
            .id
            .clone();
        let copied = execute(
            &mut l,
            request("copy", vec![file.clone()], Some(dest.id.clone()), None),
        );
        assert_eq!(copied.outcomes[0].status, "completed");
        assert_eq!(
            fs::read(destination.path().join("mixed.txt")).unwrap(),
            bytes
        );
        assert_eq!(fs::read(t.path().join("root/mixed.txt")).unwrap(), bytes);
        let collided = execute(
            &mut l,
            request("copy", vec![file], Some(dest.id.clone()), None),
        );
        assert_eq!(collided.outcomes[0].status, "failed");
        assert_eq!(
            fs::read(destination.path().join("mixed.txt")).unwrap(),
            bytes
        );
        let moved = execute(
            &mut l,
            request("move", vec![folder], Some(dest.id.clone()), None),
        );
        assert_eq!(moved.outcomes[0].status, "partial");
        assert!(moved.outcomes[0].message.contains("source retained"));
        assert_eq!(
            fs::read(destination.path().join("tree/child.txt")).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(t.path().join("root/tree/child.txt")).unwrap(),
            bytes
        );
        // Fill only this disposable image, never the host filesystem. Exercise
        // actual ENOSPC in native copy and durable Review commits.
        use std::io::Write;
        let source = vec![b'X'; 2 * 1024 * 1024];
        fs::write(t.path().join("root/diskfull.txt"), &source).unwrap();
        l.invalidate();
        let file = all(&mut l, &root.id)
            .items
            .iter()
            .find(|item| item.name == "diskfull.txt")
            .unwrap()
            .id
            .clone();
        let store = destination.path().join("private-records");
        private_store::directory(&store).unwrap();
        let mut reviews = crate::review::Reviews::new(store.clone());
        let view = reviews
            .begin(
                crate::review::Capture::new(Path::new("/selected/owned.txt"), source.clone())
                    .unwrap(),
            )
            .unwrap();
        let record = store
            .join("review")
            .join(format!("{}.json", reviews.list().unwrap()[0].id));
        let record_bytes = fs::read(&record).unwrap();
        let fill_path = destination.path().join("fill-space");
        let mut filler = File::create(&fill_path).unwrap();
        let chunk = vec![0xa5; 1024 * 1024];
        let mut full = false;
        for _ in 0..256 {
            match filler.write_all(&chunk) {
                Ok(()) => {}
                Err(e) if e.raw_os_error() == Some(libc::ENOSPC) => {
                    full = true;
                    break;
                }
                Err(e) => panic!("unexpected image fill error: {e}"),
            }
        }
        assert!(
            full,
            "disposable 128 MiB image must report ENOSPC within the cap"
        );
        let failed = execute(
            &mut l,
            request("copy", vec![file], Some(dest.id.clone()), None),
        );
        assert_ne!(failed.outcomes[0].status, "completed");
        assert_eq!(
            fs::read(t.path().join("root/diskfull.txt")).unwrap(),
            source
        );
        assert_eq!(
            fs::read(destination.path().join("mixed.txt")).unwrap(),
            bytes
        );
        assert!(
            reviews
                .update(&view.token, view.version, crate::review::Action::Reviewed)
                .is_err()
        );
        assert_eq!(fs::read(record).unwrap(), record_bytes);
        drop(filler);
        fs::remove_file(fill_path).unwrap();
    }
}
