<p align="center">
  <img src="src-tauri/icons/app.svg" width="88" height="88" alt="Marktub logo">
</p>

<h1 align="center">Marktub</h1>

<p align="center"><strong>Markdown, at your own pace.</strong><br>
An offline Markdown reader and source-preserving desktop editor.</p>

<p align="center">
  <a href="#get-started">Get started</a> ·
  <a href="#features">Features</a> ·
  <a href="AGENTS.md#contributing">Contribute</a> ·
  <a href="#status">Status</a> ·
  <a href="LICENSE">MIT license</a>
</p>

![Marktub's Git workbench showing a side-by-side comparison of saved local changes](docs/assets/git-workbench.png)

_The Git workbench on macOS. An actual app capture using a disposable test repository._

## Features

- **Read first.** A quiet reading layout, outline, search, section navigation and focus mode.
- **Four document modes.** Read, Source, Split and Visual share the same document. Visual tools edit Markdown spans and preserve the surrounding source.
- **Keep ordinary files.** Work directly with local folders, up to four tabs, contextual recent files and quick open. No vault, account or cloud service.
- **Write technical documents.** Local math, equation numbering and references, Mermaid diagrams, highlighted code, tables and footnotes. Renderers and fonts load locally on demand.
- **Inspect changes.** Saved Git diffs, history, whole-hunk stage/unstage and backed-up discard. Opt-in document Review adds anchored comments and portable feedback.
- **Export your work.** Script-free standalone HTML with embedded assets. PDF through separately installed and selected Pandoc with Typst or XeLaTeX.

The desktop shell uses Rust and Tauri with one system WebView. Installed builds need no Node server; there are no remote fonts or telemetry.

## Get started

**Prerelease:** local native verification covers macOS Apple Silicon. Other declared targets remain in qualification; see [Status](#status).

To run from source, install Node **26.13.1**, pnpm **12.18.0**, Rust **1.99.1** and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). From the repository root:

```sh
pnpm install --frozen-lockfile
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
pnpm tauri dev
```

For an installable macOS app/DMG or Linux Debian package, follow [Build and package](AGENTS.md#build-and-package).

macOS packages use **ad-hoc signing and are not notarized**. Check the package checksum before opening. If you choose to trust a blocked download, use Apple's [per-app Privacy & Security flow](https://support.apple.com/en-us/102445). Marktub does not disable Gatekeeper or remove quarantine.

## Everyday workflow

1. **Open a file or folder.** New tab offers both actions and a bundled example. Files open in Read. Remembered folders require native reselection after restart.
2. **Choose how to work.** Source exposes Markdown; Split adds a preview; Visual provides source-backed formatting. Use Save to write changes to disk.
3. **Find secondary actions in More.** Settings, Reload, saved backups and Export & run live there. In reading focus, Exit focus reveals the other controls.
4. **Review deliberately.** Git uses saved Disk/Index/HEAD/history, not unsaved editor text. Document Review advances acknowledgement only when you choose Mark saved capture reviewed.

| Action      | macOS         | Linux           |
| ----------- | ------------- | --------------- |
| Open / Save | Cmd O / Cmd S | Ctrl O / Ctrl S |
| Save As     | Cmd Shift S   | Ctrl Shift S    |
| Quick open  | Cmd P         | Ctrl P          |
| Commands    | Cmd Shift P   | Ctrl Shift P    |
| Settings    | Cmd comma     | Ctrl comma      |

Settings includes light/dark themes, typed custom colors, installed fonts, scoped `$key=value` overrides and shortcut rebinding.

<details>
<summary>File safety, recovery and current limits</summary>

- Saves preserve byte provenance, including BOM and mixed line endings. External changes prompt comparison. Restored documents and the bundled example require **Save As** to a new native destination.
- After an interruption, Restore, inspect the text, then Save As. Keep app-private storage: it can contain unsaved edits, feedback and discard backups. Capacity refusals preserve existing records.
- Split/Visual admit documents up to 2 MiB. Ordinary Source admission is 16 MiB; larger admitted documents use bounded ranges with window-local undo/find and full native saves. Large or unsupported rich blocks retain source with a reason.
- Bounded document-wide math supports numbering, labels, references and shared macros. Beyond-budget or ambiguous formulas retain source. PDF shares a 16-asset/24 MiB budget across images, diagrams and math; engine font/layout warnings are reported.
- Images require explicit approval. Text Undo does not delete copied image files; check relative references after Save As to a different folder.
- Close live Source sessions before Git mutations. Unsupported Git states remain read-only. Marktub does not expose commits, checkout, hooks, configured filters or network transport.

</details>

<details>
<summary>Optional local execution</summary>

Shell, SQLite and trusted TeX start disabled each session. Enable a tool kind, select its tool/database, trust a folder and confirm the captured code before running. Reading or exporting a document never enables these runners or runs its code blocks.

SQLite is readonly by default; confirmed DML writes use transactions and roll back on errors/cancellation. Shell and TeX have normal OS user privileges. Time/output limits and attached-process cleanup are **not an OS sandbox**; deliberately detached processes are outside that cleanup guarantee. No tools or packages are installed automatically.

</details>

## Status

Marktub targets macOS 13+ on Apple Silicon/Intel and Ubuntu 24.04 amd64 with GTK 3/WebKitGTK 4.1. Windows is outside scope. Linux, Intel, minimum-macOS, physical IME/screen-reader/printing and broader performance/distribution checks remain open. Five Rust informational advisories also remain unresolved; source publication readiness is separate from binary-release acceptance.

See the [release summary](docs/evidence/release-status.json) for recorded checks and [Phase 9](plan.md#phase-9-performance-hardening-accessibility-packaging-release-review) for the qualification ledger.

## Development and documentation

- [Contributor guide](AGENTS.md): setup, checks, packaging, measurement and implementation constraints.
- [Specification and phase record](plan.md): architecture, decisions, completion/deviation notes and remaining work.

For a bug report, include the app revision, OS, steps, expected/actual behavior and a minimal redacted document. Contribution guidance is in [AGENTS.md](AGENTS.md#contributing).

## License

Original code is [MIT licensed](LICENSE). [NOTICE](NOTICE) and [third-party notices](THIRD_PARTY_LICENSES.txt) retain upstream code/font terms. Standalone HTML carries notices for its embedded resources.
