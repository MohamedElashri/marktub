"""Exercise real Tauri signatures without using the production signing key."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("update_manifest", "scripts/update-manifest.py")
updates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updates)


class UpdatePackages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="marktub-update-test-")
        cls.root = Path(cls.directory.name)
        cls.key = cls.root / "test.key"
        subprocess.run(["node", "node_modules/@tauri-apps/cli/tauri.js", "signer", "generate", "--ci", "--password", "", "--write-keys", str(cls.key)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def test_signed_packages_combine_and_reject_mixed_releases(self):
        paths = []
        with patch.object(updates, "PUBLIC_KEY", Path(str(self.key) + ".pub")), patch.object(updates, "version", return_value="9.0.0"), patch.dict(os.environ, {"TAURI_SIGNING_PRIVATE_KEY": self.key.read_text(), "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": ""}):
            for platform, suffix in updates.PLATFORMS.items():
                package = self.root / f"Marktub_9.0.0_{suffix}"
                package.write_bytes(b"Test-only payload, never a release package")
                output = self.root / f"{platform}.json"
                updates.sign(platform, package, output)
                manifest = json.loads(output.read_text())
                self.assertIn(platform, manifest["platforms"])
                paths.append(output)
            notes = self.root / "notes.txt"
            notes.write_text("Manual application updates.")
            combined = self.root / "latest.json"
            rpm_combined = self.root / "latest-rpm.json"
            updates.combine(paths, notes, combined, rpm_combined)
            manifest = json.loads(combined.read_text())
            self.assertEqual(set(manifest["platforms"]), updates.LEGACY_PLATFORMS)
            self.assertEqual(set(json.loads(rpm_combined.read_text())["platforms"]), {"linux-x86_64-rpm"})
            self.assertEqual(manifest["notes"], notes.read_text())
            # 0.3.0 clients require exactly two platform keys in latest.json.
            updates.combine(paths[:2], notes, combined)
            self.assertEqual(set(json.loads(combined.read_text())["platforms"]), updates.LEGACY_PLATFORMS)
            with self.assertRaises(ValueError):
                updates.combine(paths, notes, combined)
            with self.assertRaises(ValueError):
                updates.combine(paths[:2], notes, combined, rpm_combined)
            with self.assertRaises(ValueError):
                updates.combine(paths[:1], notes, combined)
            with self.assertRaises(ValueError):
                updates.combine(paths + paths, notes, combined)
            manifest["platforms"]["linux-x86_64-deb"]["url"] = "https://evil.example/update.deb"
            with self.assertRaises(ValueError):
                updates.validate(manifest)
            with patch.object(updates, "PUBLIC_KEY", Path("src-tauri/update-public-key.txt")):
                with self.assertRaises(ValueError):
                    updates.validate(json.loads(paths[0].read_text()))

    def test_private_key_required_and_package_name_bound(self):
        with patch.object(updates, "version", return_value="9.0.0"), patch.dict(os.environ, {"TAURI_SIGNING_PRIVATE_KEY": ""}):
            with self.assertRaises(ValueError):
                updates.sign("linux-x86_64-deb", self.root / "wrong.deb", self.root / "bad.json")
