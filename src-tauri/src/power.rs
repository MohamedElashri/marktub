//! Optional local jobs. All authority originates in native selection/confirmation.
pub mod pdf;
pub mod process;
pub mod sql;
use crate::{files::FileError, private_store, scoped_file::ScopedFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use uuid::Uuid;
pub const CAPTURE_LIMIT: usize = 2 * 1024 * 1024;
pub fn err(s: impl Into<String>) -> FileError {
    FileError::new("power", s)
}
pub fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn uid() -> String {
    Uuid::new_v4().to_string()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Shell,
    Sqlite,
    Tex,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Shell,
    Pandoc,
    Typst,
    Tex,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub id: String,
    pub role: Role,
    pub name: String,
    pub version: String,
}
pub struct Tool {
    pub view: ToolView,
    pub path: PathBuf,
    scoped: ScopedFile,
    identity: crate::atomic_save::Identity,
    _pin: File,
}
impl Tool {
    pub fn select(path: &Path, role: Role, scratch: &Path) -> Result<Self, FileError> {
        let canonical = fs::canonicalize(path)?;
        let scoped = ScopedFile::selected(&canonical)?.authorize_selection(path);
        let file = scoped.open()?;
        if file.metadata()?.permissions().mode() & 0o111 == 0 {
            return Err(err("Choose an executable file."));
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| err("Tool name must be UTF-8."))?
            .to_string();
        let allowed = match role {
            Role::Shell => matches!(name.as_str(), "sh" | "bash" | "zsh"),
            Role::Pandoc => name == "pandoc",
            Role::Typst => name == "typst",
            Role::Tex => matches!(name.as_str(), "pdflatex" | "xelatex" | "lualatex"),
        };
        if !allowed {
            return Err(err(
                "Choose the named installed tool: sh/bash/zsh, pandoc, typst, or pdflatex/xelatex/lualatex for its role.",
            ));
        }
        let identity = crate::atomic_save::Identity::of(&file.metadata()?);
        if file.metadata()?.len() > 512 * 1024 * 1024 {
            return Err(err("Executable exceeds 512 MiB."));
        }
        let output = process::run(
            &canonical,
            &["--version".into()],
            scratch,
            None,
            &Arc::new(AtomicBool::new(false)),
            5,
        )?;
        let version = output.stdout.lines().next().unwrap_or("").to_string();
        let valid = output.code == Some(0)
            && version.len() <= 256
            && match role {
                Role::Shell => !version.is_empty(),
                Role::Pandoc => version.starts_with("pandoc 3."),
                Role::Typst => version.starts_with("typst 0."),
                Role::Tex => version.contains("TeX"),
            };
        if !valid {
            return Err(err(
                "Unsupported tool/version. Use Pandoc 3.x, Typst 0.x, a supported shell or TeX Live engine.",
            ));
        }
        let tool = Self {
            view: ToolView {
                id: uid(),
                role,
                name,
                version,
            },
            path: canonical,
            scoped,
            identity,
            _pin: file,
        };
        tool.validate()?;
        Ok(tool)
    }
    pub fn validate(&self) -> Result<(), FileError> {
        self.scoped.validate_selection()?;
        if crate::atomic_save::Identity::of(&self.scoped.open()?.metadata()?) != self.identity {
            return Err(err("Selected tool changed; choose it again."));
        }
        Ok(())
    }
}
pub struct Root {
    pub id: String,
    pub kind: Kind,
    pub path: PathBuf,
    pub file: File,
    identity: (u64, u64),
}
impl Root {
    pub fn select(path: &Path, kind: Kind) -> Result<Self, FileError> {
        let path = fs::canonicalize(path)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let m = file.metadata()?;
        Ok(Self {
            id: uid(),
            kind,
            path,
            file,
            identity: (m.dev(), m.ino()),
        })
    }
    pub fn validate(&self) -> Result<(), FileError> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)?;
        let m = f.metadata()?;
        if (m.dev(), m.ino()) != self.identity {
            return Err(err("Trusted folder moved or changed; choose it again."));
        }
        Ok(())
    }
}
pub struct Database {
    pub id: String,
    pub path: PathBuf,
    scoped: ScopedFile,
    identity: (u64, u64),
    _pin: File,
}
impl Database {
    pub fn select(path: &Path) -> Result<Self, FileError> {
        let canonical = fs::canonicalize(path)?;
        let scoped = ScopedFile::selected(&canonical)?.authorize_selection(path);
        let file = scoped.open()?;
        let m = file.metadata()?;
        if m.nlink() != 1 || m.len() > 256 * 1024 * 1024 {
            return Err(err("Choose a regular single-link SQLite DB up to 256 MiB."));
        }
        let db = Self {
            id: uid(),
            path: canonical,
            scoped,
            identity: (m.dev(), m.ino()),
            _pin: file,
        };
        db.validate()?;
        Ok(db)
    }
    pub fn validate(&self) -> Result<(), FileError> {
        self.scoped.validate_selection()?;
        let m = self.scoped.open()?.metadata()?;
        if (m.dev(), m.ino()) != self.identity || m.nlink() != 1 {
            return Err(err("Database identity changed; choose it again."));
        }
        Ok(())
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: String,
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub code: String,
}
impl Block {
    pub fn execution_code(&self) -> String {
        self.code.replace("\r\n", "\n").replace('\r', "\n")
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureView {
    pub id: String,
    pub name: String,
    pub revision: String,
    pub origin: String,
    pub blocks: Vec<Block>,
    pub text: String,
    pub executable: bool,
}
#[derive(Clone)]
pub enum Proof {
    Saved(String),
    Working {
        id: String,
        generation: String,
        revision: u64,
    },
}
#[derive(Clone)]
pub struct Capture {
    pub view: CaptureView,
    pub path: Option<PathBuf>,
    pub proof: Option<Proof>,
    pub assets: HashMap<String, Vec<u8>>,
    pub created: Instant,
}
impl Capture {
    pub fn new(
        name: String,
        bytes: Vec<u8>,
        origin: String,
        path: Option<PathBuf>,
        document: Option<String>,
    ) -> Result<Self, FileError> {
        if bytes.len() > CAPTURE_LIMIT || bytes.contains(&0) {
            return Err(err(
                "Export/run captures admit UTF-8 up to 2 MiB without NUL. Larger documents remain readable/editable.",
            ));
        }
        let text = String::from_utf8(bytes).map_err(|_| err("Capture is not UTF-8."))?;
        let blocks = blocks(&text)?;
        let revision = hash(text.as_bytes());
        Ok(Self {
            view: CaptureView {
                id: uid(),
                name,
                revision,
                origin,
                blocks,
                text,
                executable: path.is_some(),
            },
            path,
            proof: document.map(Proof::Saved),
            assets: HashMap::new(),
            created: Instant::now(),
        })
    }
}
fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let line = &line[indent..];
    let c = *line.as_bytes().first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = line.bytes().take_while(|b| *b == c).count();
    if n < 3 {
        return None;
    }
    Some((c, n, line[n..].trim()))
}
pub fn blocks(text: &str) -> Result<Vec<Block>, FileError> {
    let mut result = vec![];
    let mut open: Option<(u8, usize, Option<Kind>, usize, usize)> = None;
    let mut at = 0;
    // split_inclusive also recognizes raw CR, CRLF and LF without normalizing
    // byte coordinates. Nested/indented/blockquote fences remain source only.
    let mut lines = vec![];
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' || b[i] == b'\n' {
            i += if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            lines.push(&text[start..i]);
            start = i;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    if lines.len() > 50000 {
        return Err(err("Capture exceeds 50,000 logical lines."));
    }
    for (number, line) in lines.into_iter().enumerate() {
        let check = if number == 0 {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if let Some((ch, n, kind, from, line_number)) = open {
            if fence(check).is_some_and(|(c, len, tail)| c == ch && len >= n && tail.is_empty()) {
                if let Some(kind) = kind {
                    let code = &text[from..at];
                    if code.len() > 65536 {
                        return Err(err("Runnable fences admit 64 KiB each."));
                    }
                    if result.len() >= 100 {
                        return Err(err("At most 100 runnable fences per capture."));
                    }
                    result.push(Block {
                        id: uid(),
                        kind,
                        start: from,
                        end: at,
                        line: line_number,
                        code: code.into(),
                    });
                }
                open = None;
            }
        } else if let Some((ch, n, info)) = fence(check) {
            let kind = match info {
                "sh" | "bash" | "shell" | "zsh" => Some(Kind::Shell),
                "sql" | "sqlite" => Some(Kind::Sqlite),
                "tex" | "latex" => Some(Kind::Tex),
                _ => None,
            };
            open = Some((ch, n, kind, at + line.len(), number + 2));
        }
        at += line.len();
    }
    Ok(result)
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootView {
    pub id: String,
    pub kind: Kind,
    pub name: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbView {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub shell: bool,
    pub sqlite: bool,
    pub tex: bool,
    pub tools: Vec<ToolView>,
    pub roots: Vec<RootView>,
    pub databases: Vec<DbView>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView {
    pub status: String,
    pub output: Option<process::Output>,
    pub table: Option<sql::Table>,
    pub message: String,
    pub pdf: bool,
    pub warnings: Vec<String>,
}
pub struct Job {
    pub kind: Option<Kind>,
    pub cancel: Arc<AtomicBool>,
    pub result: Mutex<ResultView>,
    pub pdf: Mutex<Option<Vec<u8>>>,
}
impl Job {
    pub fn new(kind: Option<Kind>) -> Self {
        Self {
            kind,
            cancel: Arc::new(AtomicBool::new(false)),
            result: Mutex::new(ResultView {
                status: "running".into(),
                output: None,
                table: None,
                message: "Running the confirmed captured revision.".into(),
                pdf: false,
                warnings: vec![],
            }),
            pdf: Mutex::new(None),
        }
    }
    pub fn finish(&self, result: Result<ResultView, FileError>) {
        let mut state = self.result.lock().unwrap();
        *state = match result {
            Ok(v) => v,
            Err(e) => ResultView {
                status: if e.code == "canceled" || self.cancel.load(Ordering::SeqCst) {
                    "canceled"
                } else {
                    "failed"
                }
                .into(),
                output: None,
                table: None,
                message: e.message,
                pdf: false,
                warnings: vec![],
            },
        };
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub capture: String,
    pub block: Option<String>,
    pub root: Option<String>,
    pub database: Option<String>,
    pub tool: Option<String>,
    pub pandoc: Option<String>,
    pub mode: Mode,
    pub toc: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Pdf,
    Shell,
    SqliteRead,
    SqliteWrite,
    Tex,
    PdfTex,
}
impl Mode {
    pub fn kind(self) -> Option<Kind> {
        match self {
            Self::Pdf => None,
            Self::Shell => Some(Kind::Shell),
            Self::SqliteRead | Self::SqliteWrite => Some(Kind::Sqlite),
            Self::Tex | Self::PdfTex => Some(Kind::Tex),
        }
    }
}
pub struct Prepared {
    pub request: Request,
    pub capture: Arc<Capture>,
    pub block: Option<Block>,
    pub root: Option<Arc<Root>>,
    pub database: Option<Arc<Database>>,
    pub tool: Option<Arc<Tool>>,
    pub pandoc: Option<Arc<Tool>>,
    pub summary: String,
    pub created: Instant,
}
pub struct Scratch {
    pub path: PathBuf,
}
impl Scratch {
    pub fn new(parent: &Path) -> Result<Self, FileError> {
        private_store::directory(parent)?;
        let path = parent.join(uid());
        private_store::directory(&path)?;
        Ok(Self { path })
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
pub struct Powers {
    pub store: PathBuf,
    pub protected: Vec<PathBuf>,
    enabled: [bool; 3],
    pub tools: HashMap<String, Arc<Tool>>,
    pub roots: HashMap<String, Arc<Root>>,
    pub databases: HashMap<String, Arc<Database>>,
    pub captures: HashMap<String, Arc<Capture>>,
    pub prepared: HashMap<String, Prepared>,
    pub jobs: HashMap<String, Arc<Job>>,
    pub exports: HashMap<String, (ScopedFile, String)>,
}
impl Powers {
    pub fn new(store: PathBuf, protected: Vec<PathBuf>) -> Self {
        Self {
            store,
            protected,
            enabled: [false; 3],
            tools: HashMap::new(),
            roots: HashMap::new(),
            databases: HashMap::new(),
            captures: HashMap::new(),
            prepared: HashMap::new(),
            jobs: HashMap::new(),
            exports: HashMap::new(),
        }
    }
    pub fn state(&self) -> State {
        State {
            shell: self.enabled[0],
            sqlite: self.enabled[1],
            tex: self.enabled[2],
            tools: self.tools.values().map(|t| t.view.clone()).collect(),
            roots: self
                .roots
                .values()
                .map(|r| RootView {
                    id: r.id.clone(),
                    kind: r.kind,
                    name: r.path.display().to_string(),
                })
                .collect(),
            databases: self
                .databases
                .values()
                .map(|d| DbView {
                    id: d.id.clone(),
                    name: d.path.display().to_string(),
                })
                .collect(),
        }
    }
    pub fn enabled(&self, k: Kind) -> bool {
        self.enabled[match k {
            Kind::Shell => 0,
            Kind::Sqlite => 1,
            Kind::Tex => 2,
        }]
    }
    pub fn enable(&mut self, k: Kind, on: bool) {
        self.enabled[match k {
            Kind::Shell => 0,
            Kind::Sqlite => 1,
            Kind::Tex => 2,
        }] = on;
        self.prepared.clear();
        if !on {
            self.roots.retain(|_, r| r.kind != k);
            for j in self.jobs.values().filter(|j| j.kind == Some(k)) {
                j.cancel.store(true, Ordering::SeqCst);
            }
        }
    }
    pub fn protect(&self, path: &Path) -> Result<(), FileError> {
        if self.protected.iter().any(|p| path.starts_with(p)) {
            return Err(err(
                "App-owned resources/private records cannot be selected for local jobs.",
            ));
        }
        Ok(())
    }
    pub fn scratch(&self) -> Result<Scratch, FileError> {
        if self.store.as_os_str().is_empty() {
            return Err(err("Private storage is unavailable."));
        }
        Scratch::new(&self.store.join("jobs"))
    }
    pub fn capture(&mut self, c: Capture) -> Result<CaptureView, FileError> {
        if self.captures.len() >= 4 {
            return Err(err(
                "Release another captured document first (four captures maximum).",
            ));
        }
        let view = c.view.clone();
        self.captures.insert(view.id.clone(), Arc::new(c));
        Ok(view)
    }
    pub fn release(&mut self, id: &str) {
        self.captures.remove(id);
        self.prepared.retain(|_, p| p.request.capture != id);
    }
    pub fn prepare(&mut self, r: Request) -> Result<(String, String), FileError> {
        if self.prepared.len() >= 8 {
            return Err(err(
                "Too many pending confirmations; close and reopen Export & run.",
            ));
        }
        let c = self
            .captures
            .get(&r.capture)
            .cloned()
            .ok_or_else(|| err("Capture ended. Capture the document again."))?;
        if c.created.elapsed().as_secs() > 1800 {
            return Err(err(
                "Capture expired after 30 minutes; capture the document again.",
            ));
        }
        let kind = r.mode.kind();
        let mut root = None;
        let mut block = None;
        if let Some(kind) = kind {
            if !self.enabled(kind) {
                return Err(err(
                    "This runner is disabled. Enable it explicitly in Export & run.",
                ));
            }
            if !c.view.executable {
                return Err(err(
                    "Save As before executing recovered text or app-owned resources.",
                ));
            }
            let rt = self
                .roots
                .get(r.root.as_deref().unwrap_or(""))
                .cloned()
                .filter(|rt| rt.kind == kind)
                .ok_or_else(|| err("Select and trust a folder for this runner first."))?;
            rt.validate()?;
            if !c.path.as_ref().is_some_and(|p| p.starts_with(&rt.path)) {
                return Err(err(
                    "The captured document is outside this runner's trusted folder.",
                ));
            }
            root = Some(rt);
            if r.mode != Mode::PdfTex {
                block = Some(
                    c.view
                        .blocks
                        .iter()
                        .find(|b| Some(&b.id) == r.block.as_ref() && b.kind == kind)
                        .cloned()
                        .ok_or_else(|| err("Select an admitted captured fence."))?,
                );
            }
        }
        if block.as_ref().is_some_and(|b| b.code.len() > 8192) {
            return Err(err(
                "Native confirmation admits at most 8 KiB of runnable code. Split this fence before running.",
            ));
        }
        let expected = match r.mode {
            Mode::Pdf => Some(Role::Typst),
            Mode::PdfTex | Mode::Tex => Some(Role::Tex),
            Mode::Shell => Some(Role::Shell),
            _ => None,
        };
        let tool = expected
            .map(|role| {
                self.tools
                    .get(r.tool.as_deref().unwrap_or(""))
                    .cloned()
                    .filter(|t| t.view.role == role)
                    .ok_or_else(|| err("Select the installed tool for this job first."))
            })
            .transpose()?;
        if r.mode == Mode::PdfTex && tool.as_ref().is_some_and(|t| t.view.name == "pdflatex") {
            return Err(err(
                "Unicode PDF conversion uses xelatex or lualatex. pdflatex remains available for explicitly trusted standalone TeX blocks.",
            ));
        }
        let pandoc = if matches!(r.mode, Mode::Pdf | Mode::PdfTex) {
            Some(
                self.tools
                    .get(r.pandoc.as_deref().unwrap_or(""))
                    .cloned()
                    .filter(|t| t.view.role == Role::Pandoc)
                    .ok_or_else(|| err("Select installed Pandoc first."))?,
            )
        } else {
            None
        };
        let database = if matches!(r.mode, Mode::SqliteRead | Mode::SqliteWrite) {
            Some(
                self.databases
                    .get(r.database.as_deref().unwrap_or(""))
                    .cloned()
                    .ok_or_else(|| err("Select a local SQLite database first."))?,
            )
        } else {
            None
        };
        let summary=format!("{:?}\nDocument: {}\n{}\nSHA-256: {}\nFolder: {}\nTool: {}\nDatabase: {}\n\n{}\n\n{}",r.mode,c.view.name,c.view.origin,c.view.revision,root.as_ref().map(|r|r.path.display().to_string()).unwrap_or_else(||"Private export staging".into()),tool.as_ref().map(|t|format!("{} ({})",t.path.display(),t.view.version)).unwrap_or_else(||"Built-in SQLite".into()),database.as_ref().map(|d|d.path.display().to_string()).unwrap_or_else(||"None".into()),block.as_ref().map(|b|b.execution_code()).unwrap_or_else(||"Convert this captured revision to PDF. Raw document commands, remote links and unapproved images are excluded.".into()),if kind.is_some(){"Shell/TeX retain normal user privileges. Folder trust is not an OS sandbox. Executable fence copies normalize line endings to LF; Markdown bytes are unchanged. SQLite write mode commits only a successful, uncanceled DML transaction."}else{"PDF tools are trusted local executables. PDF conversion does not enable block execution."});
        let token = uid();
        self.prepared.insert(
            token.clone(),
            Prepared {
                request: r,
                capture: c,
                block,
                root,
                database,
                tool,
                pandoc,
                summary: summary.clone(),
                created: Instant::now(),
            },
        );
        Ok((token, summary))
    }
    pub fn take(&mut self, token: &str) -> Result<Prepared, FileError> {
        let p = self
            .prepared
            .remove(token)
            .ok_or_else(|| err("Confirmation expired or was already used."))?;
        if p.created.elapsed().as_secs() > 300
            || p.request.mode.kind().is_some_and(|k| !self.enabled(k))
            || !self.captures.contains_key(&p.request.capture)
        {
            return Err(err("Job approval expired/revoked; prepare again."));
        }
        if self.jobs.len() >= 8 {
            return Err(err(
                "Clear completed jobs before starting another (eight maximum).",
            ));
        }
        Ok(p)
    }
    pub fn clear(&mut self) {
        self.prepared.clear();
        self.jobs
            .retain(|_, j| j.result.lock().unwrap().status == "running");
        self.exports.clear();
    }
}
pub fn execute(p: Prepared, scratch: Scratch, job: &Job) -> Result<ResultView, FileError> {
    if let Some(root) = &p.root {
        root.validate()?;
    }
    if let Some(tool) = &p.tool {
        tool.validate()?;
    }
    if let Some(tool) = &p.pandoc {
        tool.validate()?;
    }
    if let Some(db) = &p.database {
        db.validate()?;
    }
    let mut view = ResultView {
        status: "completed".into(),
        output: None,
        table: None,
        message: "Captured job completed.".into(),
        pdf: false,
        warnings: vec![],
    };
    match p.request.mode {
        Mode::Pdf | Mode::PdfTex => {
            let (bytes, warnings) = pdf::convert(&p, &scratch.path, &job.cancel)?;
            if job.cancel.load(Ordering::SeqCst) {
                return Err(FileError::new("canceled", "PDF canceled."));
            }
            *job.pdf.lock().unwrap() = Some(bytes);
            view.pdf = true;
            view.warnings = warnings;
            view.message = "PDF ready. Export chooses a new native destination.".into();
        }
        Mode::SqliteRead | Mode::SqliteWrite => {
            let db = p.database.as_ref().unwrap();
            view.table = Some(sql::query(
                &db.path,
                &p.block.as_ref().unwrap().execution_code(),
                p.request.mode == Mode::SqliteWrite,
                job.cancel.clone(),
                || {
                    db.validate()?;
                    p.root.as_ref().unwrap().validate()
                },
            )?);
        }
        Mode::Shell => {
            let tool = p.tool.as_ref().unwrap();
            let script = scratch.path.join("script.sh");
            private_store::atomic(
                &script,
                p.block.as_ref().unwrap().execution_code().as_bytes(),
            )?;
            let args = if tool.view.name == "bash" {
                vec![
                    "--noprofile".into(),
                    "--norc".into(),
                    script.display().to_string(),
                ]
            } else if tool.view.name == "zsh" {
                vec!["-f".into(), script.display().to_string()]
            } else {
                vec![script.display().to_string()]
            };
            let root = p.root.as_ref().unwrap();
            view.output = Some(process::run(
                &tool.path,
                &args,
                &scratch.path,
                Some(&root.file),
                &job.cancel,
                30,
            )?);
        }
        Mode::Tex => {
            let file = scratch.path.join("document.tex");
            private_store::atomic(&file, p.block.as_ref().unwrap().execution_code().as_bytes())?;
            let output = pdf::tex(p.tool.as_ref().unwrap(), &scratch.path, &job.cancel)?;
            let bytes = pdf::read_pdf(&scratch.path.join("document.pdf"))?;
            *job.pdf.lock().unwrap() = Some(bytes);
            view.pdf = true;
            view.output = Some(output);
            view.message =
                "Trusted local TeX PDF ready. Export to a new native destination.".into();
        }
    }
    if view.output.as_ref().is_some_and(|o| o.code != Some(0)) {
        view.status = "failed".into();
        view.message = "Tool exited unsuccessfully; review its bounded output.".into();
    }
    Ok(view)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fences_preserve_raw_coordinates_and_never_offer_nested_or_unclosed_code() {
        let s = "\u{feff}# Test\r\n```sh\rprintf exact\n```\r\n````text\n```sh\nevil\n```\n````\n    ```sh\nno\n    ```\n~~~sql\rSELECT 1;\r~~~\r```tex\nunclosed";
        let b = blocks(s).unwrap();
        assert_eq!(b.len(), 2);
        for block in &b {
            assert_eq!(&s[block.start..block.end], block.code);
        }
        assert_eq!(b[0].code, "printf exact\n");
        assert_eq!(b[1].code, "SELECT 1;\r");
        assert_eq!(b[1].execution_code(), "SELECT 1;\n");
    }
    #[test]
    fn disabled_forged_recovered_and_expired_authority_refuses() {
        let t = tempfile::tempdir().unwrap();
        let mut service = Powers::new(t.path().into(), vec![]);
        let path = t.path().join("doc.txt");
        fs::write(&path, "x").unwrap();
        let cap = service
            .capture(
                Capture::new(
                    "doc.txt".into(),
                    b"```sql\nSELECT 1;\n```".to_vec(),
                    "Saved".into(),
                    Some(path),
                    Some("grant".into()),
                )
                .unwrap(),
            )
            .unwrap();
        let r = Request {
            capture: cap.id,
            block: Some(cap.blocks[0].id.clone()),
            root: None,
            database: None,
            tool: None,
            pandoc: None,
            mode: Mode::SqliteRead,
            toc: false,
        };
        assert!(service.prepare(r.clone()).is_err());
        service.enable(Kind::Sqlite, true);
        assert!(service.prepare(r).is_err());
        assert!(service.take("forged").is_err());
        assert!(!Powers::new(t.path().into(), vec![]).enabled(Kind::Sqlite));
    }
}
