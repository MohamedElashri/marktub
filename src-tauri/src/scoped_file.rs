//! Read-only native grants. Paths never enter this API from JavaScript.
use crate::files::FileError;
use sha2::{Digest, Sha256};
use std::{
    ffi::{CString, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};
use uuid::Uuid;

// A pinned parent directory plus a single filename, never an unchecked relative path.
pub struct ScopedFile {
    parent: File,
    parent_path: PathBuf,
    name: OsString,
    selected: PathBuf,
    canonical: PathBuf,
    new_destination: bool,
}

pub(crate) fn path_key(path: &Path) -> String {
    format!("{:x}", Sha256::digest(path.as_os_str().as_bytes()))
}

fn changed() -> FileError {
    FileError::new(
        "changed",
        "The selected file changed while capturing its snapshot. Open it again.",
    )
}

fn identity(a: &Metadata, b: &Metadata) -> bool {
    data_identity(a, b) && a.ctime() == b.ctime() && a.ctime_nsec() == b.ctime_nsec()
}

fn data_identity(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
}

#[cfg(test)]
thread_local! {
    static AFTER_COPY: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_SELECTION: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

impl ScopedFile {
    // Only called after a library directory capability has been validated.
    pub(crate) fn from_parent(
        parent: File,
        parent_path: PathBuf,
        name: OsString,
    ) -> Result<Self, FileError> {
        if Path::new(&name).components().count() != 1 || name == "." || name == ".." {
            return Err(changed());
        }
        let canonical = parent_path.join(&name);
        let scoped = Self {
            parent,
            parent_path,
            name,
            selected: canonical.clone(),
            canonical,
            new_destination: false,
        };
        scoped.validate_parent()?;
        scoped.open()?;
        Ok(scoped)
    }

    pub fn selected(canonical: &Path) -> Result<Self, FileError> {
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(canonical.parent().ok_or_else(changed)?)?;
        let name = canonical.file_name().ok_or_else(changed)?.to_owned();
        let scoped = Self {
            parent,
            parent_path: canonical.parent().ok_or_else(changed)?.to_owned(),
            name,
            selected: canonical.to_owned(),
            canonical: canonical.to_owned(),
            new_destination: false,
        };
        // Hold the selected node while checking it through the pinned parent.
        // This prevents inode reuse; metadata version belongs to snapshot's
        // strict two-hash checks, not this node-authorization comparison.
        let selected_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(canonical)?;
        let selected = selected_file.metadata()?;
        #[cfg(test)]
        AFTER_SELECTION.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook();
            }
        });
        let pinned = scoped.open()?.metadata()?;
        if (selected.dev(), selected.ino()) != (pinned.dev(), pinned.ino()) {
            return Err(changed());
        }
        drop(selected_file);
        Ok(scoped)
    }

    pub fn authorize_selection(mut self, selected: &Path) -> Self {
        self.selected = selected.to_owned();
        self
    }
    pub fn canonical_path(&self) -> &Path {
        &self.canonical
    }
    pub fn destination(path: &Path) -> Result<Self, FileError> {
        let parent_path = fs::canonicalize(path.parent().ok_or_else(changed)?)?;
        let name = path.file_name().ok_or_else(changed)?.to_owned();
        let canonical = parent_path.join(&name);
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&parent_path)?;
        Ok(Self {
            parent,
            parent_path,
            name,
            selected: path.to_owned(),
            canonical,
            new_destination: true,
        })
    }
    pub fn validate_selection(&self) -> Result<(), FileError> {
        self.validate_parent()?;
        if self.new_destination {
            if fs::canonicalize(self.selected.parent().ok_or_else(changed)?)? != self.parent_path {
                return Err(changed());
            }
        } else if fs::canonicalize(&self.selected).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileError::new(
                    "missing",
                    "The selected path is missing. Use Save As to choose a new destination.",
                )
            } else {
                changed()
            }
        })? != self.canonical
        {
            return Err(changed());
        }
        Ok(())
    }
    pub fn assert_writable(&self) -> Result<(), FileError> {
        self.validate_selection()?;
        let name = self.filename()?;
        let fd = unsafe {
            libc::openat(
                self.parent_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(FileError::new(
                "readonly",
                "The target does not grant write access. Use Save As.",
            ));
        }
        drop(unsafe { File::from_raw_fd(fd) });
        Ok(())
    }
    pub fn parent_fd(&self) -> std::os::fd::RawFd {
        self.parent.as_raw_fd()
    }
    pub fn filename(&self) -> Result<CString, FileError> {
        CString::new(self.name.as_bytes()).map_err(|_| changed())
    }
    pub fn sync_parent(&self) -> Result<(), FileError> {
        self.parent.sync_all()?;
        Ok(())
    }
    fn validate_parent(&self) -> Result<(), FileError> {
        let parent = fs::symlink_metadata(&self.parent_path).map_err(|_| changed())?;
        let pinned = self.parent.metadata()?;
        if !parent.is_dir() || parent.dev() != pinned.dev() || parent.ino() != pinned.ino() {
            return Err(changed());
        }
        Ok(())
    }
    pub fn open(&self) -> Result<File, FileError> {
        self.validate_parent()?;
        let name = CString::new(self.name.as_bytes()).map_err(|_| changed())?;
        // SAFETY: the directory FD remains alive, name is one NUL-terminated filename,
        // and a successful owned FD is transferred exactly once into File.
        let fd = unsafe {
            libc::openat(
                self.parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::ENOENT) => FileError::new("missing", "The selected file is missing."),
                Some(libc::ELOOP) => changed(),
                _ => error.into(),
            });
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            return Err(FileError::new(
                "not-file",
                "Select a regular Markdown or text file.",
            ));
        }
        Ok(file)
    }

    pub fn renamed_name(&self, original: (u64, u64)) -> Result<Option<String>, FileError> {
        self.validate_parent()?;
        let mut names = vec![];
        for (n, e) in fs::read_dir(&self.parent_path)?.enumerate() {
            if n >= 4096 {
                return Err(FileError::new(
                    "capacity",
                    "Rename recognition admits 4,096 sibling entries.",
                ));
            }
            let e = e?;
            let m = fs::symlink_metadata(e.path())?;
            if m.is_file() && m.nlink() == 1 && (m.dev(), m.ino()) == original {
                names.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        self.validate_parent()?;
        Ok(if names.len() == 1 { names.pop() } else { None })
    }
    /// Copy to an unlinked mode-0600 file. The original FD is not a snapshot:
    /// external in-place writes must never change bytes already granted to the UI.
    pub fn snapshot(&self) -> Result<(File, String, u64), FileError> {
        self.snapshot_bounded(u64::MAX)
    }
    pub fn snapshot_bounded(&self, limit: u64) -> Result<(File, String, u64), FileError> {
        let mut source = self.open()?;
        let before = source.metadata()?;
        if before.len() > limit {
            return Err(FileError::new(
                "capacity",
                "The requested snapshot exceeds its resource budget; no data was changed.",
            ));
        }
        let path = std::env::temp_dir().join(format!("marktub-{}", Uuid::new_v4()));
        let mut snapshot = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        // No named snapshot survives an app crash; no public path can reopen it.
        if let Err(error) = fs::remove_file(&path) {
            drop(snapshot);
            let _ = fs::remove_file(path);
            return Err(error.into());
        }
        // OS document-open bookkeeping can change xattrs/ctime during a read.
        // Retry only a ctime-only race, at most twice, on this same original FD.
        // Every successful attempt still requires both full hashes and all strict
        // metadata/path checks. Save fingerprinting never uses this retry.
        for attempt in 0..3 {
            let start = source.metadata()?;
            if !data_identity(&before, &start) {
                return Err(changed());
            }
            snapshot.set_len(0)?;
            snapshot.rewind()?;
            let result = (|| {
                let (hash, length) = fingerprint(&mut source, Some(&mut snapshot))?;
                #[cfg(test)]
                AFTER_COPY.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook();
                    }
                });
                if !identity(&start, &source.metadata()?) {
                    return Err(changed());
                }
                // Re-open only for validation, never to adopt another file.
                let mut current = self.open()?;
                if !identity(&start, &current.metadata()?)
                    || fingerprint(&mut current, None)?.0 != hash
                    || !identity(&start, &current.metadata()?)
                {
                    return Err(changed());
                }
                Ok((hash, length))
            })();
            match result {
                Ok((hash, length)) => {
                    snapshot.rewind()?;
                    return Ok((snapshot, hash, length));
                }
                Err(error) => {
                    let after = source.metadata()?;
                    if attempt == 2
                        || error.code != "changed"
                        || !data_identity(&start, &after)
                        || identity(&start, &after)
                    {
                        return Err(error);
                    }
                }
            }
        }
        unreachable!("bounded capture attempt returns on its last iteration")
    }
}

pub fn fingerprint(
    file: &mut File,
    mut copy: Option<&mut File>,
) -> Result<(String, u64), FileError> {
    let before = file.metadata()?;
    file.rewind()?;
    let mut hash = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0; 64 * 1024];
    // A continuously growing file must not keep capture running indefinitely.
    let mut input = Read::by_ref(file).take(before.len().saturating_add(1));
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        if let Some(output) = copy.as_mut() {
            output.write_all(&buffer[..n])?;
        }
        length += n as u64;
    }
    if !identity(&before, &file.metadata()?) || length != before.len() {
        return Err(changed());
    }
    Ok((format!("{:x}", hash.finalize()), length))
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::{cell::Cell, os::unix::fs::PermissionsExt, rc::Rc};
    struct Hook;
    impl Drop for Hook {
        fn drop(&mut self) {
            AFTER_COPY.with(|h| *h.borrow_mut() = None);
            AFTER_SELECTION.with(|h| *h.borrow_mut() = None);
        }
    }
    fn hook(f: impl FnMut() + 'static) -> Hook {
        AFTER_COPY.with(|h| *h.borrow_mut() = Some(Box::new(f)));
        Hook
    }
    #[test]
    fn selection_pins_node_across_metadata_change_but_never_a_replacement() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("source");
        fs::write(&p, b"original").unwrap();
        let path = p.clone();
        AFTER_SELECTION.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap()
            }))
        });
        let guard = Hook;
        let scoped = ScopedFile::selected(&p).unwrap();
        drop(guard);
        assert_eq!(scoped.snapshot().unwrap().2, 8);
        let path = p.clone();
        AFTER_SELECTION.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                let next = path.with_extension("next");
                fs::write(&next, b"foreign!").unwrap();
                fs::rename(next, &path).unwrap();
            }))
        });
        let _guard = Hook;
        assert!(matches!(
            ScopedFile::selected(&p),
            Err(FileError {
                code: "changed",
                ..
            })
        ));
        assert_eq!(fs::read(p).unwrap(), b"foreign!");
    }
    #[test]
    fn metadata_race_retries_same_file_with_exact_bytes_and_private_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("source");
        let bytes = b"\xef\xbb\xbfa\r\nb\nc\r";
        fs::write(&p, bytes).unwrap();
        let scoped = ScopedFile::selected(&p).unwrap();
        let count = Rc::new(Cell::new(0));
        let counter = count.clone();
        let path = p.clone();
        let _hook = hook(move || {
            counter.set(counter.get() + 1);
            if counter.get() == 1 {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
            }
        });
        let (mut f, hash, len) = scoped.snapshot().unwrap();
        let mut copy = vec![];
        f.read_to_end(&mut copy).unwrap();
        assert_eq!(copy, bytes);
        assert_eq!(len, bytes.len() as u64);
        assert_eq!(hash, format!("{:x}", Sha256::digest(bytes)));
        assert_eq!(count.get(), 2);
        assert_eq!(f.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    #[test]
    fn replacement_is_never_adopted_by_a_capture_retry() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("source");
        fs::write(&p, b"original").unwrap();
        let scoped = ScopedFile::selected(&p).unwrap();
        let path = p.clone();
        let mut replaced = false;
        let _hook = hook(move || {
            if !replaced {
                replaced = true;
                let next = path.with_extension("next");
                fs::write(&next, b"foreign!").unwrap();
                fs::rename(next, &path).unwrap();
            }
        });
        assert_eq!(scoped.snapshot().unwrap_err().code, "changed");
        assert_eq!(fs::read(p).unwrap(), b"foreign!");
    }
    #[test]
    fn content_changes_and_continuous_metadata_changes_still_refuse() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("source");
        fs::write(&p, b"old").unwrap();
        let scoped = ScopedFile::selected(&p).unwrap();
        let path = p.clone();
        {
            let _hook = hook(move || {
                fs::write(&path, b"new content").unwrap();
            });
            assert_eq!(scoped.snapshot().unwrap_err().code, "changed");
        }
        let scoped = ScopedFile::selected(&p).unwrap();
        let path = p.clone();
        let count = Rc::new(Cell::new(0));
        let counter = count.clone();
        let _hook = hook(move || {
            counter.set(counter.get() + 1);
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if counter.get() % 2 == 1 { 0o640 } else { 0o600 }),
            )
            .unwrap();
        });
        assert_eq!(scoped.snapshot().unwrap_err().code, "changed");
        assert_eq!(count.get(), 3);
    }
}
