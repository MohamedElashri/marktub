"""Build the supported ad-hoc macOS packages without Apple account credentials."""
import os
import subprocess
import sys
from pathlib import Path
from macos_binary import verify

if sys.platform != "darwin":
    raise SystemExit("macOS app/DMG packaging requires a macOS host.")

environment = {
    key: value for key, value in os.environ.items()
    if not key.startswith("APPLE_")
    and key not in {"TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"}
}
environment["SDKROOT"] = subprocess.check_output(["xcrun", "--sdk", "macosx", "--show-sdk-path"], text=True).strip()
result = subprocess.run([
    "pnpm", "tauri", "build", "--target", "universal-apple-darwin", "--bundles", "app,dmg", "--config",
    '{"bundle":{"macOS":{"signingIdentity":"-"}}}',
], env=environment)
if result.returncode:
    raise SystemExit(result.returncode)
verify(Path(os.environ.get("CARGO_TARGET_DIR", "src-tauri/target")) / "universal-apple-darwin/release/bundle/macos/Marktub.app/Contents/MacOS/marktub")
