"""Give the single Tauri RPM a stable release/update asset name."""
import argparse
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--version", required=True)
parser.add_argument("--directory", type=Path, default=Path("src-tauri/target/release/bundle/rpm"))
args = parser.parse_args()
packages = list(args.directory.glob("*.rpm"))
if len(packages) != 1 or not packages[0].is_file() or packages[0].is_symlink():
    parser.error("Expected exactly one freshly built RPM")
target = args.directory / f"Marktub_{args.version}_x86_64.rpm"
if target != packages[0]:
    packages[0].rename(target)
print(target)
