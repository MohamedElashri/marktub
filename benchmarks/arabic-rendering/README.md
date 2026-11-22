# Linux Arabic editing prototypes

The native prototype edits Markdown source in a GTK 3 TextView. The WebView
prototype uses the repository's CodeMirror direction extension and system
WebKitGTK. It shapes visible lines containing Arabic marks with Pango, paints
the resulting Cairo outlines as SVG paths, and masks the WebKit text through
CSS Custom Highlights. The same text remains in the CodeMirror document for
selection and editing. Plain mode is the CodeMirror control.

These are standalone editors. The native prototype tests GTK editing and
wrapping. History, Markdown decorations, and synchronization with Marktub's
reader would need integration. The outline prototype falls back to the
WebKit glyphs on wrapped lines. Wrapping, caret geometry, per-token colors,
and composition would need more work for a live overlay.

## Run

Use the repository's Node and installed JavaScript dependencies. The Linux
prototype uses the distribution Python with PyGObject, Pycairo, GTK 3 and
WebKit2 4.1 introspection, and PangoCairo. X11 timing additionally uses
libXtst.

```sh
web_dir=$(mktemp -d)
node benchmarks/arabic-rendering/build.mjs "$web_dir"

env FONTCONFIG_FILE="$PWD/benchmarks/arabic-rendering/fontconfig.xml" \
  /usr/bin/python3 benchmarks/arabic-rendering/prototype.py native

env FONTCONFIG_FILE="$PWD/benchmarks/arabic-rendering/fontconfig.xml" \
  /usr/bin/python3 benchmarks/arabic-rendering/prototype.py overlay --web "$web_dir"
```

`--smoke` inserts stacked Arabic marks through the editor and checks the
full text. `--width 520` demonstrates narrow wrapping. To time edits, run
`--bench` with `GDK_BACKEND=x11` in a private X11 session. The benchmark
sends real X11 key events to the focused prototype. It types and deletes
one character in a mixed Arabic and Latin line, then polls a small part of
the window until the expected pixels appear. It uses a 2,000-line fixture
and hides transient scrollbars and the caret during capture. `--uncached`
evicts the next edited line from the outline cache before each key event.
The eviction is outside the timed interval. `--screenshot PATH` saves the
settled original frame during a benchmark.

The benchmark measures a window in a virtual X11 display. Physical keyboard
latency, monitor refresh, and the rest of Marktub are outside the measurement.

## Measured on WebKitGTK 2.52.6

Three Xvfb runs of 60 key events per configuration, in rotated order, at a
900 px editor width with accelerated compositing disabled. The ranges span
the three run statistics.

| Editor configuration                               | Median (ms) | 95th percentile (ms) |
| -------------------------------------------------- | ----------: | -------------------: |
| Native GTK                                         |     3.2–3.4 |                  3.8 |
| Plain CodeMirror control                           |   14.0–15.8 |            17.0–24.2 |
| CodeMirror with cached outlines                    |   21.3–25.0 |            25.9–32.7 |
| CodeMirror with an outline cache miss on each edit |   39.3–40.4 |            41.4–42.7 |

At 520 px wide, the native window painted the long Arabic paragraph with
correct relative mark placement. The outline editor skipped that wrapped
paragraph at both tested widths. In unwrapped lines, the outline and WebKit
text widths differed by up to 4.23 px. Both prototypes preserved the
entire document and combining mark order after an Arabic insertion in X11
and nested Weston. The smoke checks used editor input APIs for the Arabic
insertion and did not exercise a physical Arabic keyboard or IME.

GTK is the better candidate for Linux Source and the editor side of Split.
It owns shaping, wrapping, caret placement, and drawing together. The
outline technique remains a candidate for a read-only surface, where
selection and wrapping can be designed around a single layout.
