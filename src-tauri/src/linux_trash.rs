//! Home-volume freedesktop Trash. Other volumes fail without deleting a source.
use crate::{files::FileError, private_store};
use std::{
    ffi::{CString, OsStr},
    fs::{self, File},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

fn refusal() -> FileError {
    FileError::new(
        "trash",
        "Safe home-volume Trash is unavailable; source retained. No permanent deletion was attempted.",
    )
}
fn name(value: &OsStr) -> Result<CString, FileError> {
    CString::new(value.as_bytes()).map_err(|_| refusal())
}
fn directory(parent: &File, component: &OsStr, create: bool) -> Result<File, FileError> {
    let component = name(component)?;
    if create {
        let result = unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(refusal());
        }
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            component.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(refusal());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn private(file: &File, device: u64) -> Result<(), FileError> {
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.dev() != device
    {
        return Err(refusal());
    }
    Ok(())
}
fn data_directory(path: &Path) -> Result<File, FileError> {
    if !path.is_absolute() {
        return Err(refusal());
    }
    let mut current = File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(component) => {
                current = match directory(&current, component, false) {
                    Ok(next) => next,
                    Err(_) => {
                        let held = current.metadata()?;
                        if held.uid() != unsafe { libc::geteuid() } || held.mode() & 0o022 != 0 {
                            return Err(refusal());
                        }
                        let next = directory(&current, component, true)?;
                        private_store::sync(&current)?;
                        next
                    }
                };
            }
            _ => return Err(refusal()),
        }
    }
    let metadata = current.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
        return Err(refusal());
    }
    Ok(current)
}
fn location(file: &File) -> Result<PathBuf, FileError> {
    let path = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    let actual = fs::metadata(&path)?;
    let held = file.metadata()?;
    if !path.is_absolute() || (actual.dev(), actual.ino()) != (held.dev(), held.ino()) {
        return Err(refusal());
    }
    Ok(path)
}
fn node(parent: &File, entry: &OsStr) -> Result<File, FileError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name(entry)?.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(refusal());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn encoded(path: &Path) -> String {
    let mut output = String::new();
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            output.push(*byte as char);
        } else {
            use std::fmt::Write;
            write!(output, "%{byte:02X}").expect("String write");
        }
    }
    output
}
fn date() -> Result<String, FileError> {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&now, local.as_mut_ptr()) }.is_null() {
        return Err(refusal());
    }
    let mut buffer = [0u8; 32];
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%Y-%m-%dT%H:%M:%S".as_ptr(),
            local.as_ptr(),
        )
    };
    if length != 19 {
        return Err(refusal());
    }
    String::from_utf8(buffer[..length].to_vec()).map_err(|_| refusal())
}

pub(super) fn trash(
    parent: &File,
    entry: &OsStr,
    expected: &super::Stamp,
) -> Result<(), FileError> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty()) {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(refusal)?
            .join(".local/share"),
    };
    trash_in(parent, entry, expected, &data)
}

fn trash_in(
    parent: &File,
    entry: &OsStr,
    expected: &super::Stamp,
    data: &Path,
) -> Result<(), FileError> {
    // Native caller supplies one entry relative to an already-authorized parent FD.
    if entry.as_bytes().is_empty()
        || entry.as_bytes().contains(&b'/')
        || entry == "."
        || entry == ".."
    {
        return Err(refusal());
    }
    let original_parent = location(parent)?;
    let source = node(parent, entry)?;
    let device = expected.0;
    if super::stamp(&source.metadata()?) != *expected {
        return Err(refusal());
    }
    let before = crate::atomic_save::Identity::of(&source.metadata()?);
    if source.metadata()?.dev() != device {
        return Err(refusal());
    }
    let original = original_parent.join(entry);
    if data.join("Trash").starts_with(&original) {
        return Err(refusal());
    }
    let data_fd = data_directory(data)?;
    if data_fd.metadata()?.dev() != device {
        return Err(refusal());
    }
    let trash = directory(&data_fd, OsStr::new("Trash"), true)?;
    private(&trash, device)?;
    let files = directory(&trash, OsStr::new("files"), true)?;
    let info = directory(&trash, OsStr::new("info"), true)?;
    private(&files, device)?;
    private(&info, device)?;
    // Preserve raw filename bytes in Path; the stored name is bounded and unique.
    let id = uuid::Uuid::new_v4().to_string();
    let stored = CString::new(id.clone()).expect("UUID");
    let information = CString::new(format!("{id}.trashinfo")).expect("UUID");
    let fd = unsafe {
        libc::openat(
            info.as_raw_fd(),
            information.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(refusal());
    }
    let mut record = unsafe { File::from_raw_fd(fd) };
    // A failed metadata write leaves the source untouched and may leave an orphan
    // record. Never unlink a name another process may have replaced.
    write!(
        record,
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encoded(&original),
        date()?
    )?;
    private_store::sync(&record)?;
    private_store::sync(&info)?;
    private_store::sync(&trash)?;
    private_store::sync(&data_fd)?;
    let record_identity = crate::atomic_save::Identity::of(&record.metadata()?);
    // Revalidate directory identities immediately before the only source mutation.
    if location(parent)? != original_parent
        || location(&data_fd)? != data
        || location(&trash)? != data.join("Trash")
        || location(&files)? != data.join("Trash/files")
        || location(&info)? != data.join("Trash/info")
    {
        return Err(refusal());
    }
    private(&trash, device)?;
    private(&files, device)?;
    private(&info, device)?;
    if crate::atomic_save::Identity::of(&source.metadata()?) != before
        || crate::atomic_save::Identity::of(&node(parent, entry)?.metadata()?) != before
        || crate::atomic_save::Identity::of(
            &node(&info, OsStr::new(information.to_str().expect("UUID")))?.metadata()?,
        ) != record_identity
    {
        return Err(refusal());
    }
    let result = unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            name(entry)?.as_ptr(),
            files.as_raw_fd(),
            stored.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(refusal());
    }
    private_store::sync(&files).and_then(|_| private_store::sync(parent)).map_err(|_| FileError::new("durability", "Moved to System Trash, but directory synchronization failed. Check Trash and the original location before retrying."))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };

    fn send(parent: &File, entry: &OsStr, device: u64, data: &Path) -> Result<(), FileError> {
        let mut expected = super::super::stamp(&super::super::stat_at(parent, entry)?);
        expected.0 = device;
        super::trash_in(parent, entry, &expected, data)
    }

    #[test]
    fn stale_selected_entry_is_retained_without_adopting_replacement() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("selected.md");
        fs::write(&path, b"selected").unwrap();
        let parent = File::open(fixture.path()).unwrap();
        let expected = super::super::stamp(&fs::metadata(&path).unwrap());
        fs::rename(&path, fixture.path().join("original.md")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        let data = fixture.path().join("data");
        assert!(super::trash_in(&parent, OsStr::new("selected.md"), &expected, &data).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(
            fs::read(fixture.path().join("original.md")).unwrap(),
            b"selected"
        );
        assert!(!data.exists());
    }

    #[test]
    fn trash_preserves_raw_names_bytes_and_metadata_for_system_restore() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let data = root.join("data");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        let raw = std::ffi::OsString::from_vec(b"raw-\xff %\n.md".to_vec());
        let path = root.join(&raw);
        fs::write(&path, b"\xef\xbb\xbfmixed\r\nend\r").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let before = fs::metadata(&path).unwrap();
        let parent = File::open(root).unwrap();
        send(&parent, &raw, before.dev(), &data).unwrap();
        assert!(!path.exists());
        let records: Vec<_> = fs::read_dir(data.join("Trash/info"))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(records.len(), 1);
        let text = fs::read_to_string(records[0].path()).unwrap();
        assert!(text.starts_with(&format!(
            "[Trash Info]\nPath={}\nDeletionDate=",
            encoded(&path)
        )));
        assert!(text.contains("raw-%FF%20%25%0A.md"));
        let stored = data.join("Trash/files").join(
            records[0]
                .file_name()
                .to_str()
                .unwrap()
                .strip_suffix(".trashinfo")
                .unwrap(),
        );
        let after = fs::metadata(&stored).unwrap();
        assert_eq!(
            (
                before.ino(),
                before.mode(),
                before.uid(),
                before.gid(),
                before.mtime()
            ),
            (
                after.ino(),
                after.mode(),
                after.uid(),
                after.gid(),
                after.mtime()
            )
        );
        fs::rename(stored, &path).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"\xef\xbb\xbfmixed\r\nend\r");
    }

    #[test]
    fn trash_refuses_symlinks_unsafe_directories_and_foreign_devices() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let data = root.join("data");
        let other = root.join("other");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&other).unwrap();
        fs::write(root.join("keep.md"), b"keep").unwrap();
        let parent = File::open(root).unwrap();
        let device = parent.metadata().unwrap().dev();
        assert!(
            send(
                &parent,
                OsStr::new("keep.md"),
                device.wrapping_add(1),
                &data
            )
            .is_err()
        );
        symlink(&other, data.join("Trash")).unwrap();
        assert!(send(&parent, OsStr::new("keep.md"), device, &data).is_err());
        fs::remove_file(data.join("Trash")).unwrap();
        fs::create_dir(data.join("Trash")).unwrap();
        fs::set_permissions(data.join("Trash"), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(send(&parent, OsStr::new("keep.md"), device, &data).is_err());
        assert_eq!(fs::read(root.join("keep.md")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(other).unwrap().count(), 0);
    }

    #[test]
    fn repeated_names_and_directories_keep_independent_restore_records() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let data = root.join("fresh/share");
        let parent = File::open(root).unwrap();
        let device = parent.metadata().unwrap().dev();
        for bytes in [b"first".as_slice(), b"second"] {
            fs::write(root.join("same.md"), bytes).unwrap();
            send(&parent, OsStr::new("same.md"), device, &data).unwrap();
        }
        fs::create_dir(root.join("folder")).unwrap();
        fs::write(root.join("folder/child.md"), b"child").unwrap();
        send(&parent, OsStr::new("folder"), device, &data).unwrap();
        let mut contents: Vec<_> = fs::read_dir(data.join("Trash/files"))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                assert!(
                    data.join("Trash/info")
                        .join(format!(
                            "{}.trashinfo",
                            path.file_name().unwrap().to_str().unwrap()
                        ))
                        .exists()
                );
                fs::read(if path.is_dir() {
                    path.join("child.md")
                } else {
                    path
                })
                .unwrap()
            })
            .collect();
        contents.sort();
        assert_eq!(
            contents,
            [b"child".to_vec(), b"first".to_vec(), b"second".to_vec()]
        );
    }

    #[test]
    fn linked_trash_children_and_metadata_failure_keep_the_source() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let data = root.join("data");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(data.join("Trash")).unwrap();
        fs::set_permissions(data.join("Trash"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(root.join("other")).unwrap();
        fs::write(root.join("keep.md"), b"keep").unwrap();
        let parent = File::open(root).unwrap();
        let device = parent.metadata().unwrap().dev();
        symlink(root.join("other"), data.join("Trash/files")).unwrap();
        assert!(send(&parent, OsStr::new("keep.md"), device, &data).is_err());
        fs::remove_file(data.join("Trash/files")).unwrap();
        fs::write(data.join("Trash/info"), b"occupied").unwrap();
        assert!(send(&parent, OsStr::new("keep.md"), device, &data).is_err());
        assert_eq!(fs::read(root.join("keep.md")).unwrap(), b"keep");
        assert_eq!(fs::read(data.join("Trash/info")).unwrap(), b"occupied");
        assert_eq!(fs::read_dir(root.join("other")).unwrap().count(), 0);
    }
}
