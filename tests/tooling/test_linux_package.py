import importlib.util
from pathlib import Path
import shutil
import tempfile
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


class PackageIcons(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="marktub-package-icons-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        source = Path(__file__).resolve().parents[2] / "src-tauri/icons"
        self.icons = [source / "32x32.png", source / "64x64.png"]
        self.desktop = "[Desktop Entry]\nIcon=marktub\nStartupWMClass=marktub\n"

    def install(self):
        for icon, size in zip(self.icons, [32, 64]):
            path = self.root / f"usr/share/icons/hicolor/{size}x{size}/apps/marktub.png"
            path.parent.mkdir(parents=True)
            shutil.copyfile(icon, path)

    def test_released_missing_icons_are_rejected(self):
        with self.assertRaisesRegex(AssertionError, "Missing packaged launcher icon"):
            module.verify_icons(self.root, self.desktop, self.icons)

    def test_all_declared_sizes_must_match_the_original_artwork(self):
        self.install()
        self.assertEqual(len(module.verify_icons(self.root, self.desktop, self.icons)), 2)
        (self.root / "usr/share/icons/hicolor/64x64/apps/marktub.png").write_bytes(b"wrong icon")
        with self.assertRaisesRegex(AssertionError, "differs from the app logo"):
            module.verify_icons(self.root, self.desktop, self.icons)

    def test_desktop_name_and_window_class_must_resolve_the_app(self):
        self.install()
        for entry in [self.desktop.replace("Icon=marktub", "Icon=default"),
                      self.desktop.replace("StartupWMClass=marktub", "StartupWMClass=other")]:
            with self.subTest(entry=entry), self.assertRaises(AssertionError):
                module.verify_icons(self.root, entry, self.icons)

    def test_configuration_without_icons_and_symlinked_icons_are_rejected(self):
        with self.assertRaisesRegex(AssertionError, "No PNG icons"):
            module.verify_icons(self.root, self.desktop, [])
        self.install()
        path = self.root / "usr/share/icons/hicolor/32x32/apps/marktub.png"
        path.unlink()
        path.symlink_to(self.icons[0])
        with self.assertRaisesRegex(AssertionError, "Missing packaged launcher icon"):
            module.verify_icons(self.root, self.desktop, self.icons)
