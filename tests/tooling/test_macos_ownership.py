import errno
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "benchmarks"))
from macos_ownership import select, verify


def entry(pid, owner=10, start=1, owner_start=1):
    return {"pid": pid, "incarnation": {"uuid": "test", "startAbstime": start},
            "responsiblePid": owner, "responsibleIncarnation": {"uuid": "owner", "startAbstime": owner_start}}


class OwnershipTests(unittest.TestCase):
    def test_excludes_only_positive_foreign_and_keeps_shared_owner_candidates(self):
        values = {1: entry(1), 2: entry(2), 3: entry(3, owner=30)}
        helpers, record = select(1, {2, 3}, {1}, values.__getitem__)
        self.assertEqual(helpers, {2})
        self.assertEqual(record["excludedForeign"], [3])
        self.assertTrue(verify(record, {2, 3}, values.__getitem__)["complete"])

    def test_unknown_query_is_not_foreign_exclusion(self):
        def read(pid):
            if pid == 3:
                raise OSError(errno.EACCES, "denied")
            return entry(pid)
        helpers, record = select(1, {2, 3}, {1}, read)
        self.assertEqual(helpers, {2, 3})
        self.assertFalse(record["complete"])

    def test_root_failure_cannot_admit_any_helpers(self):
        def read(pid):
            if pid == 1:
                raise OSError(errno.ENOSYS, "unavailable")
            return entry(pid)
        helpers, record = select(1, {2}, {1}, read)
        self.assertEqual(helpers, {2})
        self.assertFalse(record["complete"])

    def test_owner_or_pid_reuse_and_new_helpers_refuse_admission(self):
        for changed in [entry(2, owner=30), entry(2, start=2), entry(2, owner_start=2)]:
            values = {1: entry(1), 2: entry(2)}
            _, record = select(1, {2}, {1}, values.__getitem__)
            values[2] = changed
            self.assertFalse(verify(record, {2}, values.__getitem__)["complete"])
        values = {1: entry(1), 2: entry(2)}
        _, record = select(1, {2}, {1}, values.__getitem__)
        self.assertFalse(verify(record, {2, 3}, values.__getitem__)["complete"])

    def test_foreign_ancestry_conflict_stays_unknown(self):
        values = {1: entry(1), 2: entry(2, owner=30)}
        helpers, record = select(1, {2}, {1, 2}, values.__getitem__)
        self.assertEqual(helpers, {2})
        self.assertFalse(record["complete"])

    def test_preexisting_shared_owner_is_not_assumed_to_belong_to_new_app(self):
        values = {1: entry(1), 2: entry(2)}
        helpers, record = select(1, {2}, {1}, values.__getitem__, preexisting={2})
        self.assertEqual(helpers, {2})
        self.assertFalse(record["complete"])


if __name__ == "__main__":
    unittest.main()
