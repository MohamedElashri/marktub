"""Serialized ABBA launch-to-page-load control; no interactive-paint claim."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import subprocess
import tempfile

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--before", type=Path, required=True)
p.add_argument("--after", type=Path, required=True)
p.add_argument("--runs", type=int, default=20, help="runs per binary, positive and even")
p.add_argument("--cooldown", type=float, default=2)
p.add_argument("--output", type=Path, required=True)
a = p.parse_args()
if a.runs < 2 or a.runs % 2 or a.cooldown < 0: p.error("positive even runs and nonnegative cooldown required")
samples = {"before": [], "after": []}
order = ["before", "after", "after", "before"] * (a.runs // 2)
with tempfile.TemporaryDirectory(prefix="marktub-startup-compare-") as temp:
    for index, label in enumerate(order):
        output = Path(temp) / "sample.json"
        subprocess.run(["python3", "benchmarks/startup.py", "--binary", str(getattr(a,label)), "--runs", "1", "--cooldown", str(a.cooldown), "--stages", "--output", str(output)], check=True, stdout=subprocess.DEVNULL)
        sample = json.loads(output.read_text())["samples"][0]
        sample["order"] = index
        samples[label].append(sample)
summary = {}
for label, rows in samples.items():
    values = sorted(row["launchToPageLoadMs"] for row in rows)
    summary[label] = {"binarySha256": hashlib.sha256(getattr(a,label).read_bytes()).hexdigest(), "medianMs": statistics.median(values), "p95Ms": values[math.ceil(len(values)*.95)-1]}
result = {"metric": "process launch to WebView page-load; not first interactive paint", "method": "fixed ABBA order, serialized, filesystem/XPC caches uncontrolled", "percentileMethod": "nearest rank", "cooldownSeconds": a.cooldown, "order": order, "summary": summary, "samples": samples}
a.output.parent.mkdir(parents=True, exist_ok=True)
a.output.write_text(json.dumps(result,indent=2)+"\n")
print(json.dumps(summary,indent=2))
