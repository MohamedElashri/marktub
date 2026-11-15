# Working on Marktub

Use [README.md](README.md) for product usage and [plan.md](plan.md) for architecture
and release work. Keep these three documentation entry points concise. Do not add
Markdown files. Put exact provenance and verification facts in machine-readable
records; keep raw local reports under ignored artifacts.

## Development

Use the Node pin in `.node-version`, the package manager in `package.json`, and the
Rust toolchain/components in `rust-toolchain.toml`. Install the
[Tauri native prerequisites](https://v2.tauri.app/start/prerequisites/) for the host.
CI defines the tested host matrix. If Homebrew hides Linux GTK/WebKit libraries,
set `PKG_CONFIG=/usr/bin/pkg-config` for native commands.

```sh
pnpm install --frozen-lockfile
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
pnpm tauri dev
```

## Verification

```sh
pnpm check:repository
pnpm check:dependencies
pnpm check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
python3 scripts/check-dependencies.py --native # Linux
pnpm test:e2e
pnpm audit --prod
pnpm audit
cargo audit --file src-tauri/Cargo.lock --deny unsound --deny unmaintained
```

Use the audit tool selected by CI. Record its database revision and findings; no
advisory ignores. Registry audit does not cover local dependency overrides: their
provenance/integrity checks and optimized regressions are also required.

Native e2e uses an isolated test build and disposable fixtures. It verifies the
actual WebView/backend with synthetic input, not physical IME or screen-reader
usability. Export tests need the selected converters and Poppler tools.
The Linux viewer roundtrip also needs Evince and system Python GI. The
checksum-pinned test-tool downloader and `MARKTUB_E2E_*` selections are development
tools; the installed app never downloads packages. Serialize Cargo/Tauri builds.

## Packaging

```sh
python3 scripts/build-macos.py
pnpm tauri build --bundles deb
pnpm check:release
```

Choose the host command. Rebuild normally after e2e or blank-baseline builds.
Outputs are under `src-tauri/target/release/bundle/`. Validate the actual package
with `scripts/check-macos-package.py --dmg` or `scripts/check-linux-package.py --deb`.
The macOS channel is ad-hoc and unnotarized; never change trust/quarantine settings.

On Linux, `/usr/bin/python3 scripts/check-linux-installed.py --binary <executable>
--example` checks the normal app through AT-SPI with an isolated preserved profile.
It needs system Python GI/AT-SPI and a desktop session; CI supplies Xvfb/Openbox.

Before preparing source, run `pnpm check:repository`, then `pnpm source:archive`.
The inventory guard supplements human review. Publication preparation is
allowed; keep unresolved qualifications explicit. Never delete app-private
histories as repository cleanup.

## Measurement

`pnpm fixtures`, `pnpm bench:open` and `pnpm bench:dom` have fixture/range/scaffold
scopes. `pnpm bench:baseline` produces a blank control; rebuild normally afterward.
Use `benchmarks/startup.py --binary` and `benchmarks/release_profile.py --binary
--label --output` for external normal-process measurements. The installed Linux
check supports mode cycles, bounded large-Source fixtures, multiple tabs and
post-close sampling.

Do not compile concurrently. Bind results to binary/workload hashes, host and
helper lifecycles. Page load is not interactive paint, RSS/PSS is not physical
footprint, and context switches are not wakeups or energy. Reject uncertain
ownership or incomplete intervals. Startup timing is opt-in through
`MARKTUB_PROFILE_STARTUP`; no release evaluator is permitted.

## Implementation invariants

- Keep the pinned Rust, Tauri, Svelte, TypeScript and CodeMirror stack with one local WebView.
  Markdown is untrusted. Keep explicit command permissions and native grants;
  never expose generic path, shell, network or SQL access.
- Source is the write authority. Preserve exact bytes, mixed EOL/BOM provenance,
  expected-base validation, receipt undo and recoverable save failures. Recovered
  documents and app resources require a new native Save As grant.
- Reader context stays provisional until scanning finishes. Preserve global
  definitions/heading order and bounded source fallbacks. Compare paging against
  whole parsing before parity claims. Keep rich renderers/fonts local and lazy.
- Preferences use the closed native catalogue/validator; retain literal invalid
  records with safe defaults/diagnostics. Preserve CM state, undo and journal
  barriers. Themes are typed values, never CSS or resource URLs. Workspace scope
  follows an authorized root or file parent; shortcuts are global/session only.
- Author controls patch verified spans in the ordinary CM view/Text. Never
  serialize Markdown or fetch image paths from Visual. Images require native
  selection, opaque offers or validated bytes. Copy is exclusive; text undo
  never deletes copied assets.
- Range switches require durable ACKs and new transport generations. Undo/find
  are local; saves/Read use full provenance. Keep live replay receipts/tokens and
  skip full checkpoints for large sessions. Capacity rescue is an independent
  copy/Save As, never discarded edits or path reauthorization.
- Git uses ephemeral native repository/file/hunk handles and raw captures. Stage
  holds the real index lock, preserves unrelated metadata and revalidates late.
  Never remove a foreign/replaced lock or invoke configured filters, hooks,
  textconv, tools or transports. Unsupported states are read-only. Mutations
  exclude live Source; discard needs exact durable backup before atomic save.
- Review is default-off, lazy and uses saved native captures. Freeze displayed
  revisions; only explicit Mark Reviewed advances acknowledgement. Preserve
  drafts on failure; leaving requires explicit discard. Verify anchors against
  retained bytes and never invent positions. Pin history/comments and refuse
  capacity instead of pruning. Exports omit paths/snippets by default; real paths
  require consent and a live grant. Archived data restores no authority. Only
  unique same-parent inode renames may be recognized; following needs selection.
- PDF uses finalized saved/durable-ACK captures, closed AST/template/options and
  private approved assets. No repository defaults, filters or raw execution.
  HTML stays script-free with CSP, sanitization and embedded font notices.
- Runners are session-default-off with separate per-kind enable/root trust and
  one-shot native confirmation of captured code/hash/tool/cwd. Flush Source
  before working captures. SQLite is separately selected, readonly/query-only,
  defensive/untrusted-schema with a closed authorizer. Confirmed DML rolls back
  on cancellation/error/bounds; no ATTACH, extensions, pragmas or schema writes.
- Shell/TeX have ordinary OS privileges. Process-group cancellation and limits
  are not an OS sandbox; TeX disables shell escape. Open only hash-revalidated
  app-produced/exported PDFs through the fixed system viewer.

Keep changes focused, preserve failure evidence and run relevant checks. Include
OS/revision, reproduction steps and a redacted fixture for integrity bugs. Avoid
unrelated formatting, speculative performance claims and unnecessary dependencies.
