"""Process-cold/page-cache-uncontrolled launch to WebView page-load event.

This is not an input-to-paint or first-rich-document benchmark.
"""
import argparse
import json
import math
import statistics
import hashlib
from pathlib import Path
import selectors
import subprocess
import time
import os
import platform
from process_tree import processes, descendants

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", required=True)
parser.add_argument("--runs", type=int, default=20)
parser.add_argument("--cooldown", type=float, default=0, help="seconds after each process exits; caches still uncontrolled")
parser.add_argument("--output", type=Path, default=Path("artifacts/startup.json"))
args = parser.parse_args()
if args.runs < 1 or args.cooldown < 0:
    parser.error("runs must be positive and cooldown nonnegative")
binary_hash = hashlib.sha256(Path(args.binary).read_bytes()).hexdigest()
samples = []
for index in range(args.runs):
    start = time.monotonic()
    process = subprocess.Popen([str(Path(args.binary).resolve())], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env={**os.environ, "MARKTUB_PROFILE_STARTUP": "1"})
    selector = selectors.DefaultSelector()
    selector.register(process.stderr, selectors.EVENT_READ)
    loaded = None
    pending = b""
    try:
        deadline = start + 15
        while loaded is None and time.monotonic() < deadline and process.poll() is None:
            if not selector.select(timeout=0.25):
                continue
            chunk = os.read(process.stderr.fileno(), 65536)
            if not chunk: break
            pending += chunk
            while b"\n" in pending:
                raw, _, pending = pending.partition(b"\n")
                line = raw.decode("utf-8", errors="replace")
                if line.startswith("MARKTUB_PAGE_LOADED_US="):
                    loaded = {"run": index, "launchToPageLoadMs": (time.monotonic() - start) * 1000, "rustToPageLoadMs": int(line.split("=")[1]) / 1000}
                    break
            # Framework noise is not part of the closed timing-line protocol.
            pending = pending[-65536:]
        if loaded is None:
            raise RuntimeError("App did not report page load within 15 seconds")
        samples.append(loaded)
    finally:
        selector.close()
        table = processes()
        helpers = {pid: table[pid]["startTicks"] for pid in descendants(table, process.pid)
                   if pid != process.pid and pid in table and "startTicks" in table[pid]}
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        deadline = time.monotonic() + 5
        remaining = dict(helpers)
        while remaining and time.monotonic() < deadline:
            table = processes()
            remaining = {pid: ticks for pid, ticks in remaining.items()
                         if pid in table and table[pid]["startTicks"] == ticks}
            if remaining:
                time.sleep(0.1)
        if loaded is not None:
            loaded["observedHelperPids"] = sorted(helpers)
            loaded["helpersRemainingAfterExit"] = sorted(remaining)
            loaded["observedHelperLifecycleConfirmed"] = bool(helpers) and not remaining
        if remaining:
            raise RuntimeError("Observed startup helpers survived the app exit")
    time.sleep(args.cooldown)
values = sorted(row["launchToPageLoadMs"] for row in samples)
result = {"metric": "launch-to-WebView-page-load, not first interactive paint", "cacheState": "process cold; filesystem cache uncontrolled/warm", "binarySha256": binary_hash, "cooldownSeconds": args.cooldown, "samples": samples, "medianMs": statistics.median(values), "p95Ms": values[math.ceil(len(values)*0.95)-1], "percentileMethod": "nearest rank"}
result["host"] = {"system": platform.system(), "release": platform.release(), "machine": platform.machine()}
result["helperScope"] = "Linux descendants observed at page load, pinned by PID/startTicks; other hosts and helpers launched outside the tree are not attributed"
result["binaryUnchanged"] = hashlib.sha256(Path(args.binary).read_bytes()).hexdigest() == binary_hash
if not result["binaryUnchanged"]:
    raise RuntimeError("Executable changed during startup qualification")
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({key: value for key, value in result.items() if key != "samples"}, indent=2))
