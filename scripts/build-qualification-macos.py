"""Build a normal-feature ad-hoc app with a new, isolated native data identifier.

Diagnostic variant only: its executable hash differs from the shipping identifier.
No HOME override, fixture command, evaluator, credentials or profile deletion.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path)
args = parser.parse_args()
if sys.platform != "darwin":
    parser.error("This qualification build requires macOS.")
root = Path(__file__).resolve().parent.parent
token = uuid.uuid4().hex
identifier = "app.marktub.qualification.q" + token
profile = Path.home() / "Library/Application Support" / identifier
if profile.exists():
    raise SystemExit("Refusing to reuse an existing native data profile.")
artifacts = (root / "artifacts").resolve()
output = (args.output or artifacts / "mac-local-qualification" / ("profile-app-" + token)).resolve()
if not output.is_relative_to(artifacts) or output == artifacts:
    parser.error("Output must be a new directory inside project artifacts.")
output.mkdir(parents=True, exist_ok=False)
config = output / "config.json"
config.write_text(json.dumps({"identifier": identifier, "bundle": {"macOS": {"signingIdentity": "-"}}}, indent=2) + "\n")
env = {key: value for key, value in os.environ.items()
       if not key.startswith("APPLE_")
       and key not in {"TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"}}
env.pop("VITE_MARKTUB_E2E", None)
subprocess.run(["node", "scripts/check-repository.mjs"], cwd=root, env=env, check=True)
subprocess.run(["pnpm", "tauri", "build", "--bundles", "app", "--config", str(config)],
               cwd=root, env=env, check=True)
subprocess.run(["pnpm", "check:release"], cwd=root, env=env, check=True)
source = root / "src-tauri/target/release/bundle/macos/Marktub.app"
app = output / "Marktub.app"
subprocess.run(["ditto", str(source), str(app)], check=True)
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
signature = subprocess.check_output(["codesign", "--display", "--verbose=4", str(app)], stderr=subprocess.STDOUT, text=True)
if "Signature=adhoc" not in signature or "TeamIdentifier=not set" not in signature:
    raise SystemExit("Expected an ad-hoc app without an Apple developer identity.")
info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
if info["CFBundleIdentifier"] != identifier or profile.exists():
    raise SystemExit("Qualification identifier/profile isolation check failed.")
binary = app / "Contents/MacOS/marktub"
data = binary.read_bytes()
for marker in [b"e2e_fixture", b"allow-e2e-fixture", b"WDIO Plugin", b"wdioTauri", b"MARKTUB_TEST_"]:
    if marker in data:
        raise SystemExit("Diagnostic variant contains a forbidden fixture/evaluator marker.")
result = {"scope": "normal-feature diagnostic variant, not the shipping identifier/binary",
          "identifier": identifier, "expectedNativeDataDirectory": str(profile),
          "dataProfileAbsentBeforeFirstLaunch": True, "app": str(app),
          "binarySha256": hashlib.sha256(data).hexdigest(),
          "configSha256": hashlib.sha256(config.read_bytes()).hexdigest(),
          "strictAdHocSignature": True, "noFixtureEvaluatorMarkers": True,
          "originalHomeEnvironmentRetained": True}
(output / "profile.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2), flush=True)
