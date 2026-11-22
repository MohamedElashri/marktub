import { StateEffect, StateField, type EditorState } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import { tableAt, imageSpan, writableBlock } from "./author";
import { grammar, emptyContext } from "../reader/grammar";
import { AuthorMath } from "./math";
const display = grammar();
const mathViews = new WeakMap<
  EditorView,
  { doc: EditorState["doc"]; context: AuthorMath }
>();
function documentMath(view: EditorView) {
  const prior = mathViews.get(view);
  if (prior?.doc === view.state.doc) return prior.context;
  prior?.context.destroy();
  const context = new AuthorMath(view.state.doc.toString());
  mathViews.set(view, { doc: view.state.doc, context });
  return context;
}
const images = new WeakMap<EditorView, Map<string, string>>();
const refreshImage = StateEffect.define<null>();
let imageEpoch = 0;
export function rememberImage(view: EditorView, path: string, dataUrl: string) {
  if (
    !/^data:image\/(png|jpeg);base64,/.test(dataUrl) ||
    dataUrl.length > 8 * 1024 * 1024
  )
    return;
  const cache = images.get(view) ?? new Map<string, string>();
  cache.delete(path);
  while (
    cache.size >= 8 ||
    [...cache.values()].reduce((n, v) => n + v.length, 0) + dataUrl.length >
      8 * 1024 * 1024
  ) {
    const key = cache.keys().next().value;
    if (!key) break;
    cache.delete(key);
  }
  cache.set(path, dataUrl);
  images.set(view, cache);
  imageEpoch++;
  view.dispatch({ effects: refreshImage.of(null) });
}

import { renderRich } from "../renderers/rich";
import { sanitizeRich } from "../reader/safe";
const viewport = StateEffect.define<{
  from: number;
  to: number;
  composing: boolean;
}>();
class Block extends WidgetType {
  readonly epoch = imageEpoch;
  constructor(
    readonly kind: string,
    readonly from: number,
    readonly source: string,
    readonly rows?: string[][],
    readonly doc?: EditorState["doc"],
  ) {
    super();
  }
  eq(other: Block) {
    return (
      this.from === other.from &&
      this.kind === other.kind &&
      this.source === other.source &&
      this.epoch === other.epoch &&
      this.doc === other.doc
    );
  }
  toDOM(view: EditorView) {
    const host = document.createElement(
      this.kind === "math" || this.kind === "image" ? "span" : "div",
    );
    host.className = "cm-author-block";
    if (this.kind === "image") {
      const token = display
        .parseInline(this.source, { ...emptyContext() })[0]
        ?.children?.find((t) => t.type === "image");
      let path = String(token?.attrGet("src") ?? "");
      try {
        path = decodeURIComponent(path);
      } catch {
        path = "";
      }
      const src = images.get(view)?.get(path);
      if (src) {
        const image = document.createElement("img");
        image.src = src;
        image.alt = token?.content ?? "";
        image.style.maxWidth = "240px";
        image.style.maxHeight = "240px";
        host.append(image);
      }
      host.draggable = true;
      host.ondragstart = (e) => {
        e.dataTransfer?.setData("application/x-marktub-block", "move");
        host.dispatchEvent(
          new CustomEvent("author-dragstart", {
            bubbles: true,
            detail: { from: this.from },
          }),
        );
      };
    }

    const button = (label: string, row?: number, column?: number) => {
      const b = document.createElement("button");
      b.textContent = label;
      b.type = "button";
      b.onmousedown = (e) => e.preventDefault();
      b.onclick = () =>
        host.dispatchEvent(
          new CustomEvent("author-block", {
            bubbles: true,
            detail: { kind: this.kind, from: this.from, row, column },
          }),
        );
      return b;
    };
    if (this.rows) {
      const table = document.createElement("table");
      for (const [i, row] of this.rows.entries()) {
        if (i === 1) continue;
        const tr = document.createElement("tr");
        for (const [column, cell] of row.entries()) {
          const td = document.createElement(i === 0 ? "th" : "td");
          td.append(
            button(cell.replace(/\\\|/g, "|") || "Empty cell", i, column),
          );
          tr.append(td);
        }
        table.append(tr);
      }
      host.append(table);
    } else {
      host.append(
        button(
          this.kind === "image"
            ? "Edit image"
            : this.kind === "math"
              ? "Edit math"
              : "Edit diagram",
        ),
      );
      if (this.kind !== "image") {
        const preview = document.createElement(
          this.kind === "math" ? "span" : "div",
        );
        preview.className = "author-block-preview";
        preview.textContent = "Preparing local preview…";
        host.append(preview);
        // Widget DOM is attached by CodeMirror before this microtask. Retired
        // widgets must not keep queued diagram work running after a mode/close.
        void Promise.resolve()
          .then(() =>
            this.kind === "math"
              ? documentMath(view).render(
                  this.from,
                  this.from + 1,
                  this.source,
                  () => preview.isConnected && view.state.doc === this.doc,
                )
              : renderRich(
                  {
                    kind: this.kind === "math" ? "math" : "mermaid",
                    source: this.source,
                    display: this.kind !== "math",
                  },
                  "auto",
                  () => preview.isConnected,
                ),
          )
          .then((result) => {
            if (preview.isConnected)
              preview.innerHTML = sanitizeRich(result, "author-widget");
          })
          .catch(() => {
            if (preview.isConnected)
              preview.textContent = "Preview unavailable. Edit source.";
          });
      }
    }
    return host;
  }
  ignoreEvent() {
    return true;
  }
}
function build(
  state: EditorState,
  range: { from: number; to: number; composing: boolean },
): DecorationSet {
  if (range.composing) return Decoration.none;
  const decorations: import("@codemirror/state").Range<Decoration>[] = [];
  let end = 0,
    count = 0;
  const inactive = (from: number, to: number) =>
    !state.selection.ranges.some((r) => r.from <= to && r.to >= from);
  syntaxTree(state).iterate({
    from: range.from,
    to: range.to,
    enter(node) {
      if (node.from < end || count >= 32) return false;
      let kind = "",
        body = "",
        rows: string[][] | undefined,
        from = node.from,
        to = node.to;
      if (
        node.name === "Table" &&
        node.to - node.from <= 16384 &&
        inactive(from, to)
      ) {
        try {
          const t = tableAt(state, from);
          if (t.rows.length > 22) return false;
          kind = "table";
          rows = t.rows.map((r) => r.map((c) => c.text));
          body = state.doc.sliceString(t.from, t.to);
          from = t.from;
          to = t.to;
        } catch {
          return false;
        }
      }
      if (
        node.name === "FencedCode" &&
        node.to - node.from <= 16384 &&
        inactive(from, to)
      ) {
        const text = state.doc.sliceString(from, to);
        const m = /^```mermaid[^\n]*\n([\s\S]*?)\n```\s*$/.exec(text);
        if (m) {
          kind = "mermaid";
          body = m[1];
        }
      }
      // Inline images remain an explicit control, never an automatic filesystem read.
      if (node.name === "Image" && inactive(from, to)) {
        const image = imageSpan(state, from);
        if (!image) return false;
        kind = "image";
        ({ from, to } = image);
        body = state.doc.sliceString(from, to);
      }
      if (kind) {
        decorations.push(
          Decoration.replace({
            widget: new Block(kind, from, body, rows),
            block: kind === "table" || kind === "mermaid",
          }).range(from, to),
        );
        end = to;
        count++;
        return false;
      }
    },
  });
  // Simple expression-local math agrees with the display grammar's conservative delimiters.
  for (
    let p = state.doc.lineAt(range.from).number;
    p <= state.doc.lineAt(Math.min(range.to, state.doc.length)).number &&
    count < 32;
    p++
  ) {
    const line = state.doc.line(p);
    if (!inactive(line.from, line.to)) continue;
    const node = syntaxTree(state).resolveInner(
      line.from + Math.min(1, line.length),
      1,
    );
    let code = false;
    for (let n = node; n; n = n.parent!)
      if (["FencedCode", "CodeBlock", "InlineCode"].includes(n.name))
        code = true;
    if (code) continue;
    for (const match of line.text.matchAll(
      /(?<![\\\w])(\${1,2})([^$\n]{1,8192})\1/g,
    )) {
      const tokens = display.parseInline(match[0], { ...emptyContext() })[0]
        ?.children;
      if (
        tokens?.length !== 1 ||
        tokens[0].type !== "math" ||
        tokens[0].content !== match[2]
      )
        continue;
      const from = line.from + match.index!;
      const to = from + match[0].length;
      if (!writableBlock(state, from) || !writableBlock(state, to)) continue;
      if (decorations.some((d) => d.from < to && d.to > from)) continue;
      decorations.push(
        Decoration.replace({
          widget: new Block("math", from, match[2], undefined, state.doc),
        }).range(from, to),
      );
      count++;
    }
  }
  return Decoration.set(decorations, true);
}
const blocks = StateField.define<{
  range: { from: number; to: number; composing: boolean };
  decorations: DecorationSet;
}>({
  create(state) {
    const range = {
      from: 0,
      to: Math.min(16384, state.doc.length),
      composing: false,
    };
    return { range, decorations: build(state, range) };
  },
  update(value, tr) {
    let range = {
      ...value.range,
      from: Math.min(value.range.from, tr.newDoc.length),
      to: Math.min(value.range.to, tr.newDoc.length),
    };
    for (const effect of tr.effects)
      if (effect.is(viewport)) range = effect.value;
    return {
      range,
      decorations:
        syntaxTree(tr.startState) !== syntaxTree(tr.state) ||
        tr.docChanged ||
        tr.selection ||
        tr.effects.some((e) => e.is(viewport) || e.is(refreshImage))
          ? build(tr.state, range)
          : value.decorations,
    };
  },
  provide: (field) =>
    EditorView.decorations.from(field, (value) => value.decorations),
});
const track = ViewPlugin.fromClass(
  class {
    dead = false;
    constructor(readonly view: EditorView) {
      this.publish();
    }
    update(update: { viewportChanged: boolean; view: EditorView }) {
      const prior = mathViews.get(this.view);
      if (prior && prior.doc !== this.view.state.doc) {
        prior.context.destroy();
        mathViews.delete(this.view);
      }
      if (update.viewportChanged) this.publish();
    }
    publish() {
      queueMicrotask(() => {
        if (!this.dead)
          this.view.dispatch({
            effects: viewport.of({
              ...this.view.viewport,
              composing: this.view.compositionStarted,
            }),
          });
      });
    }
    destroy() {
      this.dead = true;
      mathViews.get(this.view)?.context.destroy();
      mathViews.delete(this.view);
      images.delete(this.view);
    }
  },
);
export const visualBlocks = [
  blocks,
  track,
  EditorView.domEventHandlers({
    compositionstart: (_e, view) => {
      setTimeout(() => {
        if (view.dom.isConnected)
          view.dispatch({
            effects: viewport.of({ ...view.viewport, composing: true }),
          });
      }, 0);
    },
    compositionend: (_e, view) => {
      setTimeout(() => {
        if (view.dom.isConnected)
          view.dispatch({
            effects: viewport.of({ ...view.viewport, composing: false }),
          });
      }, 0);
    },
  }),
];
