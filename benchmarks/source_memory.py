"""Sample an observed native app and verify new WebKit helper lifetimes.
The default profile describes debug e2e; normal release observations must supply
their own profile label. Tooling only, never app code. RSS and footprint differ.
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from process_tree import processes, descendants

BASE = Path("artifacts/process-baseline.json")
OUT = Path("artifacts/webview-memory.json")
p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--baseline", action="store_true")
p.add_argument("--finalize", action="store_true")
p.add_argument("--pid", type=int)
p.add_argument("--label")
p.add_argument("--output", type=Path, default=OUT)
p.add_argument("--baseline-file", type=Path, default=BASE)
p.add_argument("--profile", default="debug e2e; warm lazy imports; uncontrolled GC and fixture IPC transients")
p.add_argument("--rusage", action="store_true", help="per-process CPU and incarnation counters for an observed native workload")
p.add_argument("--responsibility", action="store_true", help="macOS private SPI proof to exclude positively foreign helpers; lifecycle still required")
a = p.parse_args()
if a.responsibility and sys.platform != "darwin":
    p.error("--responsibility requires macOS")
if a.rusage and sys.platform not in ("darwin", "linux"):
    p.error("--rusage requires macOS or Linux")
BASE, OUT = a.baseline_file, a.output
BASE.parent.mkdir(parents=True, exist_ok=True)
OUT.parent.mkdir(parents=True, exist_ok=True)
if a.baseline:
    BASE.write_text(json.dumps(processes()))
    OUT.unlink(missing_ok=True)
elif a.finalize:
    if OUT.exists():
        data = json.loads(OUT.read_text())
        deadline = time.monotonic() + 8
        expected = {pid for sample in data["samples"] for pid in sample["helperPids"]} | {sample["appPid"] for sample in data["samples"]}
        remaining = processes()
        while expected & remaining.keys() and time.monotonic() < deadline:
            time.sleep(0.2)
            remaining = processes()
        for sample in data["samples"]:
            sample["helpersRemainingAfterExit"] = [pid for pid in sample["helperPids"] if pid in remaining]
            sample["lifecycleAttributionConfirmed"] = sample["appPid"] not in remaining and not sample["helpersRemainingAfterExit"] and (sys.platform != "darwin" or len(sample["helperPids"]) >= 2) and sample.get("ownership", {}).get("complete", True)
        OUT.write_text(json.dumps(data, indent=2) + "\n")
else:
    baseline = {int(pid) for pid in json.loads(BASE.read_text())}
    table = processes()
    if a.pid not in table:
        raise SystemExit("Measured app is not running")
    helpers = {pid for pid, row in table.items() if pid not in baseline and "com.apple.WebKit." in row["name"]}
    candidates = set(helpers)
    ownership = None
    if a.responsibility:
        from macos_ownership import select
        candidates = {pid for pid, row in table.items() if "com.apple.WebKit." in row["name"]}
        helpers, ownership = select(a.pid, candidates, descendants(table, a.pid), preexisting=baseline)
    selected = descendants(table, a.pid) | helpers
    if sys.platform != "darwin":
        helpers = selected - {a.pid}
    rows = [table[pid] for pid in sorted(selected) if pid in table]
    sample = {"label": a.label, "appPid": a.pid, "helperPids": sorted(helpers), "processes": rows, "rssSumBytes": sum(row["rssBytes"] for row in rows)}
    if a.rusage:
        if sys.platform == "darwin":
            from macos_rusage import sample as rusage, interval
            counter_scope = "proc_pid_rusage v0 + mach_timebase_info, package-idle/interrupt wakeups; not energy, timers or UI input latency"
        else:
            from linux_rusage import sample as rusage, interval
            counter_scope = "Linux /proc CPU ticks and RSS/PSS; leader-thread context switches, not whole-thread wakeups or energy"
        sample["processCounters"] = []
        sample["counterErrors"] = []
        for pid in sorted(selected):
            try:
                counter = rusage(pid)
                if sys.platform == "linux" and counter["startTicks"] != table[pid]["startTicks"]:
                    raise OSError("PID changed before counter capture")
                sample["processCounters"].append(counter)
            except OSError as error: sample["counterErrors"].append({"pid": pid, "error": str(error)})
        sample["counterMonotonicSeconds"] = time.monotonic()
    if sys.platform == "darwin":
        raw = OUT.with_name(f"{OUT.stem}-footprint-{a.label}.json")
        args = ["footprint", "--noCategories", "-j", str(raw)]
        for pid in sorted(selected):
            args.extend(["-p", str(pid)])
        result = subprocess.run(args, capture_output=True, text=True)
        sample["footprintExitCode"] = result.returncode
        if result.returncode == 0:
            data = json.loads(raw.read_text())
            sample["physicalFootprintBytes"] = data["total footprint"]
            sample["footprintErrors"] = data.get("errors", [])
    if ownership is not None:
        from macos_ownership import verify
        after = processes()
        current = {pid for pid, row in after.items() if "com.apple.WebKit." in row["name"]}
        sample["ownership"] = verify(ownership, current)
    data = json.loads(OUT.read_text()) if OUT.exists() else {"profile": a.profile, "platform": sys.platform, "samples": []}
    if a.rusage:
        data["counterScope"] = counter_scope
        previous = data["samples"][-1] if data["samples"] else None
        if previous and "processCounters" in previous:
            sample["counterInterval"] = interval(previous["processCounters"], sample["processCounters"], sample["counterMonotonicSeconds"] - previous["counterMonotonicSeconds"])
            sample["counterInterval"]["complete"] = not sample["counterErrors"] and not previous["counterErrors"] and not sample["counterInterval"]["incompletePids"] and bool(sample["processCounters"]) and sample.get("ownership", {}).get("complete", True) and previous.get("ownership", {}).get("complete", True)
    data["samples"].append(sample)
    OUT.write_text(json.dumps(data, indent=2) + "\n")
