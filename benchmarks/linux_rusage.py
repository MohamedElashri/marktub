"""External Linux /proc counters. Leader-thread switches are not wakeups or energy."""
import os
from pathlib import Path


def stat(text):
    # comm may contain spaces and parentheses; the final ')' ends field 2.
    closing = text.rfind(")")
    fields = text[closing + 2:].split()
    if closing < 0 or len(fields) < 22:
        raise ValueError("Incomplete process stat")
    return {"pid": int(text.split("(", 1)[0]), "parent": int(fields[1]),
            "startTicks": int(fields[19]), "userTicks": int(fields[11]),
            "systemTicks": int(fields[12])}


def sample(pid):
    root = Path(f"/proc/{pid}")
    before = stat((root / "stat").read_text())
    status = (root / "status").read_text().splitlines()
    context = {key: int(line.split(":", 1)[1]) for line in status
               for key in [line.split(":", 1)[0]]
               if key in ("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")}
    if len(context) != 2:
        raise OSError("Incomplete context-switch counters")
    memory = {}
    for line in (root / "smaps_rollup").read_text().splitlines():
        if line.startswith(("Pss:", "Rss:")):
            memory[line.split(":", 1)[0].lower() + "Bytes"] = int(line.split()[1]) * 1024
    after = stat((root / "stat").read_text())
    if before["startTicks"] != after["startTicks"] or len(memory) != 2:
        raise OSError("Process incarnation changed or memory counters are incomplete")
    return {**after, **context, **memory, "clockTicksPerSecond": os.sysconf("SC_CLK_TCK")}


def interval(before, after, seconds):
    if seconds <= 0:
        raise ValueError("Positive interval required")
    before = {row["pid"]: row for row in before}
    after = {row["pid"]: row for row in after}
    incomplete = set(before) ^ set(after)
    deltas = []
    for pid in before.keys() & after.keys():
        old, new = before[pid], after[pid]
        if (old["startTicks"], old["clockTicksPerSecond"]) != (new["startTicks"], new["clockTicksPerSecond"]):
            incomplete.add(pid)
            continue
        values = {key: new[key] - old[key] for key in ("userTicks", "systemTicks", "voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")}
        if any(value < 0 for value in values.values()):
            incomplete.add(pid)
            continue
        deltas.append({"pid": pid, **values, "cpuSeconds": (values["userTicks"] + values["systemTicks"]) / new["clockTicksPerSecond"]})
    cpu = sum(row["cpuSeconds"] for row in deltas)
    return {"seconds": seconds, "matchedProcessDeltas": sorted(deltas, key=lambda row: row["pid"]),
            "incompletePids": sorted(incomplete), "cpuSeconds": cpu,
            "cpuPercentOfOneCore": cpu / seconds * 100,
            "voluntaryContextSwitchesPerSecond": sum(row["voluntary_ctxt_switches"] for row in deltas) / seconds,
            "involuntaryContextSwitchesPerSecond": sum(row["nonvoluntary_ctxt_switches"] for row in deltas) / seconds}
