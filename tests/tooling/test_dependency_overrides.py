import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("dependency_check", Path(__file__).resolve().parents[2] / "scripts/check-dependencies.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class DependencyOverrides(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        vendor = self.root / "src-tauri/vendor/example"
        vendor.mkdir(parents=True)
        (vendor / "Cargo.toml").write_text('[package]\nname="example"\nversion="1.0.0"\n')
        (self.root / "src-tauri/Cargo.toml").write_text('[package]\nversion="0.1.0"\n[patch.crates-io]\nexample={path="vendor/example"}\n')
        (self.root / "src-tauri/Cargo.lock").write_text('version=4\n')
        (self.root / "package.json").write_text('{"version":"0.1.0"}')
        (self.root / "src-tauri/tauri.conf.json").write_text('{}')
        (self.root / "patches").mkdir()
        (self.root / "patches/example.patch").write_text("reviewed patch")
        record = {"packages": [{"name": "example", "version": "1.0.0", "path": "src-tauri/vendor/example", "patch": "patches/example.patch", "patchSha256": checker.digest(self.root / "patches/example.patch"), "files": [{"path": "Cargo.toml", "sha256": checker.digest(vendor / "Cargo.toml"), "upstreamSha256": hashlib.sha256(b"original").hexdigest()}]}]}
        (self.root / "docs/evidence").mkdir(parents=True)
        (self.root / "docs/evidence/dependency-overrides.json").write_text(json.dumps(record))
        self.metadata = {"packages": [{"manifest_path": str(path), "name": name, "source": None} for path, name in [(self.root / "src-tauri/Cargo.toml", "marktub"), (vendor / "Cargo.toml", "example")]]}

    def test_exact_reviewed_source_and_cargo_version_fallback(self):
        self.assertEqual(checker.validate(self.root, self.metadata), ("0.1.0", 1))

    def test_changed_source_is_rejected(self):
        path = self.root / "src-tauri/vendor/example/Cargo.toml"
        path.write_text(path.read_text() + "# unreviewed change\n")
        with self.assertRaisesRegex(AssertionError, "Override source changed"):
            checker.validate(self.root, self.metadata)

    def test_added_source_is_rejected(self):
        (self.root / "src-tauri/vendor/example/extra.rs").write_text("unreviewed")
        with self.assertRaisesRegex(AssertionError, "Unreviewed or missing files"):
            checker.validate(self.root, self.metadata)

    def test_manifest_drift_is_rejected(self):
        (self.root / "package.json").write_text('{"version":"0.2.0"}')
        with self.assertRaisesRegex(AssertionError, "versions differ"):
            checker.validate(self.root, self.metadata)

    def test_unmaintained_dependency_is_rejected(self):
        self.metadata["packages"].append({"name": "proc-macro-error2", "source": "registry", "manifest_path": "/unused"})
        with self.assertRaisesRegex(AssertionError, "Unmaintained implementation"):
            checker.validate(self.root, self.metadata)

    def test_unreviewed_local_package_is_rejected(self):
        self.metadata["packages"].append({"name": "extra", "source": None, "manifest_path": str(self.root / "extra/Cargo.toml")})
        with self.assertRaisesRegex(AssertionError, "unreviewed local package"):
            checker.validate(self.root, self.metadata)

    def test_inactive_unmaintained_lock_entry_is_rejected(self):
        (self.root / "src-tauri/Cargo.lock").write_text('version=4\n[[package]]\nname="chrono"\nversion="0.4.45"\n')
        with self.assertRaisesRegex(AssertionError, "Unmaintained implementation returned to the lockfile"):
            checker.validate(self.root, self.metadata)


if __name__ == "__main__":
    unittest.main()
