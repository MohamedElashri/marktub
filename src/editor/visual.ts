import {
  EditorView,
  Decoration,
  ViewPlugin,
  WidgetType,
} from "@codemirror/view";
import type { DecorationSet, ViewUpdate } from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";

export function formattingKind(
  state: EditorState,
  from: number,
  text: string,
): "heading" | "list" | null {
  const heading = /^(#{1,6})\s+/.exec(text);
  const marker = heading ?? /^(?:\s*[-+*]\s+)/.exec(text);
  if (!marker) return null;
  let node = syntaxTree(state).resolveInner(from + marker[0].length, 1);
  for (;;) {
    if (
      ["FencedCode", "CodeBlock", "CodeText", "InlineCode"].includes(node.name)
    )
      return null;
    if (heading && /^ATXHeading[1-6]$/.test(node.name)) return "heading";
    if (!heading && node.name === "ListItem") return "list";
    if (!node.parent) return null;
    node = node.parent;
  }
}

class Bullet extends WidgetType {
  toDOM() {
    const mark = document.createElement("span");
    mark.textContent = "• ";
    mark.setAttribute("aria-hidden", "true");
    return mark;
  }
}
class Task extends WidgetType {
  constructor(
    readonly from: number,
    readonly checked: boolean,
  ) {
    super();
  }
  eq(other: Task) {
    return this.from === other.from && this.checked === other.checked;
  }
  toDOM(view: EditorView) {
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = this.checked;
    input.setAttribute("aria-label", "Toggle task");
    input.onmousedown = (e) => e.preventDefault();
    input.onchange = () => {
      view.dispatch({
        changes: {
          from: this.from + 1,
          to: this.from + 2,
          insert: input.checked ? "x" : " ",
        },
        userEvent: "input.task",
      });
    };
    return input;
  }
  ignoreEvent() {
    return true;
  }
}
function decorate(view: EditorView): DecorationSet {
  const marks = [];
  if (view.compositionStarted) return Decoration.none;
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to;) {
      const line = view.state.doc.lineAt(pos);
      const kind = formattingKind(view.state, line.from, line.text);
      const heading = /^(#{1,6})\s+/.exec(line.text);
      const marker = heading ?? /^(?:\s*[-+*]\s+)/.exec(line.text);
      const active = view.state.selection.ranges.some(
        (r) => r.from <= line.to && r.to >= line.from,
      );
      if (kind === "heading")
        marks.push(
          Decoration.line({ class: "cm-live-heading" }).range(line.from),
        );
      if (kind && marker && !active)
        marks.push(
          Decoration.replace({
            widget:
              kind === "list" && !/^\s*[-+*] \[/.test(line.text)
                ? new Bullet()
                : undefined,
          }).range(
            line.from +
              (kind === "list" ? (/^\s*/.exec(line.text)?.[0].length ?? 0) : 0),
            line.from + marker[0].length,
          ),
        );
      if (line.to >= to || line.to === view.state.doc.length) break;
      pos = line.to + 1;
    }
  }
  for (const visible of view.visibleRanges)
    syntaxTree(view.state).iterate({
      from: visible.from,
      to: visible.to,
      enter(node) {
        const line = view.state.doc.lineAt(node.from);
        if (node.name === "Table") return false;

        const active = view.state.selection.ranges.some(
          (r) => r.from <= line.to && r.to >= line.from,
        );
        if (
          [
            "StrongEmphasis",
            "Emphasis",
            "InlineCode",
            "Link",
            "Strikethrough",
          ].includes(node.name)
        ) {
          const cls =
            node.name === "StrongEmphasis"
              ? "cm-author-strong"
              : node.name === "Emphasis"
                ? "cm-author-em"
                : node.name === "InlineCode"
                  ? "cm-author-code"
                  : node.name === "Link"
                    ? "cm-author-link"
                    : "cm-author-strike";
          marks.push(Decoration.mark({ class: cls }).range(node.from, node.to));
          if (!active) {
            const children = [];
            for (let c = node.node.firstChild; c; c = c.nextSibling)
              children.push(c);
            for (const c of children)
              if (
                [
                  "EmphasisMark",
                  "CodeMark",
                  "LinkMark",
                  "StrikethroughMark",
                ].includes(c.name)
              )
                marks.push(Decoration.replace({}).range(c.from, c.to));
            if (node.name === "Link") {
              const url = children.find((c) => c.name === "URL");
              if (url)
                marks.push(Decoration.replace({}).range(url.from, url.to));
            }
          }
        }
        if (node.name === "TaskMarker" && !active) {
          const text = view.state.doc.sliceString(node.from, node.to);
          if (/^\[[ xX]\]$/.test(text))
            marks.push(
              Decoration.replace({
                widget: new Task(node.from, text[1] !== " "),
              }).range(node.from, node.to),
            );
        }
        if (node.name === "QuoteMark" && !active)
          marks.push(Decoration.replace({}).range(node.from, node.to));
      },
    });
  return Decoration.set(marks, true);
}

export const liveFormatting = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = decorate(view);
    }
    update(update: ViewUpdate) {
      if (
        update.transactions.length ||
        update.docChanged ||
        update.selectionSet ||
        update.viewportChanged ||
        syntaxTree(update.startState) !== syntaxTree(update.state)
      )
        this.decorations = decorate(update.view);
    }
  },
  { decorations: (value) => value.decorations },
);
