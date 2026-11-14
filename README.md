# Marktub

An offline Markdown reader and source-preserving editor for macOS and Linux.
Built with Rust, Tauri 2, Svelte 5, TypeScript and CodeMirror 6, using one local
system WebView.

**Prerelease.** Native acceptance is verified on macOS ARM64. Linux, Intel and
minimum-macOS qualification, physical IME/screen-reader checks, sustained memory
qualification and public signing remain open. The complete status and architecture
decisions are maintained in [plan.md](plan.md), with a portable
[verification summary](docs/evidence/release-status.json).

## Read and write

Open a file or authorize a folder from New tab. Read offers an outline, search,
section navigation and a typography-focused layout. Large documents render
progressively; forward references remain visibly provisional until indexing finishes.

Source, Split and Visual share one CodeMirror document, selection and undo history.
Visual controls patch verified Markdown spans for formatting, tables, images, math
and diagrams. Unsupported constructs remain source. Ordinary Split/Visual editing
admits documents up to 2 MiB; larger documents use a bounded Source range with
window-local undo/find and whole-document native saves.

Saves preserve byte provenance, including BOM and mixed line endings. External
changes trigger comparison rather than silent replacement. Recovered documents
and the bundled example require **Save As** to a newly selected destination.
Image imports require native selection or reviewed validated bytes; copied images
remain on disk after text Undo. Review relative image references after Save As to
another folder.

KaTeX, MathJax, Mermaid, highlighting and renderer fonts are bundled and loaded
locally on demand. Math is expression-local: global definitions, cross-expression
labels/references and automatic numbering retain source with a diagnostic. Explicit
tags and unnumbered environments are supported. Standalone HTML embeds finalized
sections, local assets/notices and approved images, without scripts. PDF export
uses separately installed, selected Pandoc and Typst or TeX.

## Navigate and configure

Library provides authorized roots, bounded listings, quick preview/search, tabs,
recent files and reviewed file operations. Remembered paths do not restore grants;
select their containing folder again after restarting. Up to four tabs can be open.
Duplicate recent filenames show parent-folder context.

The document toolbar keeps modes and Save actions visible while writing. **More**
contains Settings, Reload, saved backups and **Export & run**. In Read focus, use
**Exit focus** to reveal secondary actions. Settings provides scoped literal
`$key=value` overrides, four curated light/dark themes, validated custom colors,
installed fonts and shortcut rebinding. Custom themes cannot contain CSS or URLs.

| Action     | macOS       | Linux        |
| ---------- | ----------- | ------------ |
| Open       | Cmd O       | Ctrl O       |
| Save       | Cmd S       | Ctrl S       |
| Save As    | Cmd Shift S | Ctrl Shift S |
| Quick open | Cmd P       | Ctrl P       |
| Commands   | Cmd Shift P | Ctrl Shift P |
| Settings   | Cmd comma   | Ctrl comma   |

## Git, review and optional tools

Git compares saved Disk/Index/HEAD/history with bounded unified/split/intraline
hunks. Stage/unstage uses native whole-hunk IDs. Discard requires confirmation and
an exact private backup. Close Source sessions before mutations. Unsupported
index/config/filter/encoding states remain read-only with a reason. No commit,
checkout, hook/filter execution or Git transport is exposed.

Review is opt-in and captures saved document revisions independently of Source.
Comments/replies retain verified anchors; ambiguous matches are marked orphaned.
Only explicit **Mark saved capture reviewed** advances acknowledgement. Save a
comment to persist its draft. Portable feedback omits paths/snippets by default;
archived feedback restores no document-write authority.

Shell, SQLite and trusted TeX start disabled each session. Native folder trust,
selected tools/databases and per-run native confirmation are required. Working
captures flush pending Source edits first. SQLite is readonly by default;
confirmed DML transactions roll back on cancellation/errors. Tools have bounded
time/output and attached process-group cleanup. App-crash supervision stops
attached tool processes; deliberately detached or hostile same-user processes
are outside this guarantee. Shell/TeX run with normal user privileges, not an OS
sandbox. No tools, packages or network resources are installed automatically.

## Build

Pinned tools: Node **26.13.1**, pnpm **12.18.0**, Rust **1.99.1** with rustfmt and
Clippy, plus the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).
The declared targets are macOS 13+ and Ubuntu 24.04 amd64 with GTK 3/WebKitGTK 4.1.
Only the recorded macOS ARM64 host has been locally qualified.

```sh
pnpm install --frozen-lockfile
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
pnpm tauri dev
```

```sh
pnpm tauri build --bundles app,dmg   # macOS
pnpm tauri build --bundles deb       # Linux
pnpm check:release
```

The installed app needs no Node server. Vite and the loopback WebDriver server are
restricted to development/test configurations. Normal release builds reject the
e2e feature and exclude fixture commands/evaluators. Rebuild normally after native
testing before distributing an artifact. Local macOS packages are ad-hoc signed;
Developer ID signing/notarization requires a separate release-owner decision.

## Verify and prepare source

```sh
pnpm check:repository
pnpm check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
pnpm test:e2e
```

Native tests exercise the real WebView, backend and filesystem using disposable
picker fixtures and synthetic keys. They do not certify physical IME or assistive
technology. Tests, security guards, reproducible benchmarks and required license
texts are maintained source, not disposable build output.

`check:repository` checks the Git publication inventory, including untracked source.
It rejects generated output, local machine reports, credentials, recognizable
private keys/tokens, oversized entries and local home paths without echoing secrets.
It supplements review; it is not a guarantee that arbitrary confidential text
cannot be published. Generated ACL/schema files, dependencies, build outputs and
local evidence are ignored. Historical development reports/prototypes are archived
locally under `artifacts/publication-history/`; their evidence summaries remain in
`plan.md`.

```sh
pnpm source:archive
```

The deterministic source snapshot and file-hash manifest are written under
`artifacts/releases/`. No Git history, local caches or historical reports are
included. Creating this snapshot does not commit, upload or publish a release.

## Measure

```sh
pnpm fixtures
pnpm bench:open
pnpm bench:dom
pnpm bench:baseline
python3 benchmarks/startup.py --binary artifacts/baseline/Marktub.app/Contents/MacOS/marktub
pnpm tauri build --bundles app
python3 benchmarks/release_profile.py --binary src-tauri/target/release/bundle/macos/Marktub.app/Contents/MacOS/marktub --label empty --rusage --samples 3 --interval 10 --output artifacts/empty-profile.json
```

Serialize Cargo/Tauri builds and run measurements without compilation. Benchmark
results belong to their executable hash. Startup tools explicitly enable native
stderr timing with `MARKTUB_PROFILE_STARTUP=1`; normal launches emit no timing logs.
Page load is distinct from first useful/interactive paint; JSDOM is not native
scroll/paint evidence. RSS differs from physical footprint. Optional macOS
`source_memory.py --responsibility` brackets samples with ownership checks;
unknown/changing attribution or surviving helpers makes a trace inconclusive.
No normal-release evaluator or profiling endpoint is exposed.

## Recovery and release limits

After an interrupted edit, **Restore**, inspect the recovered text, then **Save As**.
Recovery never restores the old path's write authority. Keep local edits and use
Compare/independent copies when a file changes externally. Do not delete app-private
storage to dismiss a warning: it can contain unsaved work, feedback or discard
backups. Capacity refusals preserve existing records. Large-session range changes
require durable ACKs; independent Save As/reopen is the capacity rescue path.

Markdown is untrusted and cannot grant paths, commands or trust. Renderer assets
stay local; there are no remote fonts or telemetry. Native IPC exposes explicit
scoped handles. OS single-instance transport is local integration, not a sandbox
against hostile same-user processes. Native file associations are alternate
Markdown associations and do not change your defaults; CLI/OS handoff accepts
local `.md`, `.markdown` and `.txt` files with spaces/Unicode.

Strict dependency audits remain release gates: local development-dependency
mitigations do not erase upstream affected-version findings. Platform/input,
font/printing, sustained memory/latency, license/distribution and actual remote CI
qualification remain explicit in the release ledger. Source publication readiness
is separate from declaring a fully qualified public binary release.

Original code is MIT licensed. [NOTICE](NOTICE), `THIRD_PARTY_LICENSES.txt` and the
generated license inventories retain upstream code/font notices. Standalone HTML
contains notices for its embedded JavaScript/fonts. Collection of additional
upstream notices is explicit (`python3 scripts/fetch-rust-notices.py`), never a
runtime or automatic build download.
