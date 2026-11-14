//! Same-directory, handle-relative commits. A late hash check is not a CAS against
//! noncooperating writers; retained recovery inputs and explicit conflicts are essential.
use crate::{
    document::Snapshot,
    files::FileError,
    scoped_file::{ScopedFile, fingerprint},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, Metadata},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mn: i64,
    ctime: i64,
    cn: i64,
}
impl Identity {
    pub fn of(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            mtime: m.mtime(),
            mn: m.mtime_nsec(),
            ctime: m.ctime(),
            cn: m.ctime_nsec(),
        }
    }
}
pub struct Committed {
    pub hash: String,
    pub identity: Identity,
    pub noop: bool,
}
struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.update(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn source_hash(source: &Snapshot) -> Result<String, FileError> {
    let mut hash = HashWriter(Sha256::new());
    source.write_to(&mut hash)?;
    Ok(format!("{:x}", hash.0.finalize()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Written,
    Synced,
    Validated,
    Renamed,
}
struct Temp<'a> {
    target: &'a ScopedFile,
    name: CString,
    file: File,
}
impl Drop for Temp<'_> {
    fn drop(&mut self) {
        unsafe {
            libc::unlinkat(self.target.parent_fd(), self.name.as_ptr(), 0);
        }
    }
}
fn temporary(target: &ScopedFile) -> Result<Temp<'_>, FileError> {
    let name = CString::new(format!(".marktub-{}.tmp", Uuid::new_v4())).expect("UUID");
    // SAFETY: pinned directory FD, constant single-component name, exclusive creation.
    let fd = unsafe {
        libc::openat(
            target.parent_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Temp {
        target,
        name,
        file: unsafe { File::from_raw_fd(fd) },
    })
}
fn os_result(code: i32) -> Result<(), FileError> {
    if code < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
pub(crate) fn metadata(source: &File, temp: &File) -> Result<(), FileError> {
    let m = source.metadata()?;
    if m.nlink() != 1 {
        return Err(FileError::new(
            "hardlink",
            "This file has hard links. Use Save As to create an independent file.",
        ));
    }
    if m.mode() & 0o222 == 0 {
        return Err(FileError::new(
            "readonly",
            "The file is read-only. Use Save As.",
        ));
    }
    if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o7000 != 0 {
        return Err(FileError::new(
            "metadata",
            "Ownership or special permission bits require Save As.",
        ));
    }
    os_result(unsafe { libc::fchown(temp.as_raw_fd(), m.uid(), m.gid()) })?;
    os_result(unsafe { libc::fchmod(temp.as_raw_fd(), (m.mode() & 0o777) as libc::mode_t) })?;
    #[cfg(target_os = "macos")]
    os_result(unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            temp.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        )
    })?;
    #[cfg(target_os = "linux")]
    {
        let count = unsafe { libc::flistxattr(source.as_raw_fd(), std::ptr::null_mut(), 0) };
        if count < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if count > 1024 * 1024 {
            return Err(FileError::new(
                "metadata",
                "Extended attributes exceed the metadata budget. Use Save As.",
            ));
        }
        let mut names = vec![0u8; count as usize];
        let read =
            unsafe { libc::flistxattr(source.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
        if read != count {
            return Err(FileError::new(
                "metadata",
                "Extended attributes changed while saving.",
            ));
        }
        for name in names.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let name = CString::new(name)
                .map_err(|_| FileError::new("metadata", "Invalid attribute name."))?;
            let count = unsafe {
                libc::fgetxattr(source.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0)
            };
            if !(0..=1024 * 1024).contains(&count) {
                return Err(FileError::new(
                    "metadata",
                    "Cannot preserve extended attributes. Use Save As.",
                ));
            }
            let mut value = vec![0u8; count as usize];
            if unsafe {
                libc::fgetxattr(
                    source.as_raw_fd(),
                    name.as_ptr(),
                    value.as_mut_ptr().cast(),
                    value.len(),
                )
            } != count
            {
                return Err(FileError::new(
                    "metadata",
                    "Extended attributes changed while saving.",
                ));
            }
            os_result(unsafe {
                libc::fsetxattr(
                    temp.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                )
            })?;
        }
    }
    Ok(())
}

pub fn save(
    target: &ScopedFile,
    source: &Snapshot,
    expected: &str,
    identity: &Identity,
    conflict: impl FnMut(&mut File, &str) -> Result<(), FileError>,
) -> Result<Committed, FileError> {
    save_with(target, source, expected, identity, conflict, |_| Ok(()))
}
fn save_with(
    target: &ScopedFile,
    source: &Snapshot,
    expected: &str,
    identity: &Identity,
    mut conflict: impl FnMut(&mut File, &str) -> Result<(), FileError>,
    mut hook: impl FnMut(Step) -> Result<(), FileError>,
) -> Result<Committed, FileError> {
    target.validate_selection()?;
    let (mut disk, hash, _) = target.snapshot_bounded(160 * 1024 * 1024)?;
    let current = target.open()?;
    let current_id = Identity::of(&current.metadata()?);
    if hash != expected || current_id != *identity {
        conflict(&mut disk, &hash)?;
        return Err(FileError::new(
            "conflict",
            "The disk revision changed. Compare or reload it, keep your edits, or use Save As.",
        ));
    }
    let hash = source_hash(source)?;
    if hash == expected {
        target.sync_parent()?;
        return Ok(Committed {
            hash,
            identity: current_id,
            noop: true,
        });
    }
    target.assert_writable()?;
    let mut temp = temporary(target)?;
    source.write_to(&mut temp.file)?;
    hook(Step::Written)?;
    metadata(&current, &temp.file)?;
    crate::private_store::sync(&temp.file)?;
    hook(Step::Synced)?;
    target.validate_selection()?;
    let (mut latest, late_hash, _) = target.snapshot_bounded(160 * 1024 * 1024)?;
    let late = target.open()?;
    if late_hash != expected || Identity::of(&late.metadata()?) != *identity {
        conflict(&mut latest, &late_hash)?;
        return Err(FileError::new(
            "conflict",
            "The file changed while saving. Pending edits and the disk snapshot were kept.",
        ));
    }
    hook(Step::Validated)?;
    // Last target/parent validation. No portable atomic hash-and-rename CAS exists.
    target.validate_selection()?;
    let name = target.filename()?;
    os_result(unsafe {
        libc::renameat(
            target.parent_fd(),
            temp.name.as_ptr(),
            target.parent_fd(),
            name.as_ptr(),
        )
    })?;
    hook(Step::Renamed).map_err(|_|FileError::new("durability","The file was replaced, but durability is uncertain. Recovery was kept; compare disk before retrying."))?;
    target.sync_parent().map_err(|_| {
        FileError::new(
            "durability",
            "The file was replaced, but directory sync failed. Recovery was kept.",
        )
    })?;
    let mut committed = target.open()?;
    if fingerprint(&mut committed, None)?.0 != hash {
        return Err(FileError::new(
            "conflict",
            "Another writer changed the file after replacement. Recovery was kept.",
        ));
    }
    Ok(Committed {
        hash,
        identity: Identity::of(&committed.metadata()?),
        noop: false,
    })
}
/// Native Save As grants only a new name. linkat publishes without overwriting a
/// file or symlink created after the picker returned. There is no overwrite bridge.
pub fn save_as(target: &ScopedFile, source: &Snapshot) -> Result<Committed, FileError> {
    target.validate_selection()?;
    let mut temp = temporary(target)?;
    source.write_to(&mut temp.file)?;
    crate::private_store::sync(&temp.file)?;
    target.validate_selection()?;
    let name = target.filename()?;
    let result = unsafe {
        libc::linkat(
            target.parent_fd(),
            temp.name.as_ptr(),
            target.parent_fd(),
            name.as_ptr(),
            0,
        )
    };
    if result < 0 {
        let e = std::io::Error::last_os_error();
        return Err(if e.kind() == std::io::ErrorKind::AlreadyExists {
            FileError::new(
                "exists",
                "Choose an unused filename for Save As. Existing files are never replaced by this action.",
            )
        } else {
            e.into()
        });
    }
    os_result(unsafe { libc::unlinkat(target.parent_fd(), temp.name.as_ptr(), 0) })?;
    target.sync_parent().map_err(|_| {
        FileError::new(
            "durability",
            "Save As published the file, but directory sync failed. Recovery was kept.",
        )
    })?;
    let file = target.open()?;
    Ok(Committed {
        hash: source_hash(source)?,
        identity: Identity::of(&file.metadata()?),
        noop: false,
    })
}

pub(crate) fn write_asset(target: &ScopedFile, bytes: &[u8]) -> Result<Committed, FileError> {
    target.validate_selection()?;
    let mut temp = temporary(target)?;
    temp.file.write_all(bytes)?;
    crate::private_store::sync(&temp.file)?;
    target.validate_selection()?;
    let name = target.filename()?;
    let result = unsafe {
        libc::linkat(
            target.parent_fd(),
            temp.name.as_ptr(),
            target.parent_fd(),
            name.as_ptr(),
            0,
        )
    };
    if result < 0 {
        let e = std::io::Error::last_os_error();
        return Err(if e.kind() == std::io::ErrorKind::AlreadyExists {
            FileError::new(
                "exists",
                "Choose an unused filename for Save As. Existing files are never replaced by this action.",
            )
        } else {
            e.into()
        });
    }
    os_result(unsafe { libc::unlinkat(target.parent_fd(), temp.name.as_ptr(), 0) })?;
    target.sync_parent().map_err(|_| {
        FileError::new(
            "durability",
            "Save As published the file, but directory sync failed. Recovery was kept.",
        )
    })?;
    let file = target.open()?;
    Ok(Committed {
        hash: format!("{:x}", Sha256::digest(bytes)),
        identity: Identity::of(&file.metadata()?),
        noop: false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn copied_raster_is_exclusive_and_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.png");
        let target = super::ScopedFile::destination(&path).unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nraw";
        super::write_asset(&target, bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            super::write_asset(&target, b"replace").err().unwrap().code,
            "exists"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    use super::*;
    use crate::document::Provenance;
    use std::io::{Read, Seek};
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    fn target(path: &std::path::Path) -> (ScopedFile, String, Identity) {
        let canonical = fs::canonicalize(path).unwrap();
        let target = ScopedFile::selected(&canonical)
            .unwrap()
            .authorize_selection(path);
        let mut f = target.open().unwrap();
        let h = fingerprint(&mut f, None).unwrap().0;
        let id = Identity::of(&f.metadata().unwrap());
        (target, h, id)
    }
    #[test]
    fn noop_never_replaces_inode_mode_or_timestamps_and_edits_preserve_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        fs::write(&path, b"\xef\xbb\xbfa\r\nb\nc\r").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let (t, h, id) = target(&path);
        let mut source = Provenance::new(&fs::read(&path).unwrap()).unwrap();
        assert!(
            save(&t, &source.snapshot(), &h, &id, |_, _| Ok(()))
                .unwrap()
                .noop
        );
        assert_eq!(Identity::of(&fs::metadata(&path).unwrap()), id);
        source
            .apply(
                "edit",
                &[crate::document::Change {
                    from: 2,
                    to: 3,
                    insert: crate::document::Insert::Text { text: "B".into() },
                }],
            )
            .unwrap();
        let result = save(&t, &source.snapshot(), &h, &id, |_, _| Ok(())).unwrap();
        assert!(!result.noop);
        assert_ne!(result.identity.ino, id.ino);
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfa\r\nB\nc\r");
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o640);
    }
    #[test]
    fn failure_before_replace_keeps_original_and_removes_temps() {
        for step in [Step::Written, Step::Synced, Step::Validated] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("source.txt");
            fs::write(&path, b"before").unwrap();
            let (t, h, id) = target(&path);
            let source = Provenance::new(b"after").unwrap();
            let e = save_with(
                &t,
                &source.snapshot(),
                &h,
                &id,
                |_, _| Ok(()),
                |s| {
                    if s == step {
                        Err(std::io::Error::from(std::io::ErrorKind::StorageFull).into())
                    } else {
                        Ok(())
                    }
                },
            )
            .err()
            .unwrap();
            assert_eq!(e.code, "io");
            assert_eq!(fs::read(&path).unwrap(), b"before");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn late_writer_is_refused_and_captured_while_hash_rename_race_is_explicit() {
        for point in [Step::Synced, Step::Validated] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("source.txt");
            fs::write(&path, b"base").unwrap();
            let (t, h, id) = target(&path);
            let source = Provenance::new(b"pending").unwrap();
            let mut captured = vec![];
            let result = save_with(
                &t,
                &source.snapshot(),
                &h,
                &id,
                |f, _| {
                    f.rewind()?;
                    f.read_to_end(&mut captured)?;
                    Ok(())
                },
                |s| {
                    if s == point {
                        fs::write(&path, b"external")?;
                    }
                    Ok(())
                },
            );
            if point == Step::Synced {
                assert_eq!(result.err().unwrap().code, "conflict");
                assert_eq!(captured, b"external");
                assert_eq!(fs::read(&path).unwrap(), b"external");
            } else {
                assert!(result.is_ok());
                assert_eq!(fs::read(&path).unwrap(), b"pending"); /* Deliberate evidence: late check + rename is not CAS. */
            }
        }
    }
    #[test]
    fn symlink_retarget_and_parent_swap_fail_before_commit() {
        for parent_swap in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let parent = dir.path().join("selected");
            fs::create_dir(&parent).unwrap();
            let original = parent.join("source.txt");
            fs::write(&original, b"base").unwrap();
            let outside = dir.path().join("outside.txt");
            fs::write(&outside, b"secret").unwrap();
            let alias = dir.path().join("alias.txt");
            symlink(&original, &alias).unwrap();
            let (t, h, id) = target(&alias);
            let source = Provenance::new(b"pending").unwrap();
            let result = save_with(
                &t,
                &source.snapshot(),
                &h,
                &id,
                |_, _| Ok(()),
                |s| {
                    if s == Step::Synced {
                        if parent_swap {
                            fs::rename(&parent, dir.path().join("moved"))?;
                            fs::create_dir(&parent)?;
                            fs::write(parent.join("source.txt"), b"different")?;
                        } else {
                            fs::remove_file(&alias)?;
                            symlink(&outside, &alias)?;
                        }
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(fs::read(outside).unwrap(), b"secret");
            assert_eq!(
                fs::read(if parent_swap {
                    dir.path().join("moved/source.txt")
                } else {
                    original
                })
                .unwrap(),
                b"base"
            );
        }
    }
    #[test]
    fn hardlinks_readonly_and_save_as_collision_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        let peer = dir.path().join("peer.txt");
        fs::write(&path, b"base").unwrap();
        fs::hard_link(&path, &peer).unwrap();
        let (t, h, id) = target(&path);
        let source = Provenance::new(b"pending").unwrap();
        assert_eq!(
            save(&t, &source.snapshot(), &h, &id, |_, _| Ok(()))
                .err()
                .unwrap()
                .code,
            "hardlink"
        );
        let copy = dir.path().join("copy.txt");
        save_as(&ScopedFile::destination(&copy).unwrap(), &source.snapshot()).unwrap();
        assert_eq!(fs::read(&peer).unwrap(), b"base");
        assert_eq!(fs::metadata(&copy).unwrap().nlink(), 1);
        assert_eq!(
            save_as(&ScopedFile::destination(&copy).unwrap(), &source.snapshot())
                .err()
                .unwrap()
                .code,
            "exists"
        );
        fs::remove_file(peer).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        let (t, h, id) = target(&path);
        assert_eq!(
            save(&t, &source.snapshot(), &h, &id, |_, _| Ok(()))
                .err()
                .unwrap()
                .code,
            "readonly"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_acl_and_extended_attributes_survive_atomic_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        fs::write(&path, b"base").unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone allow read"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        fn acl(p: &std::path::Path) -> String {
            String::from_utf8(
                std::process::Command::new("/bin/ls")
                    .arg("-le")
                    .arg(p)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
            .lines()
            .filter(|l| l.trim_start().starts_with("0:"))
            .collect::<Vec<_>>()
            .join("\n")
        }
        let before = acl(&path);
        assert!(!before.is_empty());
        let f = File::open(&path).unwrap();
        let name = CString::new("com.marktub.test").unwrap();
        let value = b"keep exact attribute";
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    f.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            },
            0
        );
        drop(f);
        let (t, h, id) = target(&path);
        save(
            &t,
            &Provenance::new(b"after").unwrap().snapshot(),
            &h,
            &id,
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(acl(&path), before);
        let f = File::open(&path).unwrap();
        let mut bytes = vec![0; value.len()];
        assert_eq!(
            unsafe {
                libc::fgetxattr(
                    f.as_raw_fd(),
                    name.as_ptr(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                    0,
                    0,
                )
            },
            value.len() as isize
        );
        assert_eq!(bytes, value);
    }
}
