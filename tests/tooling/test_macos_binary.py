import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("macos_binary", Path(__file__).resolve().parents[2] / "scripts/macos_binary.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class UniversalMacTests(unittest.TestCase):
    def test_validates_both_platforms(self):
        with patch.object(module.subprocess, "check_output", side_effect=["arm64 x86_64\n", " platform MACOS\n", " platform MACOS\n"]) as command:
            self.assertEqual(module.verify("app"), ["arm64", "x86_64"])
            self.assertEqual(command.call_count, 3)

    def test_rejects_thin_and_simulator_binaries(self):
        for output in [["arm64\n"], ["arm64 x86_64\n", " platform IOSSIMULATOR\n"], ["arm64 x86_64\n", " platform MACOS\n", " platform MACCATALYST\n"], ["arm64 x86_64\n", "missing platform"]]:
            with self.subTest(output=output), patch.object(module.subprocess, "check_output", side_effect=output):
                with self.assertRaises(ValueError):
                    module.verify("app")
