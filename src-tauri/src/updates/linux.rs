use super::err;
use crate::files::FileError;
use gtk::{
    gio::{self, prelude::*},
    glib::{
        self,
        translate::{ToGlibPtr, from_glib_full},
        variant::{Handle, ObjectPath, ToVariant},
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    process::{Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

const SERVICE: &str = "org.freedesktop.PackageKit";
const OBJECT: &str = "/org/freedesktop/PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
const METHOD_TIMEOUT_MS: i32 = 15_000;
const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EXIT_SUCCESS: u32 = 1;
const EXIT_CANCELLED: u32 = 3;

#[derive(Clone, Copy)]
pub(super) enum Format {
    Deb,
    Rpm,
}

impl Format {
    pub(super) fn file_name(self, version: &str) -> String {
        let suffix = match self {
            Self::Deb => "amd64.deb",
            Self::Rpm => "x86_64.rpm",
        };
        format!("Marktub_{version}_{suffix}")
    }

    fn package_query(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Deb => (
                "/usr/bin/dpkg-query",
                &["-W", "-f=${db:Status-Status} ${Version}", "--", "marktub"],
            ),
            Self::Rpm => (
                "/usr/bin/rpm",
                &["-q", "--queryformat", "%{VERSION}", "--", "marktub"],
            ),
        }
    }

    fn installed_version(self, output: &str) -> Option<semver::Version> {
        let output = output.trim();
        let version = match self {
            Self::Deb => output.strip_prefix("installed ")?,
            Self::Rpm => output,
        };
        // The DEB has no distro revision suffix. RPM's release is a separate
        // field. Reject multiple rows or incomplete packages.
        let version = semver::Version::parse(version).ok()?;
        (version.pre.is_empty() && version.build.is_empty()).then_some(version)
    }
}

fn installed_version(format: Format) -> Result<semver::Version, FileError> {
    let metadata = fs::symlink_metadata("/usr/bin/marktub")?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err(err("The installed executable failed its ownership check."));
    }
    let (program, args) = format.package_query();
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("HOME", "/")
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            let mut output = String::new();
            if let Some(stdout) = child.stdout.take() {
                stdout.take(128).read_to_string(&mut output)?;
            }
            if status.success()
                && let Some(version) = format.installed_version(&output)
            {
                return Ok(version);
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(err("Could not confirm the installed Marktub package."))
}

pub(super) fn confirm_installed(format: Format, version: &str) -> Result<(), FileError> {
    let expected = semver::Version::parse(version).map_err(|_| err("Invalid update version."))?;
    if installed_version(format)? != expected {
        return Err(err("Finish installing this update before restarting."));
    }
    Ok(())
}

pub(super) fn install(format: Format, version: &str, bytes: &[u8]) -> Result<(), FileError> {
    let wanted = semver::Version::parse(version).map_err(|_| err("Invalid update version."))?;
    let current = installed_version(format)?;
    match current.cmp(&wanted) {
        std::cmp::Ordering::Equal => return Ok(()),
        std::cmp::Ordering::Greater => {
            return Err(err(
                "A newer Marktub is already installed. Restart Marktub to use it.",
            ));
        }
        std::cmp::Ordering::Less => {}
    }
    let staging = tempfile::Builder::new()
        .prefix("marktub-update-")
        .tempdir_in("/tmp")?;
    let path = staging.path().join(format.file_name(version));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let connection = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
                .map_err(|_| unavailable())?;
            install_package(&context, &connection, &path)
        })
        .map_err(|_| err("Could not start the system package installer."))??;
    confirm_installed(format, version)
}

fn unavailable() -> FileError {
    err("PackageKit is unavailable. Use the system installer to install the verified package.")
}

fn install_package(
    context: &glib::MainContext,
    connection: &gio::DBusConnection,
    path: &Path,
) -> Result<(), FileError> {
    let call = |object, interface, method, parameters: Option<&glib::Variant>| {
        connection.call_sync(
            Some(SERVICE),
            object,
            interface,
            method,
            parameters,
            None,
            gio::DBusCallFlags::NONE,
            METHOD_TIMEOUT_MS,
            gio::Cancellable::NONE,
        )
    };
    let (transaction,) = call(OBJECT, SERVICE, "CreateTransaction", None)
        .map_err(|_| unavailable())?
        .get::<(ObjectPath,)>()
        .ok_or_else(unavailable)?;

    // PackageKit 1.2 uses string hints and local paths. Newer versions
    // accept typed hints and open file descriptors.
    let hints = (vec!["interactive=true"],).to_variant();
    let hints_result = call(&transaction, TRANSACTION, "SetHints", Some(&hints));
    let result = match hints_result {
        Err(error) if invalid_arguments(&error) => {
            let hints = (HashMap::from([("interactive", true.to_variant())]),).to_variant();
            call(&transaction, TRANSACTION, "SetHints", Some(&hints))
        }
        result => result,
    };
    result.map_err(|_| unavailable())?;

    let event_loop = glib::MainLoop::new(Some(context), false);
    let exit = Rc::new(Cell::new(None));
    let detail = Rc::new(RefCell::new(None));
    let subscriber = connection.signal_subscribe(
        Some(SERVICE),
        Some(TRANSACTION),
        None,
        Some(&transaction),
        None,
        gio::DBusSignalFlags::NONE,
        {
            let exit = Rc::clone(&exit);
            let detail = Rc::clone(&detail);
            let event_loop = event_loop.clone();
            move |_, _, _, _, signal, parameters| match signal {
                "ErrorCode" => {
                    if let Some((_, reason)) = parameters.get::<(u32, String)>() {
                        *detail.borrow_mut() = Some(reason);
                    }
                }
                "Finished" => {
                    exit.set(parameters.get::<(u32, u32)>().map(|(exit, _)| exit));
                    event_loop.quit();
                }
                "Destroy" => event_loop.quit(),
                _ => {}
            }
        },
    );
    let result = (|| {
        let package_path = path
            .to_str()
            .ok_or_else(|| err("Invalid temporary update package path."))?;
        // The release signature is checked by Marktub, independently of the
        // distro's package keys. PackageKit owns approval of this local file.
        // Zero flags request the regular install, without ONLY_TRUSTED.
        let args = (0_u64, vec![package_path]).to_variant();
        match call(&transaction, TRANSACTION, "InstallFiles", Some(&args)) {
            Err(error) if invalid_arguments(&error) => {
                let descriptors = gio::UnixFDList::new();
                let file = File::open(path)?;
                let descriptor = descriptors.append(file).map_err(|_| unavailable())?;
                // GIO 0.18 expects an FD list in the return from its
                // call_with_unix_fd_list wrapper. InstallFiles returns none.
                let message = gio::DBusMessage::new_method_call(
                    Some(SERVICE),
                    &transaction,
                    Some(TRANSACTION),
                    "InstallFiles",
                );
                message.set_body(&(0_u64, vec![Handle(descriptor)]).to_variant());
                message.set_unix_fd_list(Some(&descriptors));
                let (reply, _) = connection
                    .send_message_with_reply_sync(
                        &message,
                        gio::DBusSendMessageFlags::NONE,
                        METHOD_TIMEOUT_MS,
                        gio::Cancellable::NONE,
                    )
                    .map_err(|error| install_request_error(remote_error(&error).as_deref()))?;
                if reply.message_type() != gio::DBusMessageType::MethodReturn {
                    return Err(install_request_error(reply.error_name().as_deref()));
                }
            }
            result => {
                result.map_err(|error| install_request_error(remote_error(&error).as_deref()))?;
            }
        }
        let timeout =
            glib::timeout_source_new(TRANSACTION_TIMEOUT, None, glib::Priority::DEFAULT, {
                let event_loop = event_loop.clone();
                move || {
                    event_loop.quit();
                    glib::ControlFlow::Break
                }
            });
        timeout.attach(Some(context));
        let disconnected = connection.connect_closed({
            let event_loop = event_loop.clone();
            move |_, _, _| event_loop.quit()
        });
        if !connection.is_closed() {
            event_loop.run();
        }
        connection.disconnect(disconnected);
        timeout.destroy();
        if exit.get().is_none() && !connection.is_closed() {
            let _ = call(&transaction, TRANSACTION, "Cancel", None);
        }
        installation_result(exit.get(), detail.borrow().as_deref())
    })();
    connection.signal_unsubscribe(subscriber);
    result
}

fn install_request_error(remote: Option<&str>) -> FileError {
    if remote.is_some_and(|name| name.ends_with(".NotSupported")) {
        return unavailable();
    }
    err(
        "The system package installer could not start the update. Try again or use the system installer.",
    )
}

fn invalid_arguments(error: &glib::Error) -> bool {
    remote_error(error).as_deref() == Some("org.freedesktop.DBus.Error.InvalidArgs")
}

fn remote_error(error: &glib::Error) -> Option<glib::GString> {
    // GIO owns the error; get_remote_error returns an optional owned string.
    unsafe {
        from_glib_full(gio::ffi::g_dbus_error_get_remote_error(
            error.to_glib_none().0,
        ))
    }
}

fn installation_result(exit: Option<u32>, detail: Option<&str>) -> Result<(), FileError> {
    match exit {
        Some(EXIT_SUCCESS) => Ok(()),
        Some(EXIT_CANCELLED) => Err(err("System installation was cancelled.")),
        Some(_) => Err(FileError::new(
            "update",
            detail
                .filter(|message| !message.trim().is_empty())
                .map(|message| {
                    let message: String = message
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(350)
                        .collect();
                    format!("System installation failed. {message}")
                })
                .unwrap_or_else(|| {
                    "System installation failed. Try again or use the system installer.".into()
                }),
        )),
        None => Err(err(
            "The system installer did not report completion. Check the installed version before trying again.",
        )),
    }
}

#[cfg(test)]
mod tests;
