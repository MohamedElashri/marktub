"""Measure a controlled launch, including XPC helpers reparented to launchd.

macOS: attribute newly spawned WebKit processes only when their lifecycle follows
the launched app. Preserve raw PID/footprint evidence and reject ambiguous totals.
Linux: use descendant RSS/PSS. This is a measurement script, never app runtime code.
"""
import argparse
import json
import hashlib
from pathlib import Path
import subprocess
import sys
import time


def processes():
    result = {}
    for line in subprocess.check_output(["ps", "-axo", "pid,ppid,rss,comm"], text=True).splitlines()[1:]:
        parts = line.strip().split(None, 3)
        if len(parts) == 4:
            pid, parent, rss, name = parts
            result[int(pid)] = {"pid": int(pid), "parent": int(parent), "rssBytes": int(rss) * 1024, "name": name}
    if sys.platform == "linux":
        from linux_rusage import stat
        for pid in list(result):
            try:
                identity = stat(Path(f"/proc/{pid}/stat").read_text())
                if identity["parent"] != result[pid]["parent"]:
                    del result[pid]
                else:
                    result[pid]["startTicks"] = identity["startTicks"]
            except (OSError, ValueError):
                del result[pid]
    return result


def descendants(table, root):
    found = {root}
    while True:
        updated = found | {pid for pid, row in table.items() if row["parent"] in found}
        if updated == found:
            return found
        found = updated


def measure(binary, label, output):
    binary_hash = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
    before = processes()
    process = subprocess.Popen([str(Path(binary).resolve())], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(3)
        table = processes()
        if process.pid not in table or process.poll() is not None:
            raise RuntimeError("App exited before the memory measurement")
        selected = descendants(table, process.pid)
        new_helpers = {pid for pid, row in table.items() if pid not in before and "com.apple.WebKit." in row["name"]}
        selected |= new_helpers
        if sys.platform != "darwin":
            new_helpers = selected - {process.pid}
        rows = [table[pid] for pid in sorted(selected) if pid in table]
        result = {"label": label, "platform": sys.platform, "binarySha256": binary_hash, "settleSeconds": 3, "processes": rows, "rssSumBytes": sum(row["rssBytes"] for row in rows), "method": "new-process delta + descendants; verify helper exit after app termination"}
        if sys.platform == "darwin":
            raw = output.with_suffix(".footprint.json")
            args = ["footprint", "--noCategories", "-j", str(raw)]
            for pid in sorted(selected):
                args.extend(["-p", str(pid)])
            sample = subprocess.run(args, capture_output=True, text=True)
            result["footprintExitCode"] = sample.returncode
            if sample.returncode == 0:
                footprint = json.loads(raw.read_text())
                result["physicalFootprintBytes"] = footprint["total footprint"]
                result["physicalProcesses"] = footprint["processes"]
                result["footprintErrors"] = footprint.get("errors", [])
        else:
            pss = 0
            for pid in selected:
                try:
                    lines = Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
                    pss += sum(int(line.split()[1]) * 1024 for line in lines if line.startswith("Pss:"))
                except (FileNotFoundError, PermissionError):
                    result["pssIncomplete"] = True
            result["pssBytes"] = pss
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
    deadline = time.monotonic() + 8
    remaining = set(new_helpers)
    while remaining and time.monotonic() < deadline:
        time.sleep(0.2)
        remaining &= set(processes())
    result["helpersRemainingAfterExit"] = sorted(remaining)
    result["lifecycleAttributionConfirmed"] = not remaining and (sys.platform != "darwin" or len(new_helpers) >= 2)
    if not result["lifecycleAttributionConfirmed"]:
        result["limitation"] = "Helper ownership not established; totals are candidate measurements, not a verified whole-app total."
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--label", default="app-empty")
    parser.add_argument("--output", type=Path, default=Path("artifacts/memory.json"))
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    measure(args.binary, args.label, args.output)
