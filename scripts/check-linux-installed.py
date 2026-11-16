"""Exercise a normal Linux app through AT-SPI, with a preserved private profile.

Requires python3-gi, gir1.2-atspi-2.0, at-spi2-core and a desktop/session bus.
Uses native accessibility actions, never WebView evaluation or test commands.
Mode/editor exposure is not physical input, speech, paint or parse completion.
Optional idle samples measure app/observed-child RSS/PSS and process CPU ticks;
leader-thread context switches are not wakeups or energy.
"""
import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "benchmarks"))
from process_tree import processes, descendants
from linux_rusage import sample, interval


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--document", type=Path, default=Path("tests/fixtures/specimen.md"))
    parser.add_argument("--heading", default="A place for unfinished thoughts")
    parser.add_argument("--cycles", type=int, default=1)
    parser.add_argument("--source-only", action="store_true", help="cycle Source/Read, allowing fixtures up to 128 MiB")
    parser.add_argument("--tabs", type=int, default=1, help="open 1–4 documents; extra copies live only in the isolated test profile")
    parser.add_argument("--settle-seconds", type=float, default=0, help="wait 0–60 seconds in each mode before sampling")
    parser.add_argument("--example", action="store_true", help="also open/close the bundled example through the normal UI")
    parser.add_argument("--desktop-integration", action="store_true", help="also verify the installed launcher icon through GTK")
    parser.add_argument("--idle-samples", type=int, default=0)
    parser.add_argument("--idle-interval", type=float, default=60)
    parser.add_argument("--output", type=Path, default=Path("artifacts/linux-installed-ui.json"))
    args = parser.parse_args()
    if sys.platform != "linux" or not 1 <= args.cycles <= 20 or not 0 <= args.idle_samples <= 6 or not 0 < args.idle_interval <= 60 or not 1 <= args.tabs <= 4 or not 0 <= args.settle_seconds <= 60:
        parser.error("Linux, 1–20 cycles, 1–4 tabs, 0–6 idle samples and bounded intervals up to 60 seconds required")
    binary, document = args.binary.resolve(), args.document.resolve()
    limit = (128 if args.source_only else 2) * 1024 * 1024
    if not binary.is_file() or not document.is_file() or document.stat().st_size > limit:
        parser.error("select an executable and a fixture within the selected 2/128 MiB mode limit")
    before = processes()
    if any(row["name"] in (str(binary), binary.name[:15]) for row in before.values()):
        parser.error("close other Marktub instances before starting this qualification")
    try:
        import gi
    except ModuleNotFoundError:
        parser.error("run with /usr/bin/python3 and install the documented GI/AT-SPI development prerequisites")
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi
    Atspi.set_timeout(1000, 3000)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="marktub-installed-ui-"))
    environment = dict(os.environ)
    for variable, directory in [("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache")]:
        path = profile / directory
        path.mkdir(mode=0o700)
        environment[variable] = str(path)
    documents = [document]
    for index in range(1, args.tabs):
        copy = profile / f"tab-{index}-{document.name}"
        shutil.copyfile(document, copy)
        documents.append(copy)
    document_hashes = {str(path): digest(path) for path in documents}
    result = {"status": "running", "scope": __doc__, "binarySha256": digest(binary),
              "documentSha256": digest(document), "profileDirectory": str(profile),
              "cycles": args.cycles, "tabs": args.tabs, "sourceOnly": args.source_only,
              "settleSeconds": args.settle_seconds, "documentHashes": document_hashes,
              "bundledExampleRequested": args.example, "observations": [], "samples": []}
    if args.desktop_integration:
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gio, Gtk
        Gtk.init([])
        launcher = Gio.DesktopAppInfo.new("Marktub.desktop")
        assert launcher and launcher.get_string("Icon") == "marktub", "Installed launcher icon is not registered"
        icon = Gtk.IconTheme.get_default().lookup_icon("marktub", 128, 0)
        assert icon and icon.get_filename(), "GTK cannot resolve the installed Marktub icon"
        result["desktopIntegration"] = {"desktopId": launcher.get_id(), "iconFile": icon.get_filename(), "iconSha256": digest(Path(icon.get_filename()))}
    started = time.monotonic()
    helpers = {}
    previous = None
    previous_time = None
    previous_errors = []
    app_log = args.output.with_suffix(".app.log").open("w")
    app = subprocess.Popen([str(binary), *(str(path) for path in documents)], env=environment, stdin=subprocess.DEVNULL, stdout=app_log, stderr=app_log)

    def tree(node, depth=0, rows=None):
        if rows is None:
            rows = []
        if depth > 16 or len(rows) >= 1000:
            return rows
        try:
            rows.append((node, node.get_name(), node.get_role_name(), node.get_state_set()))
            for index in range(min(node.get_child_count(), 160)):
                if len(rows) >= 1000:
                    break
                tree(node.get_child_at_index(index), depth + 1, rows)
        except Exception:
            pass  # Stale accessible objects are retried by the bounded waiter.
        return rows

    def application():
        desktop = Atspi.get_desktop(0)
        for index in range(desktop.get_child_count()):
            node = desktop.get_child_at_index(index)
            if node.get_process_id() == app.pid:
                return node
        return None

    def wait(predicate):
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError("normal app exited or handed off before UI verification")
            root = application()
            if root is not None:
                rows = tree(root)
                if predicate(rows):
                    return rows
            time.sleep(0.2)
        raise RuntimeError("accessible UI condition did not become ready within 40 seconds")

    def observe(label, rows):
        result["observations"].append({"label": label, "elapsedSeconds": time.monotonic() - started,
            "nodes": [{"name": name[:200], "role": role,
                       "pressed": state.contains(Atspi.StateType.PRESSED),
                       "enabled": state.contains(Atspi.StateType.ENABLED)} for _, name, role, state in rows]})

    def measure(label):
        nonlocal previous, previous_time, previous_errors
        table = processes()
        if app.poll() is not None or app.pid not in table:
            raise RuntimeError("app exited before resource capture")
        selected = (descendants(table, app.pid) | helpers.keys()) & table.keys()
        counters, errors = [], []
        for pid in sorted(selected):
            try:
                current = sample(pid)
                if current["startTicks"] != table[pid]["startTicks"] or (pid in helpers and helpers[pid] != current["startTicks"]):
                    raise OSError("process incarnation changed during ownership/counter capture")
                counters.append(current)
                if pid != app.pid:
                    helpers[pid] = current["startTicks"]
            except OSError as error:
                errors.append({"pid": pid, "error": str(error)})
        now = time.monotonic()
        measured = {"label": label, "elapsedSeconds": now - started, "processCounters": counters,
                    "counterErrors": errors, "rssSumBytes": sum(c["rssBytes"] for c in counters),
                    "pssBytes": sum(c["pssBytes"] for c in counters), "pssIncomplete": bool(errors)}
        if previous is not None:
            measured["counterInterval"] = interval(previous, counters, now - previous_time)
            measured["counterInterval"]["complete"] = not errors and not previous_errors and not measured["counterInterval"]["incompletePids"] and bool(counters)
        previous, previous_time, previous_errors = counters, now, errors
        result["samples"].append(measured)
        print(json.dumps({"label": label, "pssBytes": measured["pssBytes"], "counterErrors": errors}), flush=True)

    failed = False
    try:
        rows = wait(lambda rows: any(args.heading in name for _, name, _, _ in rows))
        if args.tabs > 1:
            rows = wait(lambda rows: all(any(name == f"Close {path.name}" for _, name, _, _ in rows) for path in documents))
        observe("read-heading", rows)
        measure("read-heading")
        for cycle in range(args.cycles):
            for mode in (["Source", "Read"] if args.source_only else ["Source", "Split", "Visual", "Read"]):
                rows = wait(lambda rows: any(name == mode and state.contains(Atspi.StateType.ENABLED) for _, name, _, state in rows))
                node = next(n for n, name, _, state in rows if name == mode and state.contains(Atspi.StateType.ENABLED))
                assert node.get_n_actions() > 0 and node.do_action(0)
                rows = wait(lambda rows: any(name == mode and state.contains(Atspi.StateType.PRESSED) for _, name, _, state in rows)
                            and (mode == "Read" or any(state.contains(Atspi.StateType.EDITABLE) for _, _, _, state in rows)))
                label = f"cycle-{cycle + 1}-{mode}"
                time.sleep(args.settle_seconds)
                observe(label, rows)
                measure(label)
        for path in reversed(documents):
            close = f"Close {path.name}"
            rows = wait(lambda rows: any(name == close and state.contains(Atspi.StateType.ENABLED) for _, name, _, state in rows))
            node = next(n for n, name, _, _ in rows if name == close)
            assert node.do_action(0)
            wait(lambda rows: not any(name == close for _, name, _, _ in rows))
        rows = wait(lambda rows: any("A little room to read" in name for _, name, _, _ in rows))
        observe("closed-home", rows)
        measure("closed-home")
        if args.example:
            rows = wait(lambda rows: any(name == "Read the example" and state.contains(Atspi.StateType.ENABLED) for _, name, _, state in rows))
            node = next(n for n, name, _, _ in rows if name == "Read the example")
            assert node.do_action(0)
            rows = wait(lambda rows: any("A place for unfinished thoughts" in name for _, name, _, _ in rows))
            observe("bundled-example", rows)
            measure("bundled-example")
            rows = wait(lambda rows: any(name == "Close Marktub Guide.md" and state.contains(Atspi.StateType.ENABLED) for _, name, _, state in rows))
            node = next(n for n, name, _, _ in rows if name == "Close Marktub Guide.md")
            assert node.do_action(0)
            rows = wait(lambda rows: any("A little room to read" in name for _, name, _, _ in rows))
            observe("closed-example-home", rows)
            measure("closed-example-home")
        for index in range(args.idle_samples):
            time.sleep(args.idle_interval)
            measure(f"closed-idle-{index + 1}")
    except Exception as error:
        failed = True
        result["error"] = str(error)
        root = application()
        if root is not None:
            observe("failure", tree(root))
    finally:
        exit_method = "already exited"
        if app.poll() is None:
            app.terminate()
            exit_method = "external SIGTERM"
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            app.kill()
            app.wait()
            exit_method = "external SIGKILL after five-second SIGTERM deadline"
        app_log.close()
        deadline = time.monotonic() + 8
        remaining = dict(helpers)
        while remaining and time.monotonic() < deadline:
            table = processes()
            remaining = {pid: ticks for pid, ticks in remaining.items() if pid in table and table[pid]["startTicks"] == ticks}
            if remaining:
                time.sleep(0.2)
        result["exitMethod"] = exit_method
        result["cleanHomeObserved"] = any(o["label"] == "closed-home" for o in result["observations"])
        result["helpersRemainingAfterExit"] = sorted(remaining)
        result["sampledHelperLifecycleConfirmed"] = bool(helpers) and not remaining
        result["documentUnchanged"] = digest(document) == result["documentSha256"]
        result["allDocumentsUnchanged"] = all(digest(Path(path)) == expected for path, expected in document_hashes.items())
        result["binaryUnchanged"] = digest(binary) == result["binarySha256"]
        result["allCounterIntervalsComplete"] = all(s["counterInterval"]["complete"] for s in result["samples"] if "counterInterval" in s)
        failed |= not result["allDocumentsUnchanged"] or not result["binaryUnchanged"] or not result["sampledHelperLifecycleConfirmed"]
        failed |= any(s["counterErrors"] for s in result["samples"])
        result["status"] = "fail" if failed else "pass"
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ["status", "binarySha256", "documentUnchanged", "sampledHelperLifecycleConfirmed"]}))
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
