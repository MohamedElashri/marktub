# Development

Marktub uses Rust and Tauri with Svelte, TypeScript and CodeMirror in one system
WebView. Tool versions are pinned in `.node-version`, `package.json` and
`rust-toolchain.toml`.

## Run locally

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for
your platform, then:

```sh
pnpm install --frozen-lockfile
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
pnpm tauri dev
```

## Check changes

```sh
pnpm check:repository
pnpm check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
pnpm test:e2e
```

`pnpm check` runs frontend/tooling tests, typechecking, lint and formatting, and
generates resources needed by native checks. Native e2e needs the converters and
desktop packages listed in [CI](.github/workflows/check.yml). Serialize Cargo/Tauri
builds. The E2E app uses its own application identifier and private store so it
can run beside the normal app without attaching to the user's instance.
For dependency changes, also run CI's audit and source-override checks.
Development file filters treat repeated `+(...)` and `*(...)` globs as literals;
use braces or `@(...)` alternatives instead. Reviewed local backports and raw
upstream advisory findings are checked in CI.

For editor regressions, start with a small fixture that fails on the original
code. Check the command output and the rendered Markdown independently. Cover
mixed script text, selections in both directions, and literal Markdown inside
code where relevant. In the native test, run the editor action and verify the
saved bytes, including any BOM and original line endings. For text alignment,
compare painted pixels against a same-font reference in the WebView. DOM
dimensions alone cannot detect displaced glyphs.

Use `MARKTUB_E2E_GREP='Author experience|Arabic editing layout' pnpm test:e2e`
for a focused native pass. Run the complete desktop matrix on the release
candidate to cover the broader reading, editing and packaging workflows.
Concurrent native runs need distinct `TAURI_WEBDRIVER_PORT` values as well as
separate displays and private storage. Otherwise a worker can connect to the
other run's application.

Before a macOS release, also launch the normal app copied from the DMG. Open a
saved note with Shell and SQL fences, cancel each native run confirmation, then
repeat and run each block. Confirm that the dialog appears as a sheet on the
Marktub window, displays the exact source literally (including `%`, `<` and `&`),
and returns to usable inline controls after both Cancel and Run. The automated
e2e suite substitutes native selections and consent; it cannot qualify this
dialog presentation. Check native Open/Save dialogs and tab right-click actions
in the same normal app.

On Linux, `xvfb-run -a dbus-run-session -- python3 scripts/e2e-wayland.py` reuses
the E2E build under Weston to check actual Wayland focus and rendering. It needs
Weston and Xvfb; it does not certify physical input or a particular desktop shell.

## Build packages

Use the command for your host:

```sh
python3 scripts/build-macos.py       # universal macOS app and DMG
pnpm tauri build --bundles deb,rpm   # Linux x86_64
pnpm check:release
```

macOS needs both desktop Rust targets:
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`. Packages are under
`src-tauri/target/`, including the universal target directory on macOS. Rebuild
normally after e2e testing before packaging. Build-generated attribution belongs
under `artifacts/`; keep required upstream license texts in source.

Linux CI checks both package formats, launches the installed DEB on Ubuntu, and
installs the RPM in a disposable Fedora 44 container to verify dependencies and
runtime library resolution. Normalize the RPM filename with
`python3 scripts/prepare-rpm.py --version VERSION` after a fresh build.

Update packages are signed in CI after package checks, using the dedicated Actions
secret and the public key pinned in source. macOS uses the same universal app.
For a release, download all three `update-*.json` manifests and their signed packages
from the same successful CI run, then combine them with plain-text release notes:

```sh
python3 scripts/update-manifest.py combine update-*.json --notes release-notes.txt --output latest.json --rpm-output latest-rpm.json
```

Upload both `latest.json` and `latest-rpm.json` and the exact versioned packages alongside the normal release
assets. Never commit the private signing key or put it in build logs. The initial
version with update support must be installed using the existing installer.
The original two-platform feed remains unchanged for 0.3.0 clients; installed
RPMs use the separate RPM feed. Run Installation checks manually after publishing
to verify public downloads on macOS, Debian/Ubuntu and Fedora.
Before publication, its optional `candidate_run` input can verify Fedora
installation of a built RPM when only qualification scripts have changed.

Run `pnpm check:repository` before `pnpm source:archive`. Scoped release validation
and known limits are recorded in [release-status.json](docs/evidence/release-status.json).

## Contribute

Keep changes focused and include relevant validation in your pull request. Bug
reports should include the OS, app revision, reproduction steps and a small
redacted fixture. Preserve failure evidence and app-private recovery data.

Treat Markdown as untrusted input. Preserve native permissions, exact document
bytes, undo and recoverable saves; keep test hooks out of release builds.
