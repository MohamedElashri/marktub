//! Native-selected tools; the renderer receives only opaque job handles.
use crate::files::FileError;
use serde::{Deserialize, Serialize};
pub(crate) mod supervisor;
use std::{
    fs::File,
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
pub const OUTPUT: usize = 256 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}
fn drain(r: &mut impl Read, out: &mut Vec<u8>) -> Result<(), FileError> {
    let mut b = [0u8; 8192];
    loop {
        match r.read(&mut b) {
            Ok(0) => break,
            Ok(n) => {
                if out.len() + n > OUTPUT {
                    return Err(FileError::new(
                        "capacity",
                        "Process output exceeded 256 KiB per stream; the process group was stopped.",
                    ));
                }
                out.extend_from_slice(&b[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn run_attached(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    root: Option<&File>,
    mut canceled: impl FnMut() -> bool,
    seconds: u64,
) -> Result<Output, FileError> {
    if canceled() {
        return Err(FileError::new("canceled", "Job canceled."));
    }
    let mut c = Command::new(executable);
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .env(
            "PATH",
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin",
        )
        .env("HOME", cwd)
        .env("TMPDIR", cwd)
        .env("LANG", "en_US.UTF-8")
        .env("TEXMFOUTPUT", cwd)
        .env("TEXMFVAR", cwd.join("tex-var"))
        .env("TEXMFCONFIG", cwd.join("tex-config"))
        .env("openin_any", "p")
        .env("openout_any", "p")
        .env("shell_escape", "f")
        .env("MKTEXFMT", "0")
        .env("MKTEXPK", "0")
        .env("MKTEXTEX", "0")
        .env("MKTEXTFM", "0")
        .env("TYPST_PACKAGE_PATH", cwd.join("packages"))
        .env("TYPST_PACKAGE_CACHE_PATH", cwd.join("packages"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let root_fd = root.map(AsRawFd::as_raw_fd);
    // Only async-signal-safe libc operations between fork and exec.
    unsafe {
        c.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(fd) = root_fd
                && libc::fchdir(fd) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, soft, hard) in [
                (libc::RLIMIT_CORE, 0, 0),
                (libc::RLIMIT_FSIZE, 64 * 1024 * 1024, 64 * 1024 * 1024),
                (libc::RLIMIT_CPU, seconds + 1, seconds + 2),
            ] {
                let value = libc::rlimit {
                    rlim_cur: soft as libc::rlim_t,
                    rlim_max: hard as libc::rlim_t,
                };
                if libc::setrlimit(resource, &value) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|_| {
        FileError::new(
            "tool",
            "The selected tool could not start. Select an installed executable again.",
        )
    })?;
    let pid = child.id() as i32;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        unsafe {
            let f = libc::fcntl(fd, libc::F_GETFL);
            if f < 0 || libc::fcntl(fd, libc::F_SETFL, f | libc::O_NONBLOCK) < 0 {
                let error = std::io::Error::last_os_error();
                libc::kill(-pid, libc::SIGKILL);
                let _ = child.wait();
                return Err(error.into());
            }
        }
    }
    let start = Instant::now();
    let mut out = vec![];
    let mut err = vec![];
    let result = (|| {
        loop {
            drain(&mut stdout, &mut out)?;
            drain(&mut stderr, &mut err)?;
            if canceled() {
                return Err(FileError::new(
                    "canceled",
                    "Job canceled; attached process group stopped. Detached descendants may survive.",
                ));
            }
            if start.elapsed() > Duration::from_secs(seconds) {
                return Err(FileError::new(
                    "timeout",
                    "Job timed out; attached process group stopped.",
                ));
            }
            // Retain the leader's PID until its entire group has been stopped.
            // try_wait would reap it and permit PID/group reuse before killpg.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if info.si_signo != 0 {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let status = child.wait();
    result?;
    let status = status?;
    drain(&mut stdout, &mut out)?;
    drain(&mut stderr, &mut err)?;
    Ok(Output {
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        code: status.code(),
    })
}
pub fn run(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    root: Option<&File>,
    cancel: &Arc<AtomicBool>,
    seconds: u64,
) -> Result<Output, FileError> {
    supervisor::run(executable, args, cwd, root, cancel, seconds)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    #[ignore = "exec-created supervisor entry, exercised by native runner tests"]
    fn supervisor_worker() {
        supervisor::test_entry();
    }
    #[test]
    #[ignore = "process-crash worker, exercised by its parent acceptance test"]
    fn app_crash_worker() {
        let folder = std::env::var_os("MARKTUB_TEST_CRASH_FOLDER").unwrap();
        run(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf '%s\n' $$ > leader; sleep 60 & printf '%s\n' $! > attached; wait".into(),
            ],
            Path::new(&folder),
            None,
            &Arc::new(AtomicBool::new(false)),
            60,
        )
        .unwrap();
    }
    #[test]
    fn app_sigkill_stops_attached_job_before_its_wall_timeout() {
        let folder = tempfile::tempdir().unwrap();
        let mut worker = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "power::process::tests::app_crash_worker",
                "--ignored",
            ])
            .env("MARKTUB_TEST_CRASH_FOLDER", folder.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !folder.path().join("attached").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // This is an owned test process, never a discovered application PID.
        worker.kill().unwrap();
        worker.wait().unwrap();
        let pids = ["leader", "attached"].map(|name| {
            std::fs::read_to_string(folder.path().join(name))
                .unwrap()
                .trim()
                .parse::<i32>()
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) == 0 })
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            pids.iter().all(|pid| unsafe { libc::kill(*pid, 0) < 0 }),
            "attached group survived its controller"
        );
    }
    #[test]
    #[ignore = "closed-stdio worker, exercised by its parent acceptance test"]
    fn low_stdio_worker() {
        let folder = std::env::var_os("MARKTUB_TEST_LOW_STDIO").unwrap();
        unsafe {
            libc::close(0);
            libc::close(1);
            libc::close(2);
        }
        let root = File::open(&folder).unwrap();
        let result = run(
            Path::new("/bin/sh"),
            &["-c".into(), "printf pinned > result".into()],
            Path::new("/"),
            Some(&root),
            &Arc::new(AtomicBool::new(false)),
            2,
        );
        std::process::exit(
            if result.is_ok()
                && std::fs::read(Path::new(&folder).join("result"))
                    .ok()
                    .as_deref()
                    == Some(b"pinned")
            {
                0
            } else {
                1
            },
        );
    }
    #[test]
    fn closed_standard_streams_cannot_clobber_the_native_directory_grant() {
        let folder = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "power::process::tests::low_stdio_worker",
                "--ignored",
            ])
            .env("MARKTUB_TEST_LOW_STDIO", folder.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read(folder.path().join("result")).unwrap(),
            b"pinned"
        );
    }
    #[test]
    fn selected_directory_fd_survives_supervision_and_nonzero_exit_is_reported() {
        let folder = tempfile::tempdir().unwrap();
        let root = File::open(folder.path()).unwrap();
        let output = run(
            Path::new("/bin/sh"),
            &["-c".into(), "printf pinned > result; exit 7".into()],
            Path::new("/"),
            Some(&root),
            &Arc::new(AtomicBool::new(false)),
            2,
        )
        .unwrap();
        assert_eq!(output.code, Some(7));
        assert_eq!(
            std::fs::read(folder.path().join("result")).unwrap(),
            b"pinned"
        );
    }
    #[test]
    fn naturally_exited_leader_does_not_leave_attached_descendants() {
        let folder = tempfile::tempdir().unwrap();
        run(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "sleep 60 & printf '%s\n' $! > attached; exit 0".into(),
            ],
            folder.path(),
            None,
            &Arc::new(AtomicBool::new(false)),
            2,
        )
        .unwrap();
        let pid = std::fs::read_to_string(folder.path().join("attached"))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid, 0) == 0 } && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(unsafe { libc::kill(pid, 0) < 0 });
    }
    #[test]
    fn real_output_limit_timeout_and_cancel_stop_groups() {
        let t = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let o = run(
            Path::new("/bin/sh"),
            &["-c".into(), "printf exact; printf issue >&2".into()],
            t.path(),
            None,
            &cancel,
            2,
        )
        .unwrap();
        assert_eq!(o.stdout, "exact");
        assert_eq!(o.stderr, "issue");
        assert_eq!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "yes output".into()],
                t.path(),
                None,
                &cancel,
                2
            )
            .unwrap_err()
            .code,
            "capacity"
        );
        assert_eq!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "sleep 10".into()],
                t.path(),
                None,
                &cancel,
                1
            )
            .unwrap_err()
            .code,
            "timeout"
        );
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::SeqCst);
        });
        assert_eq!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "sleep 10".into()],
                t.path(),
                None,
                &cancel,
                5
            )
            .unwrap_err()
            .code,
            "canceled"
        );
    }
}
