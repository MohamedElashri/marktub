"""Exercise release downloads without network, elevation or desktop actions."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "scripts/install.sh"
RELEASE = "https://github.com/MohamedElashri/marktub/releases/download/v88.7.6"
API = "https://api.github.com/repos/MohamedElashri/marktub/releases/latest"
PACKAGE = b"verified release fixture\n"
MOCK = r'''#!PYTHON
import hashlib,json,os,pathlib,sys
config=json.loads(pathlib.Path(os.environ['MARKTUB_INSTALL_TEST_CONFIG']).read_text())
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
record={'command':name,'args':args}
if name=='uname':
    print(config['platform'] if args==['-s'] else config['architecture'])
elif name in ['curl','wget']:
    url=args[-1]
    destination=args[args.index('--output')+1] if name=='curl' else next(a.split('=',1)[1] for a in args if a.startswith('--output-document='))
    record.update(url=url,destination=destination)
    with open(os.environ['MARKTUB_INSTALL_TEST_LOG'],'a') as log: log.write(json.dumps(record)+'\n')
    body=config['responses'].get(url)
    if body is None: sys.exit(22)
    pathlib.Path(destination).write_text(body)
    if url==config.get('failedUrl'): sys.exit(22)
elif name=='sudo':
    os.execv(str(pathlib.Path(sys.argv[0]).parent/args[0]),args)
elif name in ['apt-get','dnf','zypper','open']:
    record['sha256']=hashlib.sha256(pathlib.Path(args[-1]).read_bytes()).hexdigest()
    with open(os.environ['MARKTUB_INSTALL_TEST_LOG'],'a') as log: log.write(json.dumps(record)+'\n')
else:
    raise AssertionError(name)
'''


class InstallerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="marktub-installer-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ["grep", "cut", "awk", "mktemp", "rm", "mkdir", "cp", "chmod"]:
            (self.bin / name).symlink_to(shutil.which(name))
        digest = shutil.which("sha256sum") or shutil.which("shasum")
        (self.bin / Path(digest).name).symlink_to(digest)
        for name in ["uname", "curl", "wget", "sudo", "apt-get", "dnf", "zypper", "open"]:
            path = self.bin / name
            path.write_text(MOCK.replace("#!PYTHON", "#!" + sys.executable))
            path.chmod(0o755)
        self.config = {"platform": "Linux", "architecture": "x86_64", "responses": {}}
        self.checksum = hashlib.sha256(PACKAGE).hexdigest()
        self.output = self.root / "saved downloads"
        self.config_path = self.root / "config.json"
        self.log = self.root / "commands.jsonl"
        self.environment = {**os.environ, "PATH": str(self.bin),
                            "MARKTUB_INSTALL_TEST_CONFIG": str(self.config_path),
                            "MARKTUB_INSTALL_TEST_LOG": str(self.log)}
        self.bash = "/bin/bash"
        self.release()

    def release(self, suffix="amd64.deb", compact=False):
        self.filename = "Marktub_88.7.6_" + suffix
        asset = {"browser_download_url": RELEASE + "/" + self.filename}
        self.config['responses'] = {
            API: json.dumps({"assets": [asset]}, indent=None if compact else 2),
            asset['browser_download_url']: PACKAGE.decode(),
            RELEASE + '/SHA256SUMS': self.checksum + '  ' + self.filename + '\n',
        }

    def run_script(self, *args):
        self.config_path.write_text(json.dumps(self.config))
        result = subprocess.run([self.bash, str(SCRIPT), *args], cwd=self.root,
                                env=self.environment, capture_output=True, text=True, timeout=15)
        self.commands = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        for call in self.commands:
            if 'destination' in call:
                self.assertFalse(Path(call['destination']).parent.exists(), 'temporary downloads must be cleaned')
        return result

    def assert_no_install(self):
        self.assertFalse(any(c['command'] in ['apt-get', 'dnf', 'zypper', 'open'] for c in self.commands))

    def test_latest_compact_metadata_pins_checksum_to_selected_release(self):
        self.release(compact=True)
        result = self.run_script('--download-only', '--directory', str(self.output))
        self.assertEqual(result.returncode, 0, result.stderr)
        saved, = self.output.glob('Marktub.*/*')
        self.assertEqual(saved.read_bytes(), PACKAGE)
        self.assertEqual([c['url'] for c in self.commands], [API, RELEASE+'/'+self.filename, RELEASE+'/SHA256SUMS'])
        self.assert_no_install()

    def test_wget_fallback_downloads_and_verifies(self):
        (self.bin / 'curl').unlink()
        result = self.run_script('--download-only')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(c['command']=='wget' for c in self.commands))
        self.assert_no_install()

    def test_corruption_never_reaches_package_manager(self):
        self.config['responses'][RELEASE+'/'+self.filename] = 'corrupted bytes'
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Checksum mismatch', result.stderr)
        self.assert_no_install()

    def test_missing_or_duplicate_checksums_never_install(self):
        original = self.config['responses'][RELEASE+'/SHA256SUMS']
        for checksums in ['', original+original]:
            with self.subTest(checksums=checksums):
                self.config['responses'][RELEASE+'/SHA256SUMS'] = checksums
                result = self.run_script()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Missing or ambiguous', result.stderr)
                self.assert_no_install()

    def test_partial_download_never_installs(self):
        self.config['failedUrl'] = RELEASE+'/'+self.filename
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_install()

    def test_unsupported_linux_architecture_does_not_download(self):
        self.config['architecture'] = 'aarch64'
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('No released package', result.stderr)
        self.assertEqual(self.commands, [])

    def test_ambiguous_packages_are_rejected(self):
        self.config['responses'][API] = json.dumps({'assets': [
            {'browser_download_url': RELEASE+'/'+self.filename},
            {'browser_download_url': RELEASE+'/Marktub_other_amd64.deb'}]})
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('no unique package', result.stderr)
        self.assert_no_install()

    def test_linux_installs_only_verified_package(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        install, = [c for c in self.commands if c['command']=='apt-get']
        self.assertEqual(install['args'][:2], ['install', '-y'])
        self.assertEqual(install['sha256'], self.checksum)

    def test_rpm_detection_uses_dnf_or_zypper_and_only_verified_bytes(self):
        (self.bin / 'apt-get').unlink()
        self.release('x86_64.rpm')
        for manager in ['dnf', 'zypper']:
            with self.subTest(manager=manager):
                self.log.unlink(missing_ok=True)
                if manager == 'zypper':
                    (self.bin / 'dnf').unlink()
                result = self.run_script()
                self.assertEqual(result.returncode, 0, result.stderr)
                install, = [c for c in self.commands if c['command'] == manager]
                self.assertEqual(install['args'][0], 'install')
                self.assertEqual(install['sha256'], self.checksum)
                self.assertTrue(install['args'][-1].endswith('_x86_64.rpm'))

    def test_explicit_rpm_download_ignores_deb_and_needs_no_package_manager(self):
        self.release('x86_64.rpm')
        assets = json.loads(self.config['responses'][API])
        assets['assets'].append({'browser_download_url': RELEASE+'/Marktub_88.7.6_amd64.deb'})
        self.config['responses'][API] = json.dumps(assets)
        for name in ['apt-get', 'dnf', 'zypper']:
            (self.bin / name).unlink()
        result = self.run_script('--package-format', 'rpm', '--download-only', '--directory', str(self.output))
        self.assertEqual(result.returncode, 0, result.stderr)
        saved, = self.output.glob('Marktub.*/*')
        self.assertEqual(saved.name, self.filename)
        self.assertEqual(saved.read_bytes(), PACKAGE)
        self.assert_no_install()

    def test_corrupt_rpm_never_reaches_a_package_manager(self):
        self.release('x86_64.rpm')
        self.config['responses'][RELEASE+'/'+self.filename] = 'corrupt RPM'
        result = self.run_script('--package-format', 'rpm')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Checksum mismatch', result.stderr)
        self.assert_no_install()

    def test_unsupported_format_and_missing_manager_fail_before_downloading(self):
        for args in [('--package-format',), ('--package-format', 'zip')]:
            with self.subTest(args=args):
                result = self.run_script(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.commands, [])
        for name in ['apt-get', 'dnf', 'zypper']:
            (self.bin / name).unlink()
        result = self.run_script('--package-format', 'rpm')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.commands, [])

    def test_macos_both_architectures_open_persistent_verified_universal_image(self):
        self.config['platform'] = 'Darwin'
        self.release('universal.dmg')
        for architecture in ['arm64', 'x86_64']:
            with self.subTest(architecture=architecture):
                self.log.unlink(missing_ok=True)
                self.config['architecture'] = architecture
                result = self.run_script('--directory', str(self.output))
                self.assertEqual(result.returncode, 0, result.stderr)
                opened, = [c for c in self.commands if c['command']=='open']
                self.assertEqual(opened['sha256'], self.checksum)
                self.assertEqual(Path(opened['args'][0]).read_bytes(), PACKAGE)
                self.assertFalse(any(c['command']=='apt-get' for c in self.commands))
