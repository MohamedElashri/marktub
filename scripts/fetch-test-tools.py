"""Fetch exact desktop test converters, never called by the installed app.

Archives and extracted executables are SHA-256 pinned to official release assets.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
import stat

TOOLS = {
    "Linux-x86_64": {
        "PANDOC": {
            "url": "https://github.com/jgm/pandoc/releases/download/3.14/pandoc-3.14-linux-amd64.tar.gz",
            "archive": "c5f4720cdefb3f7bdc5d07202ef579feef0d1370773c873804dd8fdaa8c4f1b8",
            "member": "pandoc-3.14/bin/pandoc",
            "binary": "c6049d8f9f0bb8131d139193683bc91985a50e8fc57e05de865f010647a9d899"
        },
        "TYPST": {
            "url": "https://github.com/typst/typst/releases/download/v0.15.1/typst-x86_64-unknown-linux-musl.tar.xz",
            "archive": "a6d077d0a95eed5a2eba715b2dae06be954f624ccbf85758a03f389ded33118c",
            "member": "typst-x86_64-unknown-linux-musl/typst",
            "binary": "29273eaa04f6d00edd0c2bec578f565fc9c65be856bfbffc894567c68ed0b237"
        }
    },
    "Darwin-arm64": {
        "PANDOC": {
            "url": "https://github.com/jgm/pandoc/releases/download/3.14/pandoc-3.14-arm64-macOS.zip",
            "archive": "58d6593884fec9d4424b641b92503c316d9c36decb2891ff8d73ae74dd3b6304",
            "member": "pandoc-3.14-arm64/bin/pandoc",
            "binary": "42ee145b4b12d449c6aa13cb3d7f8be31d6f3cc1637ef45d06fc68fa86b7fd98"
        },
        "TYPST": {
            "url": "https://github.com/typst/typst/releases/download/v0.15.1/typst-aarch64-apple-darwin.tar.xz",
            "archive": "48f62ed034aa3a7978309579ac6ca00045e2ef0da73114e8af27cfd8e74dc05a",
            "member": "typst-aarch64-apple-darwin/typst",
            "binary": "7c4a136b377f3689400afe37b4f0fe3528d50faaa55cbb1106ac8a128f86ba1a"
        }
    },
    "Darwin-x86_64": {
        "PANDOC": {
            "url": "https://github.com/jgm/pandoc/releases/download/3.14/pandoc-3.14-x86_64-macOS.zip",
            "archive": "891708579061e7b1c1a6ce0df30ebb3cb2b6d0ca045ed3c8554ab830c1e0c270",
            "member": "pandoc-3.14-x86_64/bin/pandoc",
            "binary": "1294a2b7f38bb89d5d2223bb06ce48c843014bdc39862e8feb5e68bd3bf2cd93"
        },
        "TYPST": {
            "url": "https://github.com/typst/typst/releases/download/v0.15.1/typst-x86_64-apple-darwin.tar.xz",
            "archive": "7f9fdd9584866245de9a79e0add8f9236fae6f40a8a45e2c4771ccc14db4e0fa",
            "member": "typst-x86_64-apple-darwin/typst",
            "binary": "a6fb5786e71f6f95d8323b326353770734c1e4624e7c108783ea578723916b46"
        }
    }
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def unpack(archive, tool, staged):
    if tool["url"].endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            member = bundle.getinfo(tool["member"])
            if not stat.S_ISREG(member.external_attr >> 16) or member.file_size > 256 * 1024 * 1024:
                raise RuntimeError("Invalid test executable archive member")
            with bundle.open(member) as source, staged.open("xb") as target:
                shutil.copyfileobj(source, target)
    else:
        with tarfile.open(archive) as bundle:
            member = bundle.getmember(tool["member"])
            if not member.isfile() or member.size > 256 * 1024 * 1024:
                raise RuntimeError("Invalid test executable archive member")
            with bundle.extractfile(member) as source, staged.open("xb") as target:
                shutil.copyfileobj(source, target)


def fetch(output):
    platform = os.uname().sysname + "-" + os.uname().machine
    if platform not in TOOLS:
        raise RuntimeError("No pinned test tools for " + platform)
    output.mkdir(parents=True, exist_ok=True)
    result = {}
    for role, tool in TOOLS[platform].items():
        destination = output / role.lower()
        if destination.is_symlink():
            raise RuntimeError("Test-tool destination must not be a symlink")
        if not destination.is_file() or digest(destination) != tool["binary"]:
            with tempfile.TemporaryDirectory(prefix=".converter-", dir=output) as directory:
                archive = Path(directory) / "archive"
                subprocess.run(["curl", "--fail", "--location", "--proto", "=https", "--proto-redir", "=https", "--max-time", "180", tool["url"], "--output", str(archive)], check=True)
                if digest(archive) != tool["archive"]:
                    raise RuntimeError(f"{role} official archive checksum mismatch")
                staged = Path(directory) / "executable"
                unpack(archive, tool, staged)
                if digest(staged) != tool["binary"]:
                    raise RuntimeError(f"{role} extracted executable checksum mismatch")
                staged.chmod(0o755)
                staged.replace(destination)
        result[f"MARKTUB_E2E_{role}"] = str(destination.resolve())
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("artifacts/test-tools"))
    parser.add_argument("--github-env", type=Path)
    args = parser.parse_args()
    values = fetch(args.output)
    if args.github_env:
        with args.github_env.open("a") as environment:
            for key, value in values.items():
                if "\n" in value or "\r" in value:
                    raise RuntimeError("Invalid environment path")
                environment.write(f"{key}={value}\n")
    print(json.dumps(values, indent=2))
