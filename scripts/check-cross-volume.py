"""Run a native cross-volume fixture on a disposable local APFS disk image.
Never selects an existing disk, user's volume, or project repository for mutations.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

artifacts = Path("artifacts")
artifacts.mkdir(exist_ok=True)
image = artifacts / f"cross-volume-{uuid.uuid4()}.sparseimage"
result = {"scope": "macOS local APFS disk image, not physical/removable/network filesystem certification", "image": str(image)}
with tempfile.TemporaryDirectory(prefix="marktub-cross-volume-", dir="/private/tmp") as mount:
    attached = False
    try:
        subprocess.run(["hdiutil", "create", "-size", "128m", "-type", "SPARSE", "-fs", "APFS", "-volname", "MarktubTest", str(image)], check=True)
        subprocess.run(["hdiutil", "attach", "-nobrowse", "-mountpoint", mount, str(image)], check=True)
        attached = True
        result["devices"] = {"source": Path.cwd().stat().st_dev, "destination": Path(mount).stat().st_dev}
        assert result["devices"]["source"] != result["devices"]["destination"]
        env = dict(os.environ, MARKTUB_CROSS_VOLUME_ROOT=mount)
        run = subprocess.run(["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "cross_volume_copy_and_directory_move", "--", "--ignored", "--nocapture"], env=env)
        result["nativeExitCode"] = run.returncode
    finally:
        if attached:
            subprocess.run(["hdiutil", "detach", mount], check=True)
            result["detached"] = True
        (artifacts / "cross-volume.json").write_text(json.dumps(result, indent=2)+"\n")
assert result.get("nativeExitCode") == 0, result
