"""Close only the disposable Evince document opened by the native export test.

Uses the viewer's ordinary D-Bus window action, never compositor focus overrides.
The document URI, process arguments/owner and single-window state must all match.
"""
import os
from pathlib import Path
import re
import sys
import tempfile
import time
import xml.etree.ElementTree as ET

from gi.repository import Gio, GLib


def main():
    document = Path(sys.argv[1]).resolve(strict=True)
    assert re.fullmatch(r"marktub-e2e-[0-9a-f-]{36}", document.parent.name)
    assert document.parent.parent == Path(tempfile.gettempdir()).resolve()
    assert document.name in {"marktub-export-export.pdf", "marktub-bundled-fonts.pdf"}
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(name, path, interface, method, signature=None, values=None):
        arguments = GLib.Variant(signature, values) if signature else None
        return bus.call_sync(name, path, interface, method, arguments, None,
                             Gio.DBusCallFlags.NO_AUTO_START, 1000, None).unpack()

    deadline = time.monotonic() + 45
    owner = ""
    while not owner and time.monotonic() < deadline:
        try:
            owner, = call("org.gnome.evince.Daemon", "/org/gnome/evince/Daemon",
                          "org.gnome.evince.Daemon", "FindDocument", "(sb)",
                          (document.as_uri(), False))
        except GLib.Error:
            pass
        if not owner:
            time.sleep(0.1)
    assert owner.startswith(":"), "The exported PDF was not registered by Evince"
    pid, = call("org.freedesktop.DBus", "/org/freedesktop/DBus",
                "org.freedesktop.DBus", "GetConnectionUnixProcessID", "(s)", (owner,))
    process = Path("/proc") / str(pid)
    assert process.stat().st_uid == os.getuid()
    arguments = (process / "cmdline").read_bytes().split(b"\0")
    assert os.fsencode(document) in arguments or document.as_uri().encode() in arguments
    windows, = call(owner, "/org/gnome/evince/Evince", "org.gnome.evince.Application", "GetWindowList")
    assert len(windows) == 1, "Refuse to close a viewer with other windows"

    # GTK exports ordinary window actions separately from Evince's document API.
    pending = ["/org/gnome/Evince"]
    candidates = []
    visited = 0
    while pending:
        path = pending.pop()
        visited += 1
        assert visited <= 32, "Unexpected viewer D-Bus tree"
        xml, = call(owner, path, "org.freedesktop.DBus.Introspectable", "Introspect")
        node = ET.fromstring(xml)
        if any(i.get("name") == "org.gtk.Actions" for i in node.findall("interface")):
            actions, = call(owner, path, "org.gtk.Actions", "List")
            if "close" in actions:
                candidates.append(path)
        pending.extend(path + "/" + child.attrib["name"] for child in node.findall("node"))
    assert len(candidates) == 1, "No unique viewer close action"
    call(owner, candidates[0], "org.gtk.Actions", "Activate", "(sava{sv})", ("close", [], {}))
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            windows, = call(owner, "/org/gnome/evince/Evince", "org.gnome.evince.Application", "GetWindowList")
            if not windows:
                break
        except GLib.Error:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError("Viewer window did not close")
    print("PASS: exported PDF opened in Evince; its sole owned window closed normally")


if __name__ == "__main__":
    main()
