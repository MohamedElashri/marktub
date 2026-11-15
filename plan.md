# Implementation and release plan

Product usage is in [README.md](README.md); development commands and invariants
are in [AGENTS.md](AGENTS.md). This document tracks the current implementation
and release work. Historical development notes remain in Git history; local
qualification traces are excluded from published source.

## Architecture

Rust and Tauri provide the native boundary; Svelte, TypeScript and CodeMirror
provide one local system WebView. Markdown is untrusted data. Documents stay
ordinary files, with explicit native grants and one source-editing authority.

- Reader parsing and rich rendering are bounded, local and lazy. Global Markdown
  context must finalize before export. Unsupported constructs retain source.
- Source, Split and Visual share CodeMirror state and receipt-backed undo. Native
  provenance preserves exact bytes; saves revalidate the expected base.
- Folder, image, Git, Review, export and runner access use purpose-specific
  native handles. There is no generic filesystem, shell, network or SQL bridge.
- Review uses saved captures. Git mutations use native hunk identities and
  expected index state; discard requires a durable independent backup.
- Recovery never restores path authority. Exports use finalized captures and new
  selected destinations. Local runners are explicitly enabled, trusted and
  confirmed per kind, with bounded output/time and process-group cleanup.

## Implemented product

| Area         | Current behavior                                                                         |
| ------------ | ---------------------------------------------------------------------------------------- |
| Reading      | Outline, find, anchors, reading position, focus and bounded large-file paging            |
| Editing      | Shared Source/Split/Visual, byte-preserving patches, undo, validated save and recovery   |
| Rich content | Local math/context, diagrams, tables, images, code and source fallbacks                  |
| Library      | Selected roots, search, tabs, recents, bookmarks and safe file operations                |
| Git          | Saved comparisons/history, bounded diffs, whole-hunk stage/unstage and backed-up discard |
| Review       | Opt-in saved revision tracking, verified anchors, feedback and privacy-scoped exports    |
| Preferences  | Validated themes/fonts, scoped literal settings, shortcuts and command palette           |
| Export/tools | Standalone HTML, captured PDF, default-off shell/SQLite/trusted TeX                      |

## Release preparation

Release versions belong in manifests; tool versions belong in their pin files.
Documentation must not repeat them. Machine-readable provenance and qualification
records retain exact revisions, hashes and tested tool versions.

- [x] Remediate dependency advisories with verified code/dependency changes.
      Local dependency overrides require retained provenance, source integrity
      checks and optimized regressions; a registry-only audit is insufficient.
- [x] Verify uninterrupted native X11 and Wayland workflows, including leaving
      and returning from the external PDF viewer without weakening focus checks.
- [x] Remove unused application components and obsolete diagnostic scripts;
      retain maintained tests, benchmarks, legal notices and safety comments.
- [x] Run repository, frontend/tooling, strict Rust and native WebView checks.
- [x] Build normal packages after testing; verify test-hook isolation, resources,
      metadata, executable identity and an offline installed-app workflow.
- [x] Refresh source/package checksums and reproduce the source archive.
- [ ] Confirm the physical input/accessibility/printing and unavailable-platform
      checks below before describing them as verified.

## Qualifications requiring an operator or another host

- Current shared changes need native macOS verification, including the declared
  architectures and minimum supported system. Historical Mac packages do not
  certify the current source.
- Physical IME, keyboard/image integration, screen-reader speech/navigation,
  native font dialogs, HiDPI and printing need real desktop/input coverage.
- Broader sustained large-Source, multi-tab, Git and Review resource tests, plus
  compositor/input-to-paint/DOM/IPC distributions remain separate from finite
  process-memory and page-load observations.
- Filesystem and tool tests cover their named fixtures, not every removable,
  network/FUSE, crash, hostile-process or font/tool-version configuration.
- Actual hosted CI and downloaded-package opening are distinct from a local
  build or a container install. macOS uses the owner-selected ad-hoc channel;
  paid signing/notarization is outside the selected release scope.

Exact results and remaining gates belong in
[release-status.json](docs/evidence/release-status.json). Preserve failed evidence;
never convert an unrun check into a pass or hide dependency findings with ignores.

## Current release decisions

- Backport the upstream GLib string-iterator soundness fix without changing its
  upstream version identity. An optimized regression reproduces the original
  crash; the same test must pass on the fixed source.
- Replace unmaintained macro-error dependencies with existing `syn::Error`
  diagnostics, and migrate the single-instance plugin to the merged maintained
  D-Bus implementation. Keep upstream licenses and file/patch provenance.
- Simplify documentation around product use, contributor commands and this
  current checklist. Keep detailed evidence out of the prose entry points.
- Linux acceptance now includes uninterrupted X11 and GNOME Wayland workflows,
  normal package isolation, an offline Ubuntu runtime install, observed-helper
  cleanup, repeated mode switches and finite three-tab Source workloads. These
  results do not certify physical input or other platforms.
