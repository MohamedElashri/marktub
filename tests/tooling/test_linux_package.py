import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("linux_package", Path(__file__).resolve().parents[2] / "scripts/check-linux-package.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PackageExecutable(unittest.TestCase):
    def test_only_the_unique_fixed_bundle_marker_may_change(self):
        normal = b"executable before\0__TAURI_BUNDLE_TYPE_VAR_UNK\0after"
        package = b"executable before\0__TAURI_BUNDLE_TYPE_VAR_DEB\0after"
        module.verify_executable(package, normal)
        self.assertEqual(module.verify_executable(package, package), "identical")
        for bad in [normal, package + b"extra", package.replace(b"before", b"BEFORE"), package + b"__TAURI_BUNDLE_TYPE_VAR_DEB"]:
            with self.assertRaises(AssertionError):
                module.verify_executable(bad, normal)
        with self.assertRaises(AssertionError):
            module.verify_executable(package, normal + b"__TAURI_BUNDLE_TYPE_VAR_UNK")
