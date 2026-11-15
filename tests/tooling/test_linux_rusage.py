import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("linux_rusage", Path(__file__).resolve().parents[2] / "benchmarks/linux_rusage.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LinuxCounters(unittest.TestCase):
    def test_stat_allows_spaces_and_parentheses_in_comm(self):
        fields = ["S"] + [str(i) for i in range(4, 53)]
        result = module.stat("123 (helper (nested) name) " + " ".join(fields))
        self.assertEqual(result, {"pid": 123, "parent": 4, "userTicks": 14, "systemTicks": 15, "startTicks": 22})

    def test_intervals_refuse_reused_missing_and_regressing_processes(self):
        old = {"pid": 1, "startTicks": 20, "clockTicksPerSecond": 100,
               "userTicks": 10, "systemTicks": 5, "voluntary_ctxt_switches": 7,
               "nonvoluntary_ctxt_switches": 3}
        new = {**old, "userTicks": 30, "voluntary_ctxt_switches": 9}
        result = module.interval([old], [new], 2)
        self.assertEqual(result["cpuPercentOfOneCore"], 10)
        self.assertEqual(result["voluntaryContextSwitchesPerSecond"], 1)
        self.assertEqual(result["incompletePids"], [])
        for changed in [{**new, "startTicks": 21}, {**new, "userTicks": 0}, {**new, "clockTicksPerSecond": 200}]:
            self.assertEqual(module.interval([old], [changed], 2)["incompletePids"], [1])
        self.assertEqual(module.interval([old], [], 2)["incompletePids"], [1])
        with self.assertRaises(ValueError):
            module.interval([old], [new], 0)
