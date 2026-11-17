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
builds. For dependency changes, also run CI's audit and source-override checks.
On Linux, `xvfb-run -a dbus-run-session -- python3 scripts/e2e-wayland.py` reuses
the E2E build under Weston to check actual Wayland focus and rendering. It needs
Weston and Xvfb; it does not certify physical input or a particular desktop shell.

## Build packages

Use the command for your host:

```sh
python3 scripts/build-macos.py       # universal macOS app and DMG
pnpm tauri build --bundles deb       # Ubuntu amd64
pnpm check:release
```

macOS needs both desktop Rust targets:
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`. Packages are under
`src-tauri/target/`, including the universal target directory on macOS. Rebuild
normally after e2e testing before packaging. Build-generated attribution belongs
under `artifacts/`; keep required upstream license texts in source.

Update packages are signed in CI after package checks, using the dedicated Actions
secret and the public key pinned in source. macOS uses the same universal app.
For a release, download both `update-*.json` manifests and their signed packages
from the same successful CI run, then combine them with plain-text release notes:

```sh
python3 scripts/update-manifest.py combine update-*.json --notes release-notes.txt --output latest.json
```

Upload `latest.json` and the exact versioned packages alongside the normal release
assets. Never commit the private signing key or put it in build logs. The initial
version with update support must be installed using the existing installer.

Run `pnpm check:repository` before `pnpm source:archive`. Scoped release validation
and known limits are recorded in [release-status.json](docs/evidence/release-status.json).

## Contribute

Keep changes focused and include relevant validation in your pull request. Bug
reports should include the OS, app revision, reproduction steps and a small
redacted fixture. Preserve failure evidence and app-private recovery data.

Treat Markdown as untrusted input. Preserve native permissions, exact document
bytes, undo and recoverable saves; keep test hooks out of release builds.
