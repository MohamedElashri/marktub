//! Durable app records are separate from disposable cache and document files.
use crate::files::FileError;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub const STORE_QUOTA: u64 = 512 * 1024 * 1024;
fn unused_source_proof(folder: &Path, id: &str) -> bool {
    let Ok(raw) = read(&folder.join("unused-source.json"), 16384) else {
        return false;
    };
    let Ok(proof) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(base) = proof["baseHash"].as_str() else {
        return false;
    };
    if proof["schema"] != 1 || proof["id"] != id {
        return false;
    }
    let Ok(journal) = read(&folder.join("journal.jsonl"), 65536) else {
        return false;
    };
    use sha2::{Digest, Sha256};
    crate::document::journal::is_untouched_base_header(&journal, id, base)
        && proof["journalHash"] == format!("{:x}", Sha256::digest(journal))
}
pub fn sync(file: &File) -> std::io::Result<()> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
pub fn directory(path: &Path) -> Result<(), FileError> {
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.permissions().mode() & 0o077 != 0 {
        return Err(FileError::new(
            "private-storage",
            "Private storage has unsafe ownership or permissions.",
        ));
    }
    Ok(())
}
pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>, FileError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(FileError::new(
            "capacity",
            "Stored record exceeds its resource budget.",
        ));
    }
    let mut bytes = vec![];
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(FileError::new(
            "capacity",
            "Stored record grew beyond its budget.",
        ));
    }
    Ok(bytes)
}
#[cfg(test)]
thread_local! {static FAIL_PREFIX: std::cell::RefCell<Option<String>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) fn fail_next(prefix: &str) {
    FAIL_PREFIX.with(|p| *p.borrow_mut() = Some(prefix.into()));
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    #[cfg(test)]
    if FAIL_PREFIX.with(|slot| {
        let mut value = slot.borrow_mut();
        if value.as_ref().is_some_and(|prefix| {
            path.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        }) {
            value.take();
            true
        } else {
            false
        }
    }) {
        return Err(std::io::Error::from(std::io::ErrorKind::StorageFull).into());
    }

    let parent = path
        .parent()
        .ok_or_else(|| FileError::new("state", "Storage location unavailable."))?;
    directory(parent)?;
    let temp = parent.join(format!("{}.pending", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        file.write_all(bytes)?;
        sync(&file)?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok::<_, FileError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(crate) fn usage(path: &Path) -> Result<u64, FileError> {
    let mut bytes = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        // Other app services atomically replace/remove their private records.
        // A vanished entry contributes no live bytes; every other error remains
        // a refusal, including unexpected symlinks.
        let m = match fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if m.file_type().is_symlink() {
            return Err(FileError::new(
                "private-storage",
                "Unexpected symlink in private storage.",
            ));
        }
        bytes = bytes.saturating_add(if m.is_dir() {
            usage(&entry.path())?
        } else {
            m.len()
        });
        if bytes > STORE_QUOTA {
            return Ok(bytes);
        }
    }
    Ok(bytes)
}
struct StoreLease(File);
impl AsRawFd for StoreLease {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}
impl Drop for StoreLease {
    fn drop(&mut self) {
        // Spawned children can briefly inherit this open description before exec.
        // End the sole logical owner's lease without unlinking the lock file.
        unsafe { libc::flock(self.as_raw_fd(), libc::LOCK_UN) };
    }
}
pub struct PrivateStore {
    pub root: PathBuf,
    _lease: StoreLease,
}
impl PrivateStore {
    pub fn open(root: &Path) -> Result<Self, FileError> {
        directory(root)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("store.lock"))?;
        if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(FileError::new(
                "storage-in-use",
                "Another Marktub instance owns this private store. Close it before editing here.",
            ));
        }
        let lease = StoreLease(lease);
        directory(&root.join("recovery"))?;
        directory(&root.join("cache"))?;
        let store = Self {
            root: fs::canonicalize(root)?,
            _lease: lease,
        };
        if read(&root.join("cache/format"), 16).ok().as_deref() != Some(b"1") {
            store.reset_cache()?;
            atomic(&root.join("cache/format"), b"1")?;
        }
        for entry in fs::read_dir(root.join("cache"))? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(id) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_prefix("unused-source-"))
                    .map(str::to_owned)
                && Uuid::parse_str(&id).is_ok()
                && unused_source_proof(&entry.path(), &id)
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        Ok(store)
    }
    pub(crate) fn retire_unused_source(&self, id: &str, expected: &str) -> Result<(), FileError> {
        if Uuid::parse_str(id).is_err() {
            return Err(FileError::new("state", "Invalid source cache identity."));
        }
        let folder = self.recovery().join(id);
        let journal = read(&folder.join("journal.jsonl"), 65536)?;
        if !crate::document::journal::is_untouched_base_header(&journal, id, expected) {
            return Err(FileError::new("state", "Source history is not disposable."));
        }
        let cache = self.root.join("cache");
        directory(&cache)?;
        let destination = cache.join(format!("unused-source-{id}"));
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(FileError::new(
                    "exists",
                    "Source cache slot is occupied; recovery retained.",
                ));
            }
        }
        use sha2::{Digest, Sha256};
        let proof = serde_json::json!({"schema":1,"id":id,"baseHash":expected,"journalHash":format!("{:x}",Sha256::digest(&journal))});
        atomic(
            &folder.join("unused-source.json"),
            &serde_json::to_vec(&proof)
                .map_err(|_| FileError::new("state", "Cannot encode source retirement proof."))?,
        )?;
        fs::rename(&folder, &destination)?;
        File::open(self.recovery())?.sync_all()?;
        File::open(&cache)?.sync_all()?;
        fs::remove_dir_all(&destination)?;
        File::open(&cache)?.sync_all()?;
        Ok(())
    }
    pub fn recovery(&self) -> PathBuf {
        self.root.join("recovery")
    }
    pub fn reserve(&self, bytes: u64) -> Result<(), FileError> {
        if usage(&self.root)?.saturating_add(bytes) > STORE_QUOTA {
            return Err(FileError::new(
                "quota",
                "Recovery storage is full. Save an independent copy or explicitly discard an old recovery; unsaved text is never auto-deleted.",
            ));
        }
        Ok(())
    }
    pub fn reset_cache(&self) -> Result<(), FileError> {
        fs::remove_dir_all(self.root.join("cache"))?;
        directory(&self.root.join("cache"))
    }
}

#[cfg(test)]
mod lease_lifetime_tests {
    use super::*;
    use std::os::fd::FromRawFd;
    #[test]
    fn duplicated_description_cannot_extend_dropped_owner_or_unlock_next_owner() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("private");
        let owner = PrivateStore::open(&root).unwrap();
        // dup and fork share the same open file description and flock lifetime.
        let fd = unsafe { libc::dup(owner._lease.as_raw_fd()) };
        assert!(fd >= 0);
        let inherited = unsafe { File::from_raw_fd(fd) };
        assert!(matches!(PrivateStore::open(&root), Err(e) if e.code == "storage-in-use"));
        drop(owner);
        let next =
            PrivateStore::open(&root).expect("A duplicate FD is not a surviving store owner");
        assert!(matches!(PrivateStore::open(&root), Err(e) if e.code == "storage-in-use"));
        drop(inherited);
        assert!(matches!(PrivateStore::open(&root), Err(e) if e.code == "storage-in-use"));
        drop(next);
        assert!(PrivateStore::open(&root).is_ok());
    }
}
