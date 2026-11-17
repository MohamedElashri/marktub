"""Sign checked desktop packages, or combine their manifests for a release."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import sys

PLATFORMS = {"darwin-universal": "universal.app.tar.gz", "linux-x86_64-deb": "amd64.deb"}
PUBLIC_KEY = Path("src-tauri/update-public-key.txt")
ORIGIN = "https://github.com/MohamedElashri/marktub/releases/download"


def version():
    return json.loads(Path("package.json").read_text())["version"]


def validate(manifest):
    v = manifest["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", v):
        raise ValueError("Expected a stable release version")
    if set(manifest) != {"version", "notes", "platforms"} or len(manifest["notes"].encode()) > 4096:
        raise ValueError("Invalid manifest fields or release notes")
    for platform, package in manifest["platforms"].items():
        expected = f"{ORIGIN}/v{v}/Marktub_{v}_{PLATFORMS[platform]}"
        if set(package) != {"url", "signature"} or package["url"] != expected:
            raise ValueError("Unexpected package address")
        signature = package["signature"]
        if not isinstance(signature, str) or not 0 < len(signature) <= 4096:
            raise ValueError("Invalid package signature")
        # Fail the release preparation if CI used a different signing identity.
        public = base64.b64decode(PUBLIC_KEY.read_text().strip(), validate=True).decode().splitlines()
        signed = base64.b64decode(signature, validate=True).decode().splitlines()
        if base64.b64decode(public[1])[2:10] != base64.b64decode(signed[1])[2:10]:
            raise ValueError("Signature does not match the pinned public key")
    return manifest


def sign(platform, package, output):
    v = version()
    name = f"Marktub_{v}_{PLATFORMS[platform]}"
    if package.name != name or not package.is_file() or package.is_symlink():
        raise ValueError(f"Expected the checked package named {name}")
    if not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
        raise ValueError("TAURI_SIGNING_PRIVATE_KEY must be configured for signing")
    result = subprocess.run([
        "node", "node_modules/@tauri-apps/cli/tauri.js", "signer", "sign",
        "--app-version", v, str(package),
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    if result.returncode:
        raise ValueError("Package signing failed; no manifest was produced")
    manifest = validate({"version": v, "notes": "", "platforms": {
        platform: {"url": f"{ORIGIN}/v{v}/{name}", "signature": Path(str(package) + ".sig").read_text().strip()}
    }})
    output.write_text(json.dumps(manifest, indent=2) + "\n")


def combine(inputs, notes, output):
    merged = {"version": version(), "notes": notes.read_text().strip(), "platforms": {}}
    for source in inputs:
        manifest = validate(json.loads(source.read_text()))
        if manifest["version"] != merged["version"] or set(merged["platforms"]) & set(manifest["platforms"]):
            raise ValueError("Mixed versions or duplicate update platforms")
        merged["platforms"].update(manifest["platforms"])
    if set(merged["platforms"]) != set(PLATFORMS):
        raise ValueError("Both checked desktop platforms are required")
    output.write_text(json.dumps(validate(merged), indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    signer = commands.add_parser("sign")
    signer.add_argument("--platform", choices=PLATFORMS, required=True)
    signer.add_argument("--package", type=Path, required=True)
    signer.add_argument("--output", type=Path, required=True)
    merger = commands.add_parser("combine")
    merger.add_argument("inputs", nargs="+", type=Path)
    merger.add_argument("--notes", type=Path, required=True)
    merger.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "sign":
            sign(args.platform, args.package, args.output)
        else:
            combine(args.inputs, args.notes, args.output)
    except (ValueError, KeyError, OSError):
        print("Update manifest preparation failed. Check package names, versions, signatures and signing configuration.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
