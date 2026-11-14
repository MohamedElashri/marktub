"""Create a deterministic source archive from the checked publication inventory."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, default=Path("artifacts/releases/marktub-source.tar.gz"))
args = parser.parse_args()
files = json.loads(subprocess.check_output(["node", "scripts/check-repository.mjs", "--json"], text=True))
args.output.parent.mkdir(parents=True, exist_ok=True)
inventory = []
with args.output.open("wb") as target:
    with gzip.GzipFile(filename="", mode="wb", fileobj=target, mtime=0) as compressed:
        with tarfile.open(mode="w", fileobj=compressed, format=tarfile.PAX_FORMAT) as archive:
            for name in files:
                path = Path(name)
                data = path.read_bytes()
                info = tarfile.TarInfo("marktub/" + name)
                info.size = len(data)
                info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                archive.addfile(info, io.BytesIO(data))
                inventory.append({"path": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
result = {"format": "deterministic source snapshot; no Git history or release certification", "archive": args.output.name, "sha256": hashlib.sha256(args.output.read_bytes()).hexdigest(), "files": inventory}
args.output.with_suffix(".manifest.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({key: value for key, value in result.items() if key != "files"}, indent=2))
