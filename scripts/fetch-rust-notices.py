"""Explicit network collection of missing crate notices from pinned upstream commits.

Never run by a build. Review the resulting supplement and coverage before release.
Only GitHub repositories and archive-recorded full commit hashes are admitted.
"""
import concurrent.futures
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import urllib.request

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--manifest-path", "src-tauri/Cargo.toml", "--locked",
    "--offline", "--format-version", "1"], text=True))
missing = set(json.loads(Path("artifacts/licenses/rust-license-coverage.json").read_text())["missing"])
packages = [p for p in metadata["packages"] if f'{p["name"]}@{p["version"]}' in missing]

def collect(p):
    directory = Path(p["manifest_path"]).parent
    vcs = json.loads((directory / ".cargo_vcs_info.json").read_text())
    commit = vcs["git"]["sha1"]
    repository = p.get("repository") or ""
    match = re.fullmatch(r"https://github.com/([\w.-]+/[\w.-]+?)(?:\.git)?/?", repository)
    if not match or not re.fullmatch(r"[a-f0-9]{40}", commit):
        return {"package": f'{p["name"]}@{p["version"]}', "error": "No pinned GitHub origin"}
    # Licenses are commonly excluded from subcrate archives but retained at repo root.
    bases = ["", vcs.get("path_in_vcs", ""), "licenses", "LICENSES"]
    names = ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-APACHE-2.0", "LICENSE_MIT", "LICENSE_APACHE", "LICENSE.md", "LICENSE.txt", "COPYING", "MIT.txt", "Apache-2.0.txt", "Zlib.txt", "COPYING.MIT", "COPYING.Apache-2.0", "COPYING.LGPL-2.1", "LICENSE-MPL-2.0", "LICENSE-MPL-2.0.txt", "LICENSE-APACHE-2.0.txt", "LICENSE.MIT", "LICENSE.APACHE2", "LICENSE.LGPL2_1"]
    urls = list(dict.fromkeys(f'https://raw.githubusercontent.com/{match[1]}/{commit}/{str(PurePosixPath(base) / name)}' for base in bases for name in names))
    texts = []
    def fetch(url):
        try:
            with urllib.request.urlopen(url, timeout=8) as r:
                raw = r.read(262145)
                if len(raw) > 262144: return None
                text = raw.decode("utf-8")
                if len(text.strip()) < 100: return None
                return {"url": url, "sha256": hashlib.sha256(raw).hexdigest(), "text": text}
        except Exception:
            return None
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        for item in pool.map(fetch, urls):
            if item and not any(t["sha256"] == item["sha256"] for t in texts): texts.append(item)
    return {"package": f'{p["name"]}@{p["version"]}', "repository": repository, "commit": commit, "notices": texts}

output = Path("src/rust-notice-supplements.json")
retained = json.loads(output.read_text()) if output.exists() else {"texts": {}, "packages": []}
records = retained["packages"]
for package in packages:
    try: result = collect(package)
    except Exception as e: result = {"package": f'{package["name"]}@{package["version"]}', "error": type(e).__name__}
    for notice in result.get("notices", []):
        retained["texts"][notice["sha256"]] = notice.pop("text")
    records = [r for r in records if r["package"] != result["package"]] + [result]
    output.write_text(json.dumps({"texts": retained["texts"], "packages": sorted(records, key=lambda r:r["package"])}, ensure_ascii=False, indent=2) + "\n")
    print(result["package"], len(result.get("notices", [])), flush=True)
