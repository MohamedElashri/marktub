//! Exec-created native supervisor. The anonymous socket is also the app lifeline.
use super::{Output, run_attached};
use crate::files::FileError;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::MetadataExt,
            net::UnixStream,
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const FLAG: &str = "--marktub-job-supervisor";
const REQUEST_LIMIT: usize = 128 * 1024;
const REPLY_LIMIT: usize = 4 * 1024 * 1024;
type ExecutableIdentity = (u64, u64, u64, i64, i64, i64, i64);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    executable: Vec<u8>,
    identity: ExecutableIdentity,
    args: Vec<String>,
    cwd: Vec<u8>,
    seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    output: Option<Output>,
    error: Option<(String, String)>,
}

fn executable_identity(path: &Path) -> std::io::Result<ExecutableIdentity> {
    let metadata = std::fs::metadata(path)?;
    Ok((
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    ))
}

fn frame_write(socket: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    socket.write_all(&(bytes.len() as u32).to_le_bytes())?;
    socket.write_all(bytes)
}

fn frame_read(socket: &mut UnixStream, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let n = u32::from_le_bytes(header) as usize;
    if n > limit {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    let mut bytes = vec![0; n];
    socket.read_exact(&mut bytes)?;
    Ok(bytes)
}

// In the new executable these descriptors have no other Rust owner. Restore
// CLOEXEC immediately so the actual tool cannot keep the app lifeline open.
unsafe fn take_socket(fd: RawFd) -> std::io::Result<UnixStream> {
    if fd < 3 {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut metadata) } < 0
        || metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK
        || unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
    {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let socket = unsafe { UnixStream::from_raw_fd(fd) };
    if !socket.peer_addr()?.is_unnamed() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    Ok(socket)
}

fn serve(token: &str, socket_fd: RawFd, root_fd: RawFd) -> Result<(), FileError> {
    let mut socket = unsafe { take_socket(socket_fd)? };
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let bytes = frame_read(&mut socket, REQUEST_LIMIT)?;
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|_| FileError::new("tool", "Invalid native job request."))?;
    if request.token != token
        || request.seconds == 0
        || request.seconds > 300
        || request.args.len() > 64
        || request.executable.is_empty()
        || request.executable.contains(&0)
        || request.cwd.contains(&0)
    {
        return Err(FileError::new("tool", "Invalid native job request."));
    }
    let root = if root_fd >= 3 && root_fd != socket_fd {
        if unsafe { libc::fcntl(root_fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let root = unsafe { File::from_raw_fd(root_fd) };
        if !root.metadata()?.is_dir() {
            return Err(FileError::new("tool", "Invalid native job directory."));
        }
        Some(root)
    } else if root_fd == -1 {
        None
    } else {
        return Err(FileError::new("tool", "Invalid native job directory."));
    };
    socket.set_read_timeout(None)?;
    socket.set_nonblocking(true)?;
    let executable = PathBuf::from(std::ffi::OsString::from_vec(request.executable));
    let cwd = PathBuf::from(std::ffi::OsString::from_vec(request.cwd));
    // Revalidate after the exec handoff, before starting the selected tool.
    let output = if executable_identity(&executable).ok() != Some(request.identity) {
        Err(FileError::new(
            "tool",
            "The selected executable changed before launch. Select it again.",
        ))
    } else {
        run_attached(
            &executable,
            &request.args,
            &cwd,
            root.as_ref(),
            || {
                let mut signal = [0];
                match socket.read(&mut signal) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
                    // EOF, a cancellation byte or a failed controller all stop the group.
                    _ => true,
                }
            },
            request.seconds,
        )
    };
    let reply = match output {
        Ok(output) => Reply {
            output: Some(output),
            error: None,
        },
        Err(error) => Reply {
            output: None,
            error: Some((error.code.into(), error.message)),
        },
    };
    socket.set_nonblocking(false)?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    let bytes = serde_json::to_vec(&reply)
        .map_err(|_| FileError::new("tool", "Cannot encode native job result."))?;
    frame_write(&mut socket, &bytes)?;
    Ok(())
}

pub(crate) fn entry() -> Option<i32> {
    let args = std::env::args_os().collect::<Vec<_>>();
    if args.get(1).is_none_or(|arg| arg != FLAG) {
        return None;
    }
    let parse = || {
        if args.len() != 5 {
            return None;
        }
        let token = args[2].to_str()?;
        uuid::Uuid::parse_str(token).ok()?;
        Some((
            token,
            args[3].to_str()?.parse().ok()?,
            args[4].to_str()?.parse().ok()?,
        ))
    };
    Some(match parse() {
        Some((token, socket, root)) if serve(token, socket, root).is_ok() => 0,
        _ => 2,
    })
}

struct Controller {
    socket: Option<UnixStream>,
    child: Child,
}
impl Drop for Controller {
    fn drop(&mut self) {
        // Closing the lifeline first lets the supervisor stop/reap its own group.
        self.socket.take();
        let _ = self.child.wait();
    }
}

fn carried_fd(fd: RawFd) -> std::io::Result<OwnedFd> {
    // A desktop/CLI launch can have closed stdio. Never let the child stdio
    // setup overwrite an inherited socket or selected-directory descriptor.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

pub(super) fn run(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    root: Option<&File>,
    cancel: &Arc<AtomicBool>,
    seconds: u64,
) -> Result<Output, FileError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(FileError::new("canceled", "Job canceled."));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let request = Request {
        token: token.clone(),
        executable: executable.as_os_str().as_bytes().into(),
        identity: executable_identity(executable)
            .map_err(|_| FileError::new("tool", "The selected executable is unavailable."))?,
        args: args.into(),
        cwd: cwd.as_os_str().as_bytes().into(),
        seconds,
    };
    let bytes = serde_json::to_vec(&request)
        .map_err(|_| FileError::new("tool", "Invalid native job request."))?;
    if bytes.len() > REQUEST_LIMIT || seconds == 0 || seconds > 300 || args.len() > 64 {
        return Err(FileError::new(
            "capacity",
            "Native job request exceeds its limits.",
        ));
    }
    let (socket, child_socket) = UnixStream::pair()?;
    let inherited_socket = carried_fd(child_socket.as_raw_fd())?;
    let inherited_root = root.map(|file| carried_fd(file.as_raw_fd())).transpose()?;
    let socket_fd = inherited_socket.as_raw_fd();
    let root_fd = inherited_root.as_ref().map_or(-1, AsRawFd::as_raw_fd);
    let mut command = Command::new(std::env::current_exe()?);
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(not(test))]
    command.args([FLAG, &token, &socket_fd.to_string(), &root_fd.to_string()]);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "power::process::tests::supervisor_worker",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "MARKTUB_TEST_SUPERVISOR",
            format!("{token}:{socket_fd}:{root_fd}"),
        );
    unsafe {
        command.pre_exec(move || {
            for fd in [socket_fd, root_fd].into_iter().filter(|fd| *fd >= 0) {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop(child_socket);
    drop(inherited_socket);
    drop(inherited_root);
    let mut controller = Controller {
        socket: Some(socket),
        child,
    };
    let socket = controller.socket.as_mut().unwrap();
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    frame_write(socket, &bytes)?;
    socket.set_nonblocking(true)?;
    let start = Instant::now();
    let mut received = Vec::new();
    let mut expected = None;
    let mut canceled = false;
    loop {
        if cancel.load(Ordering::SeqCst) && !canceled {
            socket.write_all(&[1])?;
            canceled = true;
        }
        let mut buffer = [0; 8192];
        match socket.read(&mut buffer) {
            Ok(0) => {
                return Err(FileError::new(
                    "tool",
                    "Native job supervision ended unexpectedly.",
                ));
            }
            Ok(n) => {
                received.extend_from_slice(&buffer[..n]);
                if expected.is_none() && received.len() >= 4 {
                    let n = u32::from_le_bytes(received[..4].try_into().unwrap()) as usize;
                    if n > REPLY_LIMIT {
                        return Err(FileError::new(
                            "capacity",
                            "Native job result exceeds its limits.",
                        ));
                    }
                    expected = Some(n + 4);
                }
                if let Some(size) = expected
                    && received.len() >= size
                {
                    let reply: Reply = serde_json::from_slice(&received[4..size])
                        .map_err(|_| FileError::new("tool", "Invalid native job result."))?;
                    if canceled {
                        return Err(FileError::new(
                            "canceled",
                            "Job canceled; attached process group stopped.",
                        ));
                    }
                    if let Some(output) = reply.output {
                        return Ok(output);
                    }
                    let (code, message) = reply
                        .error
                        .ok_or_else(|| FileError::new("tool", "Missing native job result."))?;
                    return Err(FileError::new(
                        match code.as_str() {
                            "capacity" => "capacity",
                            "canceled" => "canceled",
                            "timeout" => "timeout",
                            "io" => "io",
                            _ => "tool",
                        },
                        message,
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        if start.elapsed() > Duration::from_secs(seconds + 5) {
            return Err(FileError::new(
                "timeout",
                "Native job supervision timed out.",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
pub(super) fn test_entry() {
    let input = std::env::var("MARKTUB_TEST_SUPERVISOR").unwrap();
    let fields = input.split(':').collect::<Vec<_>>();
    std::process::exit(
        if serve(
            fields[0],
            fields[1].parse().unwrap(),
            fields[2].parse().unwrap(),
        )
        .is_ok()
        {
            0
        } else {
            2
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;

    #[test]
    fn token_mismatch_and_oversized_frame_never_start_a_tool() {
        for oversized in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let (mut controller, receiver) = UnixStream::pair().unwrap();
            let fd = receiver.into_raw_fd();
            let thread = std::thread::spawn(move || serve("expected", fd, -1));
            if oversized {
                controller
                    .write_all(&((REQUEST_LIMIT + 1) as u32).to_le_bytes())
                    .unwrap();
            } else {
                let bytes = serde_json::to_vec(&Request {
                    token: "wrong".into(),
                    executable: b"/bin/sh".to_vec(),
                    identity: executable_identity(Path::new("/bin/sh")).unwrap(),
                    args: vec!["-c".into(), "touch must-not-exist".into()],
                    cwd: folder.path().as_os_str().as_bytes().to_vec(),
                    seconds: 2,
                })
                .unwrap();
                frame_write(&mut controller, &bytes).unwrap();
            }
            assert!(thread.join().unwrap().is_err());
            assert!(!folder.path().join("must-not-exist").exists());
        }
    }

    #[test]
    fn executable_replacement_during_handoff_refuses_execution() {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("selected-tool");
        std::fs::copy("/bin/sh", &executable).unwrap();
        let identity = executable_identity(&executable).unwrap();
        std::fs::write(&executable, b"#!/bin/sh\ntouch must-not-exist\n").unwrap();
        let (mut controller, receiver) = UnixStream::pair().unwrap();
        let fd = receiver.into_raw_fd();
        let thread = std::thread::spawn(move || serve("expected", fd, -1));
        let request = Request {
            token: "expected".into(),
            executable: executable.as_os_str().as_bytes().to_vec(),
            identity,
            args: vec![],
            cwd: folder.path().as_os_str().as_bytes().to_vec(),
            seconds: 2,
        };
        frame_write(&mut controller, &serde_json::to_vec(&request).unwrap()).unwrap();
        let reply: Reply =
            serde_json::from_slice(&frame_read(&mut controller, REPLY_LIMIT).unwrap()).unwrap();
        assert!(reply.output.is_none());
        assert!(reply.error.unwrap().1.contains("changed before launch"));
        assert!(thread.join().unwrap().is_ok());
        assert!(!folder.path().join("must-not-exist").exists());
    }

    #[test]
    fn request_bounds_and_non_directory_grants_refuse_before_execution() {
        let folder = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        for (args, seconds) in [
            (vec![], 0),
            (vec![], 301),
            (vec!["x".into(); 65], 2),
            (vec!["x".repeat(REQUEST_LIMIT)], 2),
        ] {
            assert_eq!(
                run(
                    Path::new("/bin/sh"),
                    &args,
                    folder.path(),
                    None,
                    &cancel,
                    seconds
                )
                .unwrap_err()
                .code,
                "capacity"
            );
        }
        let regular = File::create(folder.path().join("regular")).unwrap();
        assert!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "touch must-not-exist".into()],
                folder.path(),
                Some(&regular),
                &cancel,
                2
            )
            .is_err()
        );
        assert!(!folder.path().join("must-not-exist").exists());
    }
}
