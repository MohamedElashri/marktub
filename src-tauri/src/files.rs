use crate::scoped_file::{ScopedFile, fingerprint};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub const REVIEW_CAPTURE_LIMIT: u64 = 2 * 1024 * 1024;
pub const MAX_RANGE: usize = 64 * 1024;
const MAX_DOCUMENTS: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileError {
    pub code: &'static str,
    pub message: String,
}

impl FileError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for FileError {
    fn from(error: std::io::Error) -> Self {
        // Error strings may contain private paths; UI gets only OS error category.
        Self::new("io", format!("File operation failed ({:?}).", error.kind()))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub byte_length: u64,
    pub revision: String,
    pub watching: bool,
    pub location: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
    pub text: String,
}

pub struct Document {
    scoped: Arc<ScopedFile>,
    can_save: bool,
    file: Mutex<File>,
    revision: String,
    length: u64,
    identity: (u64, u64),
    // Identity only: retain the selected inode so it cannot be recycled while
    // recognizing renames. All displayed bytes still come from the private copy.
    _identity_file: File,
    // Retain the watch registration for the lifetime of the grant.
    _watcher: Option<RecommendedWatcher>,
}

#[derive(Default)]
pub struct Documents(Mutex<HashMap<String, Arc<Document>>>);

impl Documents {
    // Deliberately not a Tauri command: only native selection/app-owned examples grant paths.
    #[cfg(test)]
    pub fn grant(
        &self,
        path: &Path,
        invalidated: impl Fn(String) + Send + 'static,
    ) -> Result<Descriptor, FileError> {
        self.grant_with_policy(path, true, invalidated)
    }
    pub fn grant_with_policy(
        &self,
        selected: &Path,
        can_save: bool,
        invalidated: impl Fn(String) + Send + 'static,
    ) -> Result<Descriptor, FileError> {
        let path = fs::canonicalize(selected)?;
        let scoped = ScopedFile::selected(&path)?.authorize_selection(selected);
        self.grant_scoped(scoped, can_save, invalidated)
    }
    pub(crate) fn grant_scoped(
        &self,
        scoped: ScopedFile,
        can_save: bool,
        invalidated: impl Fn(String) + Send + 'static,
    ) -> Result<Descriptor, FileError> {
        let path = scoped.canonical_path().to_owned();
        let identity_file = scoped.open()?;
        let original = identity_file.metadata()?;
        let (file, revision, length) = scoped.snapshot()?;
        let after = scoped.open()?.metadata()?;
        if (original.dev(), original.ino()) != (after.dev(), after.ino()) {
            return Err(FileError::new(
                "changed",
                "File identity changed during selection.",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let watch_id = id.clone();
        let watched_path = path.clone();
        let watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
                Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => {}
                Ok(event) if event.paths.iter().any(|p| p == &watched_path) => {
                    invalidated(watch_id.clone())
                }
                Err(_) => invalidated(watch_id.clone()),
                _ => {}
            })
            .ok()
            .and_then(|mut watcher| {
                watcher
                    .watch(path.parent()?, RecursiveMode::NonRecursive)
                    .ok()?;
                Some(watcher)
            });
        let descriptor = Descriptor {
            id: id.clone(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            byte_length: length,
            revision: revision.clone(),
            watching: watcher.is_some(),
            location: crate::scoped_file::path_key(&path),
        };
        let mut documents = self
            .0
            .lock()
            .map_err(|_| FileError::new("state", "Document service unavailable."))?;
        if documents.len() >= MAX_DOCUMENTS {
            return Err(FileError::new(
                "capacity",
                "Close a document before opening another.",
            ));
        }
        documents.insert(
            id,
            Arc::new(Document {
                scoped: Arc::new(scoped),
                can_save,
                file: Mutex::new(file),
                revision,
                length,
                identity: (original.dev(), original.ino()),
                _identity_file: identity_file,
                _watcher: watcher,
            }),
        );
        Ok(descriptor)
    }

    fn get(&self, id: &str) -> Result<Arc<Document>, FileError> {
        self.0
            .lock()
            .map_err(|_| FileError::new("state", "Document service unavailable."))?
            .get(id)
            .cloned()
            .ok_or_else(|| {
                FileError::new("unauthorized", "This document handle is not authorized.")
            })
    }

    pub fn editing_base(
        &self,
        id: &str,
    ) -> Result<(Arc<ScopedFile>, Vec<u8>, bool, String), FileError> {
        let doc = self.get(id)?;
        if doc.length > 128 * 1024 * 1024 {
            return Err(FileError::new(
                "capacity",
                "This document exceeds the current Source editing budget. You can keep reading it in ranges.",
            ));
        }
        let mut file = doc
            .file
            .lock()
            .map_err(|_| FileError::new("state", "Document unavailable."))?;
        file.rewind()?;
        let mut bytes = Vec::with_capacity(doc.length as usize + 1);
        Read::by_ref(&mut *file)
            .take(doc.length + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != doc.length {
            return Err(FileError::new(
                "changed",
                "The private reading snapshot changed. Reopen the selected document; no Source session was created.",
            ));
        }
        Ok((
            doc.scoped.clone(),
            bytes,
            doc.can_save,
            doc.scoped
                .canonical_path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ))
    }
    pub fn byte_length(&self, id: &str) -> Result<u64, FileError> {
        Ok(self.get(id)?.length)
    }
    pub fn paths(&self) -> Result<Vec<(String, std::path::PathBuf)>, FileError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| FileError::new("state", "Documents unavailable."))?
            .iter()
            .map(|(id, d)| (id.clone(), d.scoped.canonical_path().to_owned()))
            .collect())
    }
    pub fn scoped(&self, id: &str) -> Result<Arc<ScopedFile>, FileError> {
        Ok(self.get(id)?.scoped.clone())
    }
    pub fn read_range(&self, id: &str, start: u64, limit: usize) -> Result<SourceRange, FileError> {
        let document = self.get(id)?;
        if limit == 0 || limit > MAX_RANGE || start > document.length {
            return Err(FileError::new(
                "range",
                "Requested source range is outside the read budget.",
            ));
        }
        let mut file = document
            .file
            .lock()
            .map_err(|_| FileError::new("state", "Document unavailable."))?;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![0; limit.min((document.length - start) as usize)];
        file.read_exact(&mut bytes)?;
        // Align page edges to UTF-8 scalar boundaries, retaining incomplete trailing bytes for the next page.
        let mut skip = 0;
        if start > 0 {
            while skip < bytes.len() && bytes[skip] & 0xc0 == 0x80 {
                skip += 1;
            }
        }
        let mut end = bytes.len();
        if start + (end as u64) < document.length {
            for trim in 0..=3.min(end.saturating_sub(skip)) {
                if std::str::from_utf8(&bytes[skip..end - trim]).is_ok() {
                    end -= trim;
                    break;
                }
            }
        }
        let text = std::str::from_utf8(&bytes[skip..end]).map_err(|_| {
            FileError::new(
                "encoding",
                "This range is not valid UTF-8. Conversion is not available.",
            )
        })?;
        if text.contains('\0') {
            return Err(FileError::new(
                "binary",
                "Binary content cannot be previewed as Markdown.",
            ));
        }
        Ok(SourceRange {
            start: start + skip as u64,
            end: start + end as u64,
            total: document.length,
            text: text.to_owned(),
        })
    }

    pub fn status(&self, id: &str) -> Result<&'static str, FileError> {
        let document = self.get(id)?;
        let mut file = match document.scoped.open() {
            Ok(file) => file,
            Err(error) if error.code == "missing" => return Ok("missing"),
            Err(error) if error.code == "changed" => return Ok("changed"),
            Err(error) => return Err(error),
        };
        Ok(if fingerprint(&mut file, None)?.0 == document.revision {
            "unchanged"
        } else {
            "changed"
        })
    }

    pub fn review_identity(&self, id: &str) -> Result<(String, String), FileError> {
        let d = self.get(id)?;
        if !d.can_save {
            return Err(FileError::new(
                "review",
                "Save As before reviewing app-owned resources.",
            ));
        }
        Ok((
            crate::scoped_file::path_key(d.scoped.canonical_path()),
            d.revision.clone(),
        ))
    }
    pub fn power_capture(
        &self,
        id: &str,
    ) -> Result<(String, Vec<u8>, Option<std::path::PathBuf>), FileError> {
        let d = self.get(id)?;
        if d.length > 2 * 1024 * 1024 {
            return Err(FileError::new(
                "capacity",
                "Export/run capture admits 2 MiB; Read is kept.",
            ));
        }
        let mut f = d
            .file
            .lock()
            .map_err(|_| FileError::new("state", "Saved capture unavailable."))?;
        f.rewind()?;
        let mut bytes = Vec::with_capacity(d.length as usize);
        Read::by_ref(&mut *f)
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        Ok((
            d.scoped
                .canonical_path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            bytes,
            d.can_save.then(|| d.scoped.canonical_path().to_owned()),
        ))
    }
    pub fn review_capture(
        &self,
        id: &str,
        disk: bool,
    ) -> Result<(std::path::PathBuf, Vec<u8>), FileError> {
        let d = self.get(id)?;
        if !d.can_save {
            return Err(FileError::new(
                "review",
                "Save As to a user-selected document before enabling Review.",
            ));
        }
        if !disk && d.length > REVIEW_CAPTURE_LIMIT {
            return Err(FileError::new(
                "review",
                "Review admits UTF-8 saved captures up to 2 MiB.",
            ));
        }
        let bytes = if disk {
            let (mut f, _, _) = d.scoped.snapshot_bounded(REVIEW_CAPTURE_LIMIT)?;
            let mut b = vec![];
            f.read_to_end(&mut b)?;
            b
        } else {
            let mut f = d
                .file
                .lock()
                .map_err(|_| FileError::new("state", "Document unavailable."))?;
            f.rewind()?;
            let mut b = vec![];
            Read::by_ref(&mut *f)
                .take(REVIEW_CAPTURE_LIMIT + 1)
                .read_to_end(&mut b)?;
            b
        };
        Ok((d.scoped.canonical_path().to_owned(), bytes))
    }
    pub fn renamed_name(&self, id: &str) -> Result<Option<String>, FileError> {
        let d = self.get(id)?;
        if self.status(id)? != "missing" {
            return Ok(None);
        }
        d.scoped.renamed_name(d.identity)
    }
    pub fn release(&self, id: &str) -> Result<(), FileError> {
        self.get(id)?;
        self.0
            .lock()
            .map_err(|_| FileError::new("state", "Document service unavailable."))?
            .remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn unauthorized_paths_and_handles_never_read() {
        let docs = Documents::default();
        for id in ["/etc/passwd", "../plan.md", "", "guessed-handle"] {
            assert_eq!(docs.read_range(id, 0, 64).unwrap_err().code, "unauthorized");
        }
    }

    #[test]
    fn ranges_are_bounded_and_utf8_aligned() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "a🦀文z").unwrap();
        let docs = Documents::default();
        let doc = docs.grant(file.path(), |_| {}).unwrap();
        let first = docs.read_range(&doc.id, 0, 4).unwrap();
        assert_eq!(first.text, "a");
        let next = docs.read_range(&doc.id, first.end, 7).unwrap();
        assert_eq!(next.text, "🦀文");
        assert_eq!(
            docs.read_range(&doc.id, 0, MAX_RANGE + 1).unwrap_err().code,
            "range"
        );
        docs.release(&doc.id).unwrap();
        assert_eq!(
            docs.read_range(&doc.id, 0, 1).unwrap_err().code,
            "unauthorized"
        );
    }

    #[test]
    fn atomic_replacement_invalidates_without_changing_captured_handle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan.md");
        fs::write(&path, "before").unwrap();
        let docs = Documents::default();
        let (events, changes) = std::sync::mpsc::channel();
        let doc = docs
            .grant(&path, move |id| {
                let _ = events.send(id);
            })
            .unwrap();
        let replacement = directory.path().join("new.md");
        fs::write(&replacement, "after").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(
            doc.watching,
            "native watcher must register in the test environment"
        );
        assert_eq!(
            changes
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            doc.id
        );
        assert_eq!(docs.status(&doc.id).unwrap(), "changed");
        assert_eq!(docs.read_range(&doc.id, 0, 64).unwrap().text, "before");
    }

    #[test]
    fn in_place_changes_leave_snapshot_immutable() {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), "before").unwrap();
        let docs = Documents::default();
        let doc = docs.grant(file.path(), |_| {}).unwrap();
        fs::write(file.path(), "longer and changed").unwrap();
        assert_eq!(docs.read_range(&doc.id, 0, 64).unwrap().text, "before");
        assert_eq!(docs.status(&doc.id).unwrap(), "changed");
    }

    #[test]
    fn symlink_swap_and_missing_file_never_retarget_a_grant() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selected.txt");
        let outside = directory.path().join("secret.txt");
        fs::write(&path, "authorized").unwrap();
        fs::write(&outside, "not authorized").unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(docs.status(&doc.id).unwrap(), "missing");
        symlink(&outside, &path).unwrap();
        assert_eq!(docs.status(&doc.id).unwrap(), "changed");
        assert_eq!(docs.read_range(&doc.id, 0, 64).unwrap().text, "authorized");
    }

    #[test]
    fn replacing_parent_directory_does_not_authorize_new_contents() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("selected");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("document.txt");
        fs::write(&path, "authorized").unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        fs::rename(&parent, directory.path().join("moved")).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::write(&path, "not authorized").unwrap();
        assert_eq!(docs.status(&doc.id).unwrap(), "changed");
        assert_eq!(docs.read_range(&doc.id, 0, 64).unwrap().text, "authorized");
    }

    #[test]
    fn canonical_selected_symlink_reads_its_original_target_only() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selected.txt");
        let original = directory.path().join("original.txt");
        let other = directory.path().join("other.txt");
        fs::write(&original, "original").unwrap();
        fs::write(&other, "other").unwrap();
        symlink(&original, &path).unwrap();
        let docs = Documents::default();
        let doc = docs.grant(&path, |_| {}).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&other, &path).unwrap();
        assert_eq!(docs.read_range(&doc.id, 0, 64).unwrap().text, "original");
        // Selection authorized the resolved target for reading, not the alias or its folder.
        assert_eq!(docs.status(&doc.id).unwrap(), "unchanged");
    }
    #[test]
    fn source_base_refuses_private_snapshot_length_changes_without_reading_the_selected_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selected.txt");
        let original = b"original\r\nbytes";
        fs::write(&path, original).unwrap();
        let docs = Documents::default();
        let descriptor = docs.grant(&path, |_| {}).unwrap();
        let doc = docs.get(&descriptor.id).unwrap();
        doc.file.lock().unwrap().set_len(doc.length + 1024).unwrap();
        let error = docs.editing_base(&descriptor.id).err().unwrap();
        assert_eq!(error.code, "changed");
        assert_eq!(fs::read(&path).unwrap(), original);
        doc.file.lock().unwrap().set_len(1).unwrap();
        assert_eq!(
            docs.editing_base(&descriptor.id).err().unwrap().code,
            "changed"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn review_saved_and_disk_captures_keep_original_grants_and_recognize_renames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        fs::write(&path, "Old\r\n文 🦀").unwrap();
        let docs = Documents::default();
        let old = docs.grant(&path, |_| {}).unwrap();
        let replacement = dir.path().join("next.txt");
        fs::write(&replacement, "New\n").unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(
            docs.review_capture(&old.id, false).unwrap().1,
            "Old\r\n文 🦀".as_bytes()
        );
        assert_eq!(docs.review_capture(&old.id, true).unwrap().1, b"New\n");
        let selected = docs.grant(&path, |_| {}).unwrap();
        fs::rename(&path, dir.path().join("renamed.txt")).unwrap();
        assert_eq!(
            docs.renamed_name(&selected.id).unwrap(),
            Some("renamed.txt".into())
        );
        assert!(docs.review_capture(&selected.id, true).is_err());
        assert_eq!(
            docs.review_capture(&selected.id, false).unwrap().1,
            b"New\n"
        );
        docs.release(&selected.id).unwrap();
        assert!(docs.review_identity(&selected.id).is_err());
    }
    #[test]
    fn review_capture_refuses_large_and_app_owned_files_without_revoking_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        fs::write(&path, vec![b'a'; REVIEW_CAPTURE_LIMIT as usize + 1]).unwrap();
        let docs = Documents::default();
        let large = docs.grant(&path, |_| {}).unwrap();
        assert!(docs.review_capture(&large.id, false).is_err());
        assert_eq!(
            docs.read_range(&large.id, 0, 16).unwrap().text,
            "a".repeat(16)
        );
        let path = dir.path().join("example.txt");
        fs::write(&path, "Example").unwrap();
        let resource = docs.grant_with_policy(&path, false, |_| {}).unwrap();
        assert!(docs.review_identity(&resource.id).is_err());
        assert!(docs.review_capture(&resource.id, false).is_err());
        assert_eq!(
            docs.read_range(&resource.id, 0, 16).unwrap().text,
            "Example"
        );
    }
}
