# Development

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for
your platform. Use the versions in `.node-version`, `package.json`, and
`rust-toolchain.toml`.

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

The frontend lives in `src/` and the Rust app in `src-tauri/`. Run these checks
before opening a pull request.

```sh
pnpm check:repository
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Use `pnpm test:e2e` from a desktop session for native UI changes.
[Desktop CI](.github/workflows/check.yml) lists the prerequisites and runs the
macOS and Linux packaging checks.
