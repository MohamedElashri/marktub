# Arabic diacritic rendering

WebKitGTK 2.52.6 displaced parts of the same Arabic strings in both a plain
WebView page and the Marktub document. Chromium 158 rendered the same strings
with the same bundled font files and preserved the base-letter position.
In the horizontal Skia path, WebKit carries each glyph's X advance into the
text blob and drops its Y advance. That affects vertical mark attachment and
can move the whole glyph run when its first glyph has a vertical offset.
[WebKit report](https://bugs.webkit.org/show_bug.cgi?id=255972)
[Skia text code](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/graphics/skia/FontSkia.cpp)

The comparison sampled 18 base strings with 14 single and stacked marks at
400 and 700 weight. At 72 px it flagged an example when WebKitGTK retained
over eight percentage points less of the unmarked base than Chromium or when
the vertical ink bounds differed by over four pixels. The threshold is for
diagnosis at that size.

| Bundled font      |    Regular |       Bold |
| ----------------- | ---------: | ---------: |
| Noto Naskh Arabic | 248 of 252 | 248 of 252 |
| Noto Sans Arabic  | 222 of 252 | 221 of 252 |
| Amiri             | 114 of 252 | 114 of 252 |

For example, `بً` in regular Noto Naskh retained 5% of the unmarked base pixels
in WebKitGTK. The same font in Chromium retained 100%. In regular Amiri,
both engines retained 100% and painted the mark above the base. The common
marks on `ب` and `د` passed in Amiri in both engines. `حٍ`, `إً`, and
combinations such as `بٌّ` illustrate remaining positioning issues.

On Linux, Marktub checks for displaced ink before selecting an Arabic-only
Amiri fallback for the editor and reader. The fallback retains the document's
typography settings for size and spacing and keeps the chosen faces available
for other scripts. The underlying WebKitGTK defect still affects some Arabic
clusters and needs an engine-level correction.

The native regression checks above and below placement for fathatan, dammatan,
kasratan, fatha, damma, kasra, shadda, and sukun on eight bases in Read, Source,
both Split panes, and Visual. A separate part of the test inserts vocalized
words through native WebView editing and compares the saved UTF-8 bytes,
including the combining mark order, BOM, and CRLF endings.

Local verification passed 89 native X11 checks and the 19-check Wayland
smoke group. The compiled app also kept the selected Noto face in Chromium.

```sh
MARKTUB_E2E_GREP='Arabic editing layout|Arabic reading layout' pnpm test:e2e
```

## WebKitGTK source patch

The patch retains both coordinates in horizontal Skia glyph runs that have
a nonzero vertical advance. Runs with zero vertical advances still use
Skia's horizontal position buffer.

An isolated test ran the original and patched WebKitGTK 2.52.6 text-blob code
with the same Skia build, HarfBuzz shaping, and font files. Against Chromium,
the original code flagged 1,636 of 2,016 samples across four font families
and two weights. The patched code kept 100% of the base-letter ink in
every sample. All vertical bounds were within two pixels of Chromium at
72 px, including in `بً`, `بٌّ`, `إً`, and `حٍ`.

## Native Linux rendering

A standalone Pango 1.52.1 and Cairo test used the same 2,016 samples and
font files. It retained 100% of the base-letter ink in each sample. Both
vertical ink bounds were within two pixels of Chromium at 72 px.
This verifies Pango as a candidate renderer for the affected text. Native
editing now uses the same shaping path inside Marktub on affected Linux
systems.

The [Linux editing prototypes](../../benchmarks/arabic-rendering/README.md)
compare native GTK editing with a Pango outline layer in CodeMirror. They
include X11 timing, narrow-width screenshots, and a Wayland input check.

## Native Source and Split editing

Marktub uses a GTK TextView for plain Source editing and the source pane
in Split when an Arabic document is open and the Linux shaping probe detects
the WebKitGTK bug. GTK loads the same bundled Amiri face as the Arabic
compatibility renderer. CodeMirror's document model remains responsible
for source transactions, undo, preview updates, and the recovery journal.
GtkTextBuffer scalar offsets are translated to the model's UTF-16 offsets.
The bridge sends edits incrementally and merges overlapping native and
toolbar activity using the document model's change mapping.
With both Latin and Arabic keyboard layouts configured, GTK can use the
Latin mapping for shortcuts while leaving Arabic input to the active layout.

The native pane yields to the web surface for Visual, browser spellcheck,
folded source, find and replace, and overlapping dialogs or menus.
Read and the preview pane still use the web renderer.

The native regression edits a BOM and CRLF fixture containing stacked
marks, emoji, and a Unicode paragraph separator. It checks the native
pane's exact Split allocation, the updated preview, formatting and undo,
concurrent native and toolbar edits, and saved bytes.
The local Debian package was also exercised on X11 with an Arabic keyboard
layout. Typing `بً بٌّ`, pasting vocalized Arabic, and pressing Ctrl+S with
Arabic active preserved the exact file bytes. The packaged app also showed
the native editor and the rendered preview together in Split.

```sh
MARKTUB_E2E_GREP='Native GTK Arabic source' pnpm test:e2e
```
