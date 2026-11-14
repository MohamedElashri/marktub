"""Verify a local DMG and copy its app to a disposable install directory.

No Applications install, quarantine bypass, default association change or public
signing claim. Launch/UI checks are separate from container/signature validation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import uuid

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--dmg", type=Path, required=True)
p.add_argument("--app", type=Path, default=Path("src-tauri/target/release/bundle/macos/Marktub.app"))
p.add_argument("--output", type=Path, default=Path("artifacts/package.json"))
a = p.parse_args()
def digest(file): return hashlib.sha256(file.read_bytes()).hexdigest()
def run(args): return subprocess.check_output(args, stderr=subprocess.STDOUT).decode()
run(["hdiutil", "verify", str(a.dmg.resolve())])
installed = Path("artifacts") / f"installed-{uuid.uuid4()}" / "Marktub.app"
installed.parent.mkdir(parents=True)
with tempfile.TemporaryDirectory(prefix="marktub-package-mount-", dir="/private/tmp") as mount:
    attached = False
    try:
        run(["hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", mount, str(a.dmg.resolve())])
        attached = True
        run(["ditto", str(Path(mount)/"Marktub.app"), str(installed)])
    finally:
        if attached: run(["hdiutil", "detach", mount])
run(["codesign", "--verify", "--deep", "--strict", str(installed)])
signature = run(["codesign", "--display", "--verbose=4", str(installed)])
assert "Signature=adhoc" in signature and "TeamIdentifier=not set" in signature, "Expected the supported ad-hoc signature, without an Apple developer identity"
binary = installed / "Contents/MacOS/marktub"
original = a.app / "Contents/MacOS/marktub"
assert digest(binary) == digest(original), "DMG executable differs from normal app"
for source, bundled in [(Path("THIRD_PARTY_LICENSES.txt"),"THIRD_PARTY_LICENSES.txt"),(Path("tests/fixtures/specimen.md"),"fixtures/specimen.md")]:
    assert digest(source) == digest(installed/"Contents/Resources"/bundled), f"Bundled resource differs: {bundled}"
info = plistlib.loads((installed/"Contents/Info.plist").read_bytes())
types = info.get("CFBundleDocumentTypes", [])
assert any(set(t.get("CFBundleTypeExtensions", [])) >= {"md","markdown"} and t.get("CFBundleTypeRole") == "Editor" for t in types), "Markdown editor association missing"
result = {"kind":"local ad-hoc macOS DMG verification, not public notarized/clean-machine/Gatekeeper certification", "dmg":str(a.dmg),"dmgSha256":digest(a.dmg),"dmgBytes":a.dmg.stat().st_size,"installedApp":str(installed.resolve()),"binarySha256":digest(binary),"binaryBytes":binary.stat().st_size,"bundleBytes":sum(f.stat().st_size for f in installed.rglob("*") if f.is_file()),"architectures":run(["lipo","-archs",str(binary)]).strip(),"minimumSystemVersion":info.get("LSMinimumSystemVersion"),"documentTypes":types,"resourcesByteIdentical":True,"strictAdHocSignature":"pass","volumeDetached":True}
a.output.parent.mkdir(parents=True,exist_ok=True)
a.output.write_text(json.dumps(result,indent=2)+"\n")
print(json.dumps(result,indent=2))
