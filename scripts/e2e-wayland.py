"""Run the built native E2E app as a Wayland client in an isolated compositor.

Use xvfb-run -a dbus-run-session -- python3 scripts/e2e-wayland.py.
Weston's X11 backend supplies a private input seat; the app uses GDK Wayland.
Native focus assertions remain mandatory. This does not certify GNOME policy,
physical IME, accessibility or compositor/input-to-paint performance.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time


def main():
    if not os.environ.get("DISPLAY"):
        raise SystemExit("Run under xvfb-run -a dbus-run-session; Weston needs a private X11 input seat.")
    weston = shutil.which("weston")
    node = shutil.which("node")
    if not weston or not node:
        raise SystemExit("Install Weston and the pinned Node before running Wayland checks.")
    runtime = Path(tempfile.mkdtemp(prefix="marktub-wayland-"))
    runtime.chmod(0o700)
    artifacts = Path("artifacts")
    artifacts.mkdir(exist_ok=True)
    environment = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), WAYLAND_DISPLAY="marktub-wayland", GDK_BACKEND="wayland")
    command = [weston, "--backend=x11", "--renderer=pixman", "--shell=" + os.environ.get("MARKTUB_WESTON_SHELL", "kiosk-shell.so"), "--socket=marktub-wayland", "--width=1280", "--height=900", "--idle-time=0", "--no-config"]
    with (artifacts / "wayland-compositor.log").open("w") as log:
        compositor = subprocess.Popen(command, env=environment, stdout=log, stderr=log, start_new_session=True)
        try:
            deadline = time.monotonic() + 10
            while True:
                if compositor.poll() is not None:
                    raise RuntimeError("Weston exited; inspect artifacts/wayland-compositor.log.")
                try:
                    with socket.socket(socket.AF_UNIX) as connection:
                        connection.connect(str(runtime / "marktub-wayland"))
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError("Weston did not become ready within 10 seconds.") from None
                    time.sleep(0.05)
            result = subprocess.run([node, "scripts/e2e-run.mjs"], env=environment)
            (artifacts / "wayland-environment.json").write_text(json.dumps({
                "compositor": subprocess.check_output([weston, "--version"], env=environment, text=True).strip(),
                "nestedBackend": "x11",
                "clientBackend": "wayland",
                "runtimeDirectory": str(runtime),
                "exitCode": result.returncode,
                "scope": __doc__,
            }, indent=2) + "\n")
            return result.returncode
        finally:
            if compositor.poll() is None:
                os.killpg(compositor.pid, signal.SIGTERM)
            try:
                compositor.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(compositor.pid, signal.SIGKILL)
                compositor.wait()


if __name__ == "__main__":
    raise SystemExit(main())
