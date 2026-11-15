import importlib.util
import io
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("test_tools", Path(__file__).resolve().parents[2] / "scripts/fetch-test-tools.py")
tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tools)


class TestToolExtraction(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.folder = Path(self.temporary.name) / "work"
        self.folder.mkdir()
        self.archive = self.folder / "archive"
        self.destination = self.folder / "executable"

    def test_zip_extracts_only_selected_regular_member(self):
        with zipfile.ZipFile(self.archive, "w") as bundle:
            member = zipfile.ZipInfo("tool/bin/selected")
            member.external_attr = (stat.S_IFREG | 0o755) << 16
            bundle.writestr(member, b"selected executable")
            bundle.writestr("../escape", b"unselected")
        tools.unpack(self.archive, {"url": "https://example.invalid/tool.zip", "member": member.filename}, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"selected executable")
        self.assertFalse((self.folder.parent / "escape").exists())

    def test_zip_refuses_a_selected_symlink(self):
        with zipfile.ZipFile(self.archive, "w") as bundle:
            member = zipfile.ZipInfo("link")
            member.external_attr = (stat.S_IFLNK | 0o777) << 16
            bundle.writestr(member, "../../outside")
        with self.assertRaisesRegex(RuntimeError, "Invalid test executable"):
            tools.unpack(self.archive, {"url": "https://example.invalid/tool.zip", "member": "link"}, self.destination)
        self.assertFalse(self.destination.exists())

    def test_tar_extracts_only_selected_regular_member(self):
        with tarfile.open(self.archive, "w") as bundle:
            member = tarfile.TarInfo("tool/selected")
            member.size = 8
            bundle.addfile(member, io.BytesIO(b"selected"))
            other = tarfile.TarInfo("../escape")
            bundle.addfile(other, io.BytesIO(b""))
        tools.unpack(self.archive, {"url": "https://example.invalid/tool.tar.gz", "member": "tool/selected"}, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"selected")
        self.assertFalse((self.folder.parent / "escape").exists())

    def test_tar_refuses_a_selected_link(self):
        with tarfile.open(self.archive, "w") as bundle:
            member = tarfile.TarInfo("link")
            member.type = tarfile.LNKTYPE
            member.linkname = "../../outside"
            bundle.addfile(member)
        with self.assertRaisesRegex(RuntimeError, "Invalid test executable"):
            tools.unpack(self.archive, {"url": "https://example.invalid/tool.tar.gz", "member": "link"}, self.destination)
        self.assertFalse(self.destination.exists())


if __name__ == "__main__":
    unittest.main()
