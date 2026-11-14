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

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", required=True)
parser.add_argument("--runs", type=int, default=20)
parser.add_argument("--cooldown", type=float, default=0, help="seconds after each process exits; caches still uncontrolled")
parser.add_argument("--output", type=Path, default=Path("artifacts/startup.json"))
parser.add_argument("--stages", action="store_true", help="opt-in native timing markers only; no evaluator")
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
    stages = {}
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
                if line.startswith("MARKTUB_STARTUP_STAGE="):
                    name, elapsed = line.split("=", 1)[1].rsplit(":", 1)
                    stages[name] = int(elapsed) / 1000
                if line.startswith("MARKTUB_PAGE_LOADED_US="):
                    loaded = {"run": index, "launchToPageLoadMs": (time.monotonic() - start) * 1000, "rustToPageLoadMs": int(line.split("=")[1]) / 1000}
                    if args.stages and stages: loaded["nativeStagesMs"] = stages
                    break
            # Framework noise is not part of the closed timing-line protocol.
            pending = pending[-65536:]
        if loaded is None:
            raise RuntimeError("App did not report page load within 15 seconds")
        samples.append(loaded)
    finally:
        selector.close()
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    time.sleep(args.cooldown)
values = sorted(row["launchToPageLoadMs"] for row in samples)
result = {"metric": "launch-to-WebView-page-load, not first interactive paint", "cacheState": "process cold; filesystem cache uncontrolled/warm", "binarySha256": binary_hash, "cooldownSeconds": args.cooldown, "samples": samples, "medianMs": statistics.median(values), "p95Ms": values[math.ceil(len(values)*0.95)-1], "percentileMethod": "nearest rank"}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({key: value for key, value in result.items() if key != "samples"}, indent=2))
