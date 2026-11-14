"""Read-only macOS process counters for external development profiling.

Uses the fixed RUSAGE_INFO_V0 ABI in the local SDK's sys/resource.h and
proc_pid_rusage in libproc.h. No app hooks, root access or energy estimate.
CPU absolute-time values are converted with mach_timebase_info (not assumed ns).
Package-idle and interrupt wakeup counters are distinct kernel counters, not
an exhaustive count of UI events, timer callbacks or physical energy use.
"""
import ctypes
import sys


class RusageInfoV0(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "userAbstime", "systemAbstime", "packageIdleWakeups", "interruptWakeups",
            "pageins", "wiredBytes", "residentBytes", "physicalFootprintBytes",
            "startAbstime", "exitAbstime",
        )
    ]


class TimebaseInfo(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


def sample(pid):
    if sys.platform != "darwin":
        raise RuntimeError("proc_pid_rusage profiling is macOS-only")
    library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    function = library.proc_pid_rusage
    function.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    function.restype = ctypes.c_int
    info = RusageInfoV0()
    if function(pid, 0, ctypes.byref(info)) != 0:
        raise OSError(ctypes.get_errno(), f"proc_pid_rusage failed for PID {pid}")
    clock = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
    timebase = TimebaseInfo()
    convert = clock.mach_timebase_info
    convert.argtypes = [ctypes.POINTER(TimebaseInfo)]
    convert.restype = ctypes.c_int
    if convert(ctypes.byref(timebase)) != 0 or not timebase.denom:
        raise RuntimeError("mach_timebase_info failed")
    return {
        "pid": pid,
        "uuid": bytes(info.uuid).hex(),
        **{name: getattr(info, name) for name, _ in info._fields_ if name != "uuid"},
        "timebaseNumer": timebase.numer,
        "timebaseDenom": timebase.denom,
        "userNs": info.userAbstime * timebase.numer // timebase.denom,
        "systemNs": info.systemAbstime * timebase.numer // timebase.denom,
    }


def interval(before, after, seconds):
    """Compare matching process incarnations; never zero-fill missing PIDs."""
    deltas = []
    before = {row["pid"]: row for row in before}
    after = {row["pid"]: row for row in after}
    incomplete = set(before) ^ set(after)
    for pid in before.keys() & after.keys():
        old, new = before[pid], after[pid]
        if (old["uuid"], old["startAbstime"]) != (new["uuid"], new["startAbstime"]):
            incomplete.add(pid)
            continue
        values = {name: new[name] - old[name] for name in
                  ("userNs", "systemNs", "packageIdleWakeups", "interruptWakeups")}
        if any(value < 0 for value in values.values()):
            incomplete.add(pid)
            continue
        deltas.append({"pid": pid, **values})
    cpu = sum(row["userNs"] + row["systemNs"] for row in deltas)
    return {
        "seconds": seconds,
        "matchedProcessDeltas": sorted(deltas, key=lambda row: row["pid"]),
        "incompletePids": sorted(incomplete),
        "cpuSeconds": cpu / 1e9,
        "cpuPercentOfOneCore": cpu / 1e9 / seconds * 100,
        "packageIdleWakeupsPerSecond": sum(row["packageIdleWakeups"] for row in deltas) / seconds,
        "interruptWakeupsPerSecond": sum(row["interruptWakeups"] for row in deltas) / seconds,
    }


if __name__ == "__main__":
    import json
    import os
    import resource
    import time

    before = sample(os.getpid())
    posix_before = resource.getrusage(resource.RUSAGE_SELF)
    start = time.monotonic()
    while time.monotonic() - start < 0.2:
        pass
    elapsed = time.monotonic() - start
    after = sample(os.getpid())
    posix_after = resource.getrusage(resource.RUSAGE_SELF)
    delta = interval([before], [after], elapsed)
    posix_seconds = ((posix_after.ru_utime + posix_after.ru_stime)
                     - (posix_before.ru_utime + posix_before.ru_stime))
    if abs(delta["cpuSeconds"] - posix_seconds) > 0.02:
        raise RuntimeError("CPU counters disagree with POSIX getrusage")
    replaced = {**after, "startAbstime": before["startAbstime"] + 1}
    assert interval([before], [replaced], elapsed)["incompletePids"] == [os.getpid()]
    print(json.dumps({"scope": "instrumentation check; not app performance",
                      "timebaseNumer": after["timebaseNumer"],
                      "timebaseDenom": after["timebaseDenom"],
                      "kernelCpuSeconds": delta["cpuSeconds"],
                      "posixCpuSeconds": posix_seconds,
                      "pidReuseRejected": True}, indent=2))
