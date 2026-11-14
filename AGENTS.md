# Working on Marktub

Contributor and maintainer guide. See [README.md](README.md) for product usage and [plan.md](plan.md) for the specification, phase history and release qualifications.

## Contributing

Keep changes focused on a concrete use case or defect. For bugs, include the OS, app revision, reproduction steps and a minimal redacted fixture; preserve evidence before fixing a file-integrity failure. Describe the resulting behavior and relevant validation in a pull request. Avoid unrelated reformatting, new runtime dependencies and speculative performance claims.

Read the relevant phase's completion/deviation notes before implementation. Keep new phase decisions and evidence in that section of plan.md. This repository uses three Markdown documentation entry points; required legal notices and machine-readable evidence remain separate.

## Development

| Tool | Pinned version                                     |
| ---- | -------------------------------------------------- |
| Node | 26.13.1 (`.node-version`)                          |
| pnpm | 12.18.0 (`package.json`)                           |
| Rust | 1.99.1, rustfmt and Clippy (`rust-toolchain.toml`) |

Install the [Tauri native prerequisites](https://v2.tauri.app/start/prerequisites/) for your host. The declared Linux target is Ubuntu 24.04 amd64 with GTK 3/WebKitGTK 4.1. Rust, Tauri 2, Svelte 5, TypeScript and CodeMirror 6 are the locked stack; the installed app uses one local system WebView.

```sh
pnpm install --frozen-lockfile
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
pnpm tauri dev
```

## Checks

```sh
pnpm check:repository
pnpm check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
pnpm test:e2e
```

`pnpm check` covers typechecking, lint, frontend tests, Node/Python tooling tests and formatting. Native e2e tests build an isolated test configuration and exercise the actual WebView/backend with disposable fixtures and synthetic keys. They do not certify physical IME or accessibility. Serialize Cargo/Tauri builds; rebuild normally after e2e testing before distributing an artifact.

Security checks use `pnpm audit --prod`, `pnpm audit`, and pinned `cargo-audit 0.23.0` with `cargo audit --file src-tauri/Cargo.lock --deny unsound --deny unmaintained`. Record database revision, failures and Not Run scope; do not hide findings with ignores. Current results belong in Phase 9 of plan.md, with portable facts in `docs/evidence/release-status.json`.

## Build and package

```sh
python3 scripts/build-macos.py       # macOS: ad-hoc app + DMG, no Apple credentials
pnpm tauri build --bundles deb        # Linux: Debian package on a Linux host
pnpm check:release
```

Use the command for your host. Outputs are under `src-tauri/target/release/bundle/`. The macOS channel is ad-hoc and unnotarized; it does not establish an Apple-verified publisher. Never change trust/quarantine settings in build scripts. `check:release` checks the normal frontend, permissions, executable and dependency graph for test hooks. Validate an actual local DMG separately:

```sh
python3 scripts/check-macos-package.py --dmg <path-to-dmg>
```

Before preparing source, run `pnpm check:repository`, then `pnpm source:archive`. The deterministic archive and hash manifest land in `artifacts/releases/`; Git history, caches, generated files and local reports are excluded. The inventory guard detects common credential/token/home-path leaks but cannot replace human review. Tests, benchmarks and license texts are maintained source. Never delete app-private histories as part of repository cleanup.

## Measurement

Use `pnpm fixtures`, `pnpm bench:open` and `pnpm bench:dom` for their documented fixture/DOM scopes. `pnpm bench:baseline` creates the blank-app control; rebuild the normal app afterward. Native tools require a selected executable, for example:

```sh
python3 benchmarks/startup.py --binary <app-executable>
python3 benchmarks/release_profile.py --binary <app-executable> --label empty --rusage --samples 3 --interval 10 --output artifacts/empty-profile.json
```

Measure without concurrent compilation and bind results to the executable hash, host, workload and helper lifecycles. Page load is not interactive paint; RSS is not physical footprint; browser mocks are not native behavior. Startup timing is explicit opt-in (`MARKTUB_PROFILE_STARTUP=1`). Optional macOS `source_memory.py --responsibility` uses ownership/incarnation checks; unknown ownership or surviving helpers makes attribution inconclusive. No release evaluator is exposed.

## Implementation constraints

Read `plan.md`, including previous completion/deviation notes, before changing architecture or advancing scope.
Do not create new Markdown files. Keep all phase plans, decisions, evidence summaries,
completion and deviation notes in that phase’s section of `plan.md`. Non-Markdown
fixtures and machine-readable evidence may remain separate.
Use Rust + Tauri 2 + Svelte 5 + TypeScript + CodeMirror 6; one local WebView.
Markdown is untrusted data. Keep explicit app-command permissions and backend
document grants. No generic path read/write, shell, network or SQL bridge.
Phase 1 Source saves require the documented integrity gates. Preserve native
grants, exact-byte undo, expected-base checks and recoverable failure paths.
Recovered documents and app-owned resources require Save As; never auto-reauthorize paths.
Preserve byte provenance; never assume CodeMirror serialization preserves mixed EOL.
Run targeted tests and the documented checks. Record platform and Not Run cases.
Use `--manifest-path src-tauri/Cargo.toml` for Cargo commands. Never claim native
WebView behavior from browser mocks. Exclude all e2e servers/evaluation hooks from
normal release builds. Serialize Cargo/Tauri builds in the shared target directory. Publication preparation is authorized. Keep unresolved release qualifications explicit in plan.md.

Reader sections remain provisional until context scanning finishes. Keep global
definitions/heading order and bounded source fallbacks; compare paging with whole
parsing before claiming parity. Rich renderers/fonts stay local and lazy. Images
require native grants; exports use a finalized selected revision and new native
destination. Preserve standalone CSP, sanitization and embedded font notices.

Preference changes must keep the existing CM state, byte provenance, undo and pending
journal barriers. Use the native closed catalogue/effective validator for GUI and
literal imports; custom themes are typed colors/fonts, never CSS or resource URLs.
Workspace scope follows an authorized root (else the file parent); shortcuts are
global/session only. Invalid inherited settings retain their literal records and
apply safe defaults with diagnostics. Recovered documents still require Save As.

Author modes share the ordinary CM view/Text and receipt-backed undo. Rich controls
patch verified source spans; never serialize Markdown or fetch image paths from
Visual. Image data requires native selection, opaque offers or validated pasted
bytes; copy is exclusive and text undo never deletes copied files.
Range switches require durable ACKs and rotate transport generations. Find/undo
are local to the window; saves and Read use full native provenance. Large sessions
retain native replay receipts/tokens across switches and skip full checkpoints;
never garbage-collect live history differently from replay. Capacity rescue must
remain independent-copy/Save As, not path reauthorization or discarded edits.
Synthetic WebView composition events do not certify real OS IME/accessibility.

Git uses explicit ephemeral repository/file/preview handles and raw native captures.
Stage/unstage consumes native whole-hunk IDs under the real index.lock; preserve
unrelated entries and metadata, revalidate late, never remove a replaced/foreign lock.
Unsupported config/attributes/index states remain read-only. Never invoke configured
filters, textconv, hooks, tools or transports. Git mutations share the no-live-Source
reservation. Discard needs exact durable backup before expected-base atomic save;
backup export is a new native destination and never restores a path grant.
Normal builds use the bounded native-hunk view; exclude experimental renderers.

Review is opt-in and uses bounded native saved captures, never working Source text.
Freeze the prior displayed revision at view entry; only explicit Mark Reviewed
advances acknowledgement. Preserve unsaved feedback drafts on failure and require
explicit discard before leaving. Source remains the only document-write authority.
Native-verify every anchor against its retained original bytes; ambiguous/orphaned
matches get no invented current positions. Pin comment/history snapshots and refuse
capacity rather than prune feedback. Paths/snippets are omitted from exports by
default; actual paths need consent plus a live grant. Archived exports restore no
path authority. Recognize unique same-parent inode renames only; following needs
native selection. Keep Review off by default, lazy and independent of index reset.

PDF export uses a finalized native saved or durable-ACK working capture, closed
Pandoc AST/template/options and private staged assets; never repository defaults,
filters/raw code or unapproved paths. HTML stays independent and script-free.
Local runners remain session-default-off with separate per-kind enable/root trust,
one-shot native confirmation of captured code/hash/tool/cwd and opaque job handles.
Settle and flush pending Source edits before validating a working execution capture.
SQLite uses a separately selected native DB, readonly/query-only by default,
defensive/untrusted-schema settings and a closed authorizer. Writes are confirmed
DML transactions; cancellation/errors/bounds roll back. No ATTACH, extensions,
user pragmas or schema writes. Never treat a mutable DB as immutable.
Shell/TeX have normal OS privileges; process-group cancel, output/time limits and
TeX no-shell-escape are not an OS sandbox. No package/network bootstrap. Open only
hash-revalidated PDFs produced/exported by the app; never document-supplied viewers.
