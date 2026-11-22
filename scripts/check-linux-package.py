"""Inspect a locally built DEB or RPM without installing it or changing MIME defaults.

This verifies package contents, not clean-machine launch or physical desktop input.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_executable(packaged, unbundled, kind="deb"):
    # The Tauri bundler changes this one fixed-width placeholder in its copy.
    # Accept no other byte changes, and never accept an unmarked deb executable.
    unknown = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
    marker = {"deb": b"__TAURI_BUNDLE_TYPE_VAR_DEB", "rpm": b"__TAURI_BUNDLE_TYPE_VAR_RPM"}[kind]
    assert packaged.count(marker) == 1, "Missing or ambiguous package bundle marker"
    if packaged == unbundled:
        return "identical"
    assert unbundled.count(unknown) == 1, "Missing or ambiguous original bundle marker"
    assert packaged == unbundled.replace(unknown, marker, 1), "Unexpected executable changes beyond the fixed Tauri bundle marker"
    return f"only the fixed Tauri UNK-to-{kind.upper()} marker; every other byte identical"


def verify_icons(root, desktop, source_icons):
    fields = dict(line.split("=", 1) for line in desktop.splitlines() if "=" in line)
    assert fields.get("Icon") == "marktub", "Launcher icon must match the installed icon name"
    assert fields.get("StartupWMClass") == "marktub", "Launcher window class differs from the executable"
    assert source_icons, "No PNG icons declared for the application bundle"
    icons = {}
    for source in source_icons:
        data = source.read_bytes()
        assert data[:16] == b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", "Invalid source PNG icon"
        width, height = struct.unpack_from(">II", data, 16)
        assert width == height and width > 0, "Launcher icon must be square"
        relative = Path(f"usr/share/icons/hicolor/{width}x{height}/apps/marktub.png")
        packaged = root / relative
        assert packaged.is_file() and not packaged.is_symlink(), f"Missing packaged launcher icon: {relative}"
        assert packaged.read_bytes() == data, f"Packaged icon differs from the app logo: {relative}"
        icons[str(relative)] = digest(packaged)
    return icons


def inspect(package, binary, kind="deb"):
    if kind == "deb":
        fields = {field: run("dpkg-deb", "--field", str(package), field)
                  for field in ("Package", "Version", "Architecture", "Depends")}
    else:
        fields = {field: run("rpm", "-qp", "--queryformat", tag, str(package))
                  for field, tag in [("Package", "%{NAME}"), ("Version", "%{VERSION}"), ("Architecture", "%{ARCH}"), ("Release", "%{RELEASE}")]}
        fields["Depends"] = run("rpm", "-qp", "--requires", str(package))
        assert fields["Release"] == "1", fields
    assert fields["Package"] == "marktub", fields
    assert fields["Version"] == json.loads(Path("package.json").read_text())["version"], "Debian package version differs from the application manifest"
    assert fields["Architecture"] == ("amd64" if kind == "deb" else "x86_64"), fields
    dependencies = fields["Depends"].split(",") if kind == "deb" else fields["Depends"].splitlines()
    dependencies = [entry.strip() for entry in dependencies]
    assert len(dependencies) == len(set(dependencies)), "Duplicate runtime dependencies"
    required = ("libwebkit2gtk-4.1-0", "libgtk-3-") if kind == "deb" else ("libwebkit2gtk-4.1.so.0", "libgtk-3.so.0")
    for dependency in required:
        assert dependency in fields["Depends"], fields
    with tempfile.TemporaryDirectory(prefix=f"marktub-{kind}-") as directory:
        root = Path(directory)
        if kind == "deb":
            subprocess.run(["dpkg-deb", "--extract", str(package), directory], check=True)
        else:
            subprocess.run(["bsdtar", "--extract", "--file", str(package), "--no-same-owner"], cwd=root, check=True)
        executable = root / "usr/bin/marktub"
        assert executable.is_file() and executable.stat().st_mode & 0o111
        bundled = executable.read_bytes()
        marker_patch = verify_executable(bundled, binary.read_bytes(), kind)
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
        config = json.loads(Path("src-tauri/tauri.conf.json").read_text())
        icons = verify_icons(root, text, [Path("src-tauri") / name for name in config["bundle"].get("icon", []) if name.endswith(".png")])
        resources = {}
        for name, source in [("Marktub Guide.md", Path("tests/fixtures/specimen.md")), ("marktub-example.png", Path("src-tauri/icons/256x256.png")), ("licenses.txt", Path("artifacts/licenses/licenses.txt"))]:
            candidates = [path for library in (root / "usr").glob("lib*") for path in library.rglob(name)]
            assert len(candidates) == 1, (name, candidates)
            assert candidates[0].is_file() and not candidates[0].is_symlink()
            assert candidates[0].read_bytes() == source.read_bytes(), f"Bundled {name} differs"
            resources[name] = digest(source)
        return {"scope": f"locally built {kind} extraction, executable/resources/desktop/dependencies; not install or GUI certification",
                "package": fields, f"{kind}Sha256": digest(package), f"{kind}Bytes": package.stat().st_size,
                "binarySha256": digest(executable), "binaryBytes": executable.stat().st_size,
                "unbundledBinarySha256": digest(binary), "bundleMarkerPatch": marker_patch,
                "desktopFile": desktop.name, "iconsSha256": icons, "resourcesSha256": resources,
                "nativeLibraries": [line.strip() for line in linked.splitlines()],
                "portableLoaderAndLibraries": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    packages = parser.add_mutually_exclusive_group(required=True)
    packages.add_argument("--deb", type=Path)
    packages.add_argument("--rpm", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("src-tauri/target/release/marktub"))
    parser.add_argument("--output", type=Path, default=Path("artifacts/linux-package.json"))
    args = parser.parse_args()
    result = inspect((args.deb or args.rpm).resolve(), args.binary.resolve(), "deb" if args.deb else "rpm")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "nativeLibraries"}, indent=2))
