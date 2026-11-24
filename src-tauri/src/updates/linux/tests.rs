use super::*;
use std::{
    io::{BufRead, BufReader},
    os::fd::FromRawFd,
    process::Child,
    sync::mpsc,
};

const TEST_TRANSACTION: &str = "/test_transaction";
const PACKAGE: &[u8] = b"verified update fixture";

struct TestBus(Child);

impl TestBus {
    fn start() -> (Self, String) {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        (Self(child), address.trim().to_owned())
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn connect(address: &str) -> gio::DBusConnection {
    gio::DBusConnection::for_address_sync(
        address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap()
}

fn simulate_install(descriptors: bool, exit: u32) -> Result<(), FileError> {
    let (_bus, address) = TestBus::start();
    let (ready, service) = mpsc::channel();
    let server_address = address.clone();
    let server = std::thread::spawn(move || {
        let context = glib::MainContext::new();
        context.with_thread_default(|| {
            let connection = connect(&server_address);
            connection.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&(SERVICE, 0_u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                METHOD_TIMEOUT_MS,
                gio::Cancellable::NONE,
            ).unwrap();
            let service = gio::DBusNodeInfo::for_xml(&format!(
                "<node><interface name='{SERVICE}'><method name='CreateTransaction'><arg type='o' direction='out'/></method></interface></node>"
            )).unwrap();
            connection.register_object(
                OBJECT, &service.interfaces()[0],
                |_, _, _, _, _, _, invocation| {
                    invocation.return_value(Some(&(ObjectPath::try_from(TEST_TRANSACTION).unwrap(),).to_variant()));
                },
                |_, _, _, _, _| false.to_variant(),
                |_, _, _, _, _, _| false,
            ).unwrap();
            let (hints, files) = if descriptors { ("a{sv}", "ah") } else { ("as", "as") };
            let transaction = gio::DBusNodeInfo::for_xml(&format!(
                "<node><interface name='{TRANSACTION}'>
                   <method name='SetHints'><arg type='{hints}' direction='in'/></method>
                   <method name='InstallFiles'><arg type='t' direction='in'/><arg type='{files}' direction='in'/></method>
                 </interface></node>"
            )).unwrap();
            connection.register_object(
                TEST_TRANSACTION, &transaction.interfaces()[0],
                move |connection, _, object, _, method, args, invocation| {
                    if method == "SetHints" {
                        let interactive = if descriptors {
                            args.get::<(HashMap<String, glib::Variant>,)>()
                                .and_then(|(hints,)| hints.get("interactive")?.get::<bool>())
                                == Some(true)
                        } else {
                            args.get::<(Vec<String>,)>().is_some_and(|(hints,)| hints == ["interactive=true"])
                        };
                        if interactive {
                            invocation.return_value(None);
                        } else {
                            invocation.return_dbus_error("org.freedesktop.PackageKit.Transaction.InputInvalid", "interactive authorization required");
                        }
                        return;
                    }
                    let bytes = if descriptors {
                        args.get::<(u64, Vec<Handle>)>().and_then(|(flags, files)| {
                            let [handle] = files.as_slice() else { return None };
                            if flags != 0 { return None; }
                            let fd = invocation.message().unix_fd_list()?.get(handle.0).ok()?;
                            let mut bytes = Vec::new();
                            // GIO returns a new descriptor owned by its caller.
                            unsafe { File::from_raw_fd(fd) }.read_to_end(&mut bytes).ok()?;
                            Some(bytes)
                        })
                    } else {
                        args.get::<(u64, Vec<String>)>().and_then(|(flags, files)| {
                            let [file] = files.as_slice() else { return None };
                            (flags == 0).then(|| fs::read(file).ok()).flatten()
                        })
                    };
                    if bytes.as_deref() != Some(PACKAGE) {
                        invocation.return_dbus_error("org.freedesktop.PackageKit.Transaction.InputInvalid", "wrong package");
                        return;
                    }
                    if exit != EXIT_SUCCESS {
                        connection.emit_signal(None, object, TRANSACTION, "ErrorCode",
                            Some(&(1_u32, "The package manager is busy.").to_variant())).unwrap();
                    }
                    // A fast installer can finish before replying to the method.
                    connection.emit_signal(None, object, TRANSACTION, "Finished",
                        Some(&(exit, 1_u32).to_variant())).unwrap();
                    invocation.return_value(None);
                },
                |_, _, _, _, _| false.to_variant(),
                |_, _, _, _, _, _| false,
            ).unwrap();
            let event_loop = glib::MainLoop::new(Some(&context), false);
            glib::idle_source_new(None, glib::Priority::DEFAULT, {
                let event_loop = event_loop.clone();
                move || {
                    ready.send(event_loop.clone()).unwrap();
                    glib::ControlFlow::Break
                }
            }).attach(Some(&context));
            event_loop.run();
        }).unwrap();
    });
    let event_loop = service.recv_timeout(Duration::from_secs(5)).unwrap();
    let stage = tempfile::tempdir().unwrap();
    let package = stage.path().join(Format::Deb.file_name("9.0.0"));
    fs::write(&package, PACKAGE).unwrap();
    let context = glib::MainContext::new();
    let result = context
        .with_thread_default(|| install_package(&context, &connect(&address), &package))
        .unwrap();
    event_loop.quit();
    server.join().unwrap();
    result
}

#[test]
fn packagekit_waits_for_completion_with_both_local_file_apis() {
    simulate_install(false, EXIT_SUCCESS).unwrap();
    simulate_install(true, EXIT_SUCCESS).unwrap();
    assert!(
        simulate_install(false, EXIT_CANCELLED)
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    assert!(
        simulate_install(true, 2)
            .unwrap_err()
            .message
            .contains("package manager is busy")
    );
}

#[test]
fn installed_version_requires_a_complete_package() {
    let version = semver::Version::new(0, 3, 7);
    assert_eq!(
        Format::Deb.installed_version("installed 0.3.7"),
        Some(version.clone())
    );
    assert_eq!(Format::Rpm.installed_version("0.3.7"), Some(version));
    assert_eq!(Format::Deb.installed_version("unpacked 0.3.7"), None);
    assert_eq!(Format::Rpm.installed_version("0.3.70.3.7"), None);
}
