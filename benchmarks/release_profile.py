"""External production-process sampling; no WebView evaluator or app hooks.

Native open requests select an admitted workload. Samples at fixed times do not
certify useful viewport, completed parsing, compositor frames or input latency.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
from process_tree import processes, descendants

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--binary", required=True)
p.add_argument("--document", type=Path)
p.add_argument("--label", required=True)
p.add_argument("--samples", type=int, default=3)
p.add_argument("--interval", type=float, default=5)
p.add_argument("--output", type=Path, required=True)
p.add_argument("--rusage", action="store_true", help="per-process CPU and incarnation counters; macOS wakeups or Linux context switches, not energy")
a = p.parse_args()
if a.samples < 1 or a.interval <= 0:
    p.error("positive sample count and interval required")
if a.rusage and sys.platform not in ("darwin", "linux"):
    p.error("--rusage requires macOS or Linux")
a.output.parent.mkdir(parents=True, exist_ok=True)
binary = Path(a.binary).resolve()
before = processes()
if any(row["name"] == str(binary) or (sys.platform == "linux" and row["name"] == binary.name[:15])
       for row in before.values()):
    p.error("the production app is already running; do not measure a handoff process")
doc = a.document.resolve() if a.document else None
document_hash = hashlib.sha256(doc.read_bytes()).hexdigest() if doc else None
result = {"profile": "normal release; external fixed-time samples; no interactive/parse-completion certification", "platform": sys.platform, "label": a.label, "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "documentBytes": doc.stat().st_size if doc else None, "documentSha256": document_hash, "samples": [], "notMeasured": ["CPU wakeups", "DOM", "IPC", "frame latency", "input-to-paint"]}
if a.rusage:
    if sys.platform == "darwin":
        from macos_rusage import sample as rusage, interval as rusage_interval
        result["notMeasured"].remove("CPU wakeups")
        result["counterScope"] = "proc_pid_rusage RUSAGE_INFO_V0 user/system nanoseconds, package-idle and interrupt wakeups; actual process interval deltas; not energy or all timer/UI wakeups"
    else:
        from linux_rusage import sample as rusage, interval as rusage_interval
        result["counterScope"] = "Linux /proc starttime-bracketed process CPU ticks and RSS/PSS; leader-thread context switches, not whole-thread wakeups or energy"
previous_counters = None
previous_counter_time = None
previous_counter_errors = []
start = time.monotonic()
process = subprocess.Popen([str(binary)] + ([str(doc)] if doc else []), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
helpers = set()
linux_incarnations = {}
ownership_errors = []
try:
    for index in range(a.samples):
        time.sleep(max(0, start + (index + 1) * a.interval - time.monotonic()))
        table = processes()
        if process.poll() is not None or process.pid not in table:
            raise RuntimeError("app exited/handed off before measurement")
        helpers |= {pid for pid,row in table.items() if pid not in before and "com.apple.WebKit." in row["name"]}
        selected = (descendants(table, process.pid) | helpers) & table.keys()
        if sys.platform != "darwin":
            helpers |= selected - {process.pid}
        if sys.platform == "linux":
            for pid in selected:
                current = table[pid]["startTicks"]
                if pid in linux_incarnations and linux_incarnations[pid] != current:
                    ownership_errors.append({"sample": index, "pid": pid, "reason": "PID reused; whole-app attribution refused"})
                linux_incarnations.setdefault(pid, current)
        cpu = {}
        for line in subprocess.check_output(["ps", "-axo", "pid=,time="], text=True).splitlines():
            fields = line.split()
            if len(fields) == 2 and int(fields[0]) in selected:
                cpu[fields[0]] = fields[1]
        sample = {"elapsedSeconds": time.monotonic() - start, "processes": [table[pid] for pid in sorted(selected)], "rssSumBytes": sum(table[pid]["rssBytes"] for pid in selected), "cumulativeCpuTimes": cpu}
        if a.rusage:
            counters = []
            errors = []
            for pid in sorted(selected):
                try:
                    counter = rusage(pid)
                    if sys.platform == "linux" and counter["startTicks"] != table[pid]["startTicks"]:
                        raise OSError("PID changed between ancestry and counter capture")
                    counters.append(counter)
                except OSError as error: errors.append({"pid": pid, "error": str(error)})
            counter_time = time.monotonic()
            sample["processCounters"] = counters
            sample["counterErrors"] = errors
            sample["counterElapsedSeconds"] = counter_time - start
            if previous_counters is not None:
                sample["counterInterval"] = rusage_interval(previous_counters, counters, counter_time - previous_counter_time)
                sample["counterInterval"]["complete"] = not errors and not previous_counter_errors and not sample["counterInterval"]["incompletePids"] and bool(counters)
            previous_counters, previous_counter_time = counters, counter_time
            previous_counter_errors = errors
        if sys.platform == "darwin":
            raw = a.output.with_name(f"{a.output.stem}-{index}.footprint.json")
            command = ["footprint", "--noCategories", "-j", str(raw)]
            for pid in sorted(selected): command.extend(["-p", str(pid)])
            measured = subprocess.run(command, capture_output=True, text=True)
            sample["footprintExitCode"] = measured.returncode
            if measured.returncode == 0:
                footprint = json.loads(raw.read_text())
                sample["physicalFootprintBytes"] = footprint["total footprint"]
                sample["footprintErrors"] = footprint.get("errors", [])
        else:
            sample["pssBytes"] = 0
            for pid in selected:
                try:
                    sample["pssBytes"] += sum(int(line.split()[1])*1024 for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines() if line.startswith("Pss:"))
                except (FileNotFoundError, PermissionError): sample["pssIncomplete"] = True
            if a.rusage:
                sample["pssIncomplete"] = bool(errors)
                sample["pssBytes"] = sum(row["pssBytes"] for row in counters)
                sample["rssSumBytes"] = sum(row["rssBytes"] for row in counters)
        result["samples"].append(sample)
finally:
    process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill(); process.wait()
deadline = time.monotonic() + 8
remaining = helpers
while remaining and time.monotonic() < deadline:
    time.sleep(0.2)
    remaining &= processes().keys()
result["helpersRemainingAfterExit"] = sorted(remaining)
result["lifecycleAttributionConfirmed"] = not remaining and (sys.platform != "darwin" or len(helpers) >= 2)
if sys.platform == "linux":
    result["ownershipErrors"] = ownership_errors
    result["lifecycleAttributionConfirmed"] &= not ownership_errors
result["documentUnchanged"] = not doc or hashlib.sha256(doc.read_bytes()).hexdigest() == document_hash
a.output.write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({k:v for k,v in result.items() if k != "samples"}))
