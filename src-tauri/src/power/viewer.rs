//! A fixed system viewer request, after the caller revalidates its exported PDF.
use crate::files::FileError;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn unavailable() -> FileError {
    FileError::new(
        "power",
        "System PDF viewer could not start. Open the exported file manually to print.",
    )
}

pub(super) fn open(path: &Path) -> Result<(), FileError> {
    launch(Path::new("/usr/bin/xdg-open"), path)
}

fn launch(launcher: &Path, path: &Path) -> Result<(), FileError> {
    let mut child = Command::new(launcher)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable())?;
    // Some Linux desktop launchers remain attached to the viewer. Detect quick
    // launch errors, then reap asynchronously without keeping the app modal.
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match child.try_wait().map_err(|_| unavailable())? {
            Some(status) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(unavailable())
                };
            }
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => break,
        }
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn viewer_lifetime_does_not_hold_the_app_and_finished_launchers_are_reaped() {
        let directory = tempfile::tempdir().unwrap();
        let launcher = directory.path().join("viewer");
        let marker = directory.path().join("pid");
        fs::write(&launcher, "#!/bin/sh\nprintf '%s' $$ > \"$1\"\nsleep 1\n").unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        launch(&launcher, &marker).unwrap();
        assert!(start.elapsed() < Duration::from_millis(800));
        let pid: i32 = fs::read_to_string(&marker).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "Viewer launcher must be reaped");
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::write(&launcher, "#!/bin/sh\nexit 7\n").unwrap();
        assert!(launch(&launcher, &marker).is_err());
        assert!(launch(&directory.path().join("missing"), &marker).is_err());
    }
}
