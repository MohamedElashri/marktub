"""Optional development-only macOS helper attribution. Never loaded by the app.

The private responsibility SPI is feature-detected. It is exclusion evidence,
not sole ownership proof: same-responsibility candidates still need the existing
prelaunch delta and post-exit lifecycle checks. Unknown/changed proofs refuse
admission. Historical samples without contemporaneous proofs stay unchanged.
"""
import ctypes
import errno
from macos_rusage import sample


def incarnation(info):
    return {key: info[key] for key in ("uuid", "startAbstime")}


def proof(pid):
    library = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    try:
        responsible = library.responsibility_get_pid_responsible_for_pid
    except AttributeError as error:
        raise OSError(errno.ENOSYS, "responsibility SPI unavailable") from error
    responsible.argtypes = [ctypes.c_int]
    responsible.restype = ctypes.c_int
    before = incarnation(sample(pid))
    owner = responsible(pid)
    if owner <= 0:
        raise OSError(errno.EAGAIN, "process responsibility is unknown")
    owner_before = incarnation(sample(owner))
    if (responsible(pid) != owner or incarnation(sample(pid)) != before
            or incarnation(sample(owner)) != owner_before):
        raise OSError(errno.EAGAIN, "process incarnation or responsibility changed")
    return {"pid": pid, "incarnation": before, "responsiblePid": owner,
            "responsibleIncarnation": owner_before}


def owner_key(value):
    return value["responsiblePid"], value["responsibleIncarnation"]


def select(root, candidates, descendants, read=proof, preexisting=frozenset()):
    record = {"method": "private macOS responsibility SPI + birth/lifecycle; development only",
              "root": None, "proofs": {}, "excludedForeign": [], "errors": [], "complete": False}
    helpers = set()
    try:
        record["root"] = read(root)
    except OSError as error:
        record["errors"].append({"pid": root, "error": str(error)})
    for pid in sorted(candidates):
        try:
            value = read(pid)
            record["proofs"][str(pid)] = value
            if record["root"] is None:
                helpers.add(pid)
            elif owner_key(value) != owner_key(record["root"]):
                if pid in descendants:
                    helpers.add(pid)
                    record["errors"].append({"pid": pid, "error": "foreign responsibility conflicts with app ancestry"})
                else:
                    record["excludedForeign"].append(pid)
            else:
                helpers.add(pid)
                if pid in preexisting:
                    record["errors"].append({"pid": pid, "error": "same-responsibility helper predates the app baseline"})
        except OSError as error:
            helpers.add(pid)
            record["errors"].append({"pid": pid, "error": str(error)})
    record["complete"] = not record["errors"]
    return helpers, record


def verify(record, current_candidates, read=proof):
    """Bracket measurement; changed/vanished/new helpers cannot be ignored."""
    if set(map(int, record["proofs"])) != set(current_candidates):
        record["errors"].append({"error": "candidate helper set changed during capture"})
    values = ([record["root"]] if record["root"] else []) + list(record["proofs"].values())
    for before in values:
        try:
            if read(before["pid"]) != before:
                record["errors"].append({"pid": before["pid"], "error": "ownership proof changed during capture"})
        except OSError as error:
            record["errors"].append({"pid": before["pid"], "error": str(error)})
    record["complete"] = not record["errors"]
    return record
