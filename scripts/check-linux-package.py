"""Inspect a locally built deb without installing it or changing MIME defaults.

This verifies package contents, not clean-machine launch or physical desktop input.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_executable(packaged, unbundled):
    # The Tauri bundler changes this one fixed-width placeholder in its copy.
    # Accept no other byte changes, and never accept an unmarked deb executable.
    unknown = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
    deb = b"__TAURI_BUNDLE_TYPE_VAR_DEB"
    assert packaged.count(deb) == 1, "Missing or ambiguous Debian bundle marker"
    if packaged == unbundled:
        return "identical"
    assert unbundled.count(unknown) == 1, "Missing or ambiguous original bundle marker"
    assert packaged == unbundled.replace(unknown, deb, 1), "Unexpected executable changes beyond the fixed Tauri bundle marker"
    return "only the fixed Tauri UNK-to-DEB marker; every other byte identical"


def inspect(package, binary):
    fields = {field: run("dpkg-deb", "--field", str(package), field)
              for field in ("Package", "Version", "Architecture", "Depends")}
    assert fields["Package"] == "marktub", fields
    assert fields["Version"] == json.loads(Path("package.json").read_text())["version"], "Debian package version differs from the application manifest"
    assert fields["Architecture"] == "amd64", fields
    dependencies = [entry.strip().split()[0] for entry in fields["Depends"].split(",")]
    assert len(dependencies) == len(set(dependencies)), "Duplicate Debian runtime dependencies"
    for dependency in ("libwebkit2gtk-4.1-0", "libgtk-3-"):
        assert dependency in fields["Depends"], fields
    with tempfile.TemporaryDirectory(prefix="marktub-deb-") as directory:
        root = Path(directory)
        subprocess.run(["dpkg-deb", "--extract", str(package), directory], check=True)
        executable = root / "usr/bin/marktub"
        assert executable.is_file() and executable.stat().st_mode & 0o111
        bundled = executable.read_bytes()
        marker_patch = verify_executable(bundled, binary.read_bytes())
        for marker in (b"e2e_fixture", b"allow-e2e-fixture", b"WDIO Plugin", b"wdioTauri", b"plugin:wdio|", b"MARKTUB_TEST_"):
            assert marker not in bundled, "Test hook in packaged executable"
        header = run("readelf", "--file-header", str(executable))
        assert "Advanced Micro Devices X86-64" in header, header
        loader = run("readelf", "--program-headers", str(executable))
        dynamic = run("readelf", "--dynamic", str(executable))
        for text in (loader, dynamic):
            assert not any(path in text for path in ("/home/", "/tmp/", "linuxbrew")), "Nonportable native loader/library path"
        linked = run("ldd", str(executable))
        assert "not found" not in linked, linked
        assert "linuxbrew" not in linked and "/home/" not in linked, linked
        desktops = list((root / "usr/share/applications").glob("*.desktop"))
        assert len(desktops) == 1, desktops
        desktop = desktops[0]
        subprocess.run(["desktop-file-validate", str(desktop)], check=True)
        text = desktop.read_text()
        assert "Exec=marktub %F" in text, text
        assert "MimeType=text/markdown;" in text, text
        assert "Terminal=false" in text, text
        resources = {}
        for name, source in [("specimen.md", Path("tests/fixtures/specimen.md")), ("licenses.txt", Path("artifacts/licenses/licenses.txt"))]:
            candidates = list((root / "usr/lib").rglob(name))
            assert len(candidates) == 1, (name, candidates)
            assert candidates[0].is_file() and not candidates[0].is_symlink()
            assert candidates[0].read_bytes() == source.read_bytes(), f"Bundled {name} differs"
            resources[name] = digest(source)
        return {"scope": "locally built deb extraction, executable/resources/desktop/dependencies; not install or GUI certification",
                "package": fields, "debSha256": digest(package), "debBytes": package.stat().st_size,
                "binarySha256": digest(executable), "binaryBytes": executable.stat().st_size,
                "unbundledBinarySha256": digest(binary), "bundleMarkerPatch": marker_patch,
                "desktopFile": desktop.name, "resourcesSha256": resources,
                "nativeLibraries": [line.strip() for line in linked.splitlines()],
                "portableLoaderAndLibraries": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deb", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("src-tauri/target/release/marktub"))
    parser.add_argument("--output", type=Path, default=Path("artifacts/linux-package.json"))
    args = parser.parse_args()
    result = inspect(args.deb.resolve(), args.binary.resolve())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "nativeLibraries"}, indent=2))
