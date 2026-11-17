"""Verify release-version consistency and the source of local dependency fixes.

Cargo audit scans registry packages, not local path overrides. This check admits
only the recorded, hash-verified overrides. --native also verifies optimized
GLib iteration and invalid-input diagnostics through the real Rust compiler.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(root, metadata):
    manifest = tomllib.loads((root / "src-tauri/Cargo.toml").read_text())
    application = json.loads((root / "package.json").read_text())
    config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
    version = manifest["package"]["version"]
    assert application["version"] == version, "Application manifest versions differ"
    assert config.get("version") in (None, version), "Tauri version differs from the Cargo fallback"
    record = json.loads((root / "docs/evidence/dependency-overrides.json").read_text())
    packages = {p["name"]: p for p in record["packages"]}
    assert len(packages) == len(record["packages"]), "Duplicate dependency override"
    assert set(manifest["patch"]["crates-io"]) == set(packages), "Unreviewed dependency override"
    expected_local = {root / "src-tauri/Cargo.toml"}
    for name, package in packages.items():
        folder = root / package["path"]
        assert folder.resolve() == folder and folder.is_relative_to(root / "src-tauri/vendor"), "Override outside the maintained vendor directory"
        expected_local.add(folder / "Cargo.toml")
        assert manifest["patch"]["crates-io"][name] == {"path": folder.relative_to(root / "src-tauri").as_posix()}, "Override source mismatch"
        identity = tomllib.loads((folder / "Cargo.toml").read_text())["package"]
        assert (identity["name"], identity["version"]) == (name, package["version"]), "Upstream package identity changed"
        expected = {f["path"] for f in package["files"]}
        actual = {p.relative_to(folder).as_posix() for p in folder.rglob("*") if p.is_file()}
        assert actual == expected, f"Unreviewed or missing files in {name}"
        for file in package["files"]:
            path = folder / file["path"]
            assert path.resolve() == path and path.is_relative_to(folder), "Override source traverses a symlink"
            assert digest(path) == file["sha256"], f"Override source changed: {name}/{file['path']}"
            assert len(file["upstreamSha256"]) == 64, "Missing original source provenance"
        assert digest(root / package["patch"]) == package["patchSha256"], f"Override patch changed: {name}"
    local = {Path(p["manifest_path"]).resolve() for p in metadata["packages"] if p["source"] is None}
    assert local == expected_local, "Resolved graph contains an unreviewed local package"
    removed = {"proc-macro-error", "proc-macro-error-attr", "proc-macro-error2", "proc-macro-error-attr2", "zvariant", "zvariant_derive", "zbus_names", "chrono"}
    assert not removed.intersection(p["name"] for p in metadata["packages"]), "Unmaintained implementation returned to the graph"
    locked = tomllib.loads((root / "src-tauri/Cargo.lock").read_text())
    assert not removed.intersection(p["name"] for p in locked.get("package", [])), "Unmaintained implementation returned to the lockfile"
    return version, len(packages)


def native(root):
    subprocess.run(["cargo", "test", "--manifest-path", str(root / "src-tauri/Cargo.toml"), "--locked", "--release", "--lib", "dependency_tests::glib_string_variant_iteration_is_sound_under_optimization", "--", "--exact"], check=True)
    diagnostics(root)
    serialization(root)


def serialization(root):
    """Exercise Tauri's used adapters and refuse the removed integration."""
    logs = root / "artifacts/dependency-diagnostics"
    logs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="marktub-serde-diagnostics-") as directory:
        fixture = Path(directory)
        (fixture / "src").mkdir()
        (fixture / "Cargo.toml").write_text(
            '[package]\nname="marktub-serde-diagnostics"\nversion="0.0.0"\nedition="2024"\n'
            '[dependencies]\nserde={version="1",features=["derive"]}\nserde_json="1"\n'
            'serde_with={path=' + json.dumps(str(root / "src-tauri/vendor/serde_with")) + '}\n'
        )
        (fixture / "Cargo.lock").write_bytes((root / "src-tauri/Cargo.lock").read_bytes())
        (fixture / "src/main.rs").write_text('''
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Config {
    #[serde_as(as = "Vec<DisplayFromStr>")]
    ids: Vec<u64>,
    title: String,
    optional: Option<String>,
}
fn main() {
    let config: Config = serde_json::from_str(r#"{"ids":["42","18446744073709551615"],"title":"العربية <>&"}"#).unwrap();
    assert_eq!(config.ids, [42, u64::MAX]);
    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(json["title"], "العربية <>&");
    assert_eq!(json["ids"][0], "42");
    assert!(json.get("optional").is_none());
    assert_eq!(serde_json::from_value::<Config>(json).unwrap(), config);
    assert!(serde_json::from_str::<Config>(r#"{"ids":["-1"],"title":"bad"}"#).is_err());
}
''')
        command = ["cargo", "run", "--offline", "--manifest-path", str(fixture / "Cargo.toml"), "--target-dir", str(root / "src-tauri/target")]
        run = subprocess.run(command, capture_output=True, text=True)
        (logs / "serde-valid.log").write_text(run.stdout + run.stderr)
        assert run.returncode == 0, "Tauri serialization adapters failed"
        for feature in ["chrono", "chrono_0_4"]:
            run = subprocess.run(command + ["--features", "serde_with/" + feature], capture_output=True, text=True)
            output = run.stdout + run.stderr
            (logs / ("serde-" + feature + ".log")).write_text(output)
            assert run.returncode != 0 and "Marktub excludes serde_with chrono integration" in output, "Removed chrono integration did not fail explicitly"
    print("PASS: serde adapters roundtrip Unicode, optional fields and bounded integers; both chrono feature names fail explicitly")


def diagnostics(root):
    cases = {
        "boxed": ("#[derive(glib_macros::Boxed)] struct Bad;", "requires #[boxed_type"),
        "subclass": ("#[glib_macros::object_subclass] struct Bad;", "`impl` block"),
        "variant": ("#[derive(glib_macros::Variant)] #[variant_enum(missing)] enum Bad { One }", "unknown type"),
        "closure": ("fn main() { let _ = glib_macros::closure!(@strong self => move || {}); }", "Can't use `self`"),
        "template": ("#[derive(gtk3_macros::CompositeTemplate)] struct Bad;", "requires #[template"),
    }
    logs = root / "artifacts/dependency-diagnostics"
    logs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="marktub-macro-diagnostics-") as directory:
        fixture = Path(directory)
        (fixture / "src").mkdir()
        manifest = '[package]\nname="marktub-diagnostics"\nversion="0.0.0"\nedition="2021"\n[dependencies]\n'
        for name in ["glib-macros", "gtk3-macros"]:
            manifest += name + '={path=' + json.dumps(str(root / "src-tauri/vendor" / name)) + '}\n'
        manifest += 'gtk="=0.18.2"\nglib="=0.18.5"\n[patch.crates-io]\n'
        for name in ["glib", "glib-macros", "gtk3-macros"]:
            manifest += name + '={path=' + json.dumps(str(root / "src-tauri/vendor" / name)) + '}\n'
        (fixture / "Cargo.toml").write_text(manifest)
        (fixture / "Cargo.lock").write_bytes((root / "src-tauri/Cargo.lock").read_bytes())
        for name, (code, expected) in cases.items():
            (fixture / "src/main.rs").write_text(code + ("\nfn main() {}\n" if name != "closure" else "\n"))
            run = subprocess.run(["cargo", "check", "--offline", "--manifest-path", str(fixture / "Cargo.toml"), "--target-dir", str(root / "src-tauri/target")], capture_output=True, text=True)
            output = run.stdout + run.stderr
            (logs / (name + ".log")).write_text(output)
            assert run.returncode != 0 and expected in output, f"Missing native diagnostic: {name}"
            assert "proc-macro derive panicked" not in output and "custom attribute panicked" not in output, f"Diagnostic panicked: {name}"
        (fixture / "src/template.ui").write_text("<interface><template class='MarktubTestWidget' parent='GtkBox'><child><object class='GtkLabel' id='label'/></child></template></interface>")
        (fixture / "src/main.rs").write_text('''
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

#[derive(Clone, Debug, glib_macros::Boxed)]
#[boxed_type(name = "MarktubTestBox")]
pub struct Value(String);

#[derive(Debug, glib_macros::Variant)]
pub enum State { Ready, Done }

mod imp {
    use super::*;
    #[derive(Debug, Default, gtk3_macros::CompositeTemplate)]
    #[template(file = "template.ui")]
    pub struct TestWidget { #[template_child] pub label: TemplateChild<gtk::Label> }

    #[glib_macros::object_subclass]
    impl ObjectSubclass for TestWidget {
        const NAME: &'static str = "MarktubTestWidget";
        type Type = super::TestWidget;
        type ParentType = gtk::Box;
        fn class_init(klass: &mut Self::Class) { Self::bind_template(klass); }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) { obj.init_template(); }
    }
    impl ObjectImpl for TestWidget {}
    impl WidgetImpl for TestWidget {}
    impl ContainerImpl for TestWidget {}
    impl BoxImpl for TestWidget {}
}
glib::wrapper! {
    pub struct TestWidget(ObjectSubclass<imp::TestWidget>) @extends gtk::Widget, gtk::Container, gtk::Box;
}
fn main() {
    let value = Value(String::from("captured"));
    let captured = value.0;
    let _closure = glib_macros::closure!(@strong captured => move || { let _ = captured.len(); });
}
''')
        run = subprocess.run(["cargo", "check", "--offline", "--manifest-path", str(fixture / "Cargo.toml"), "--target-dir", str(root / "src-tauri/target")], capture_output=True, text=True)
        (logs / "valid.log").write_text(run.stdout + run.stderr)
        assert run.returncode == 0, "Valid GTK/GLib macro consumer failed to compile"
    print(f"PASS: {len(cases)} compiler-error fixtures and valid boxed/variant/closure/subclass/template expansion")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--manifest-path", str(root / "src-tauri/Cargo.toml"), "--locked", "--offline", "--format-version", "1"], text=True))
    version, count = validate(root, metadata)
    print(f"PASS: release version {version}; {count} reviewed dependency overrides, exact source/patch hashes and no unmaintained predecessors")
    if args.native:
        native(root)


if __name__ == "__main__":
    main()
