"""Build the supported ad-hoc macOS packages without Apple account credentials."""
import os
import subprocess
import sys

if sys.platform != "darwin":
    raise SystemExit("macOS app/DMG packaging requires a macOS host.")

environment = {
    key: value for key, value in os.environ.items()
    if not key.startswith("APPLE_")
    and key not in {"TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"}
}
raise SystemExit(subprocess.run([
    "pnpm", "tauri", "build", "--bundles", "app,dmg", "--config",
    '{"bundle":{"macOS":{"signingIdentity":"-"}}}',
], env=environment).returncode)
