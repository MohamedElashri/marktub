import {
  Decoration,
  EditorView,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import type { Line } from "@codemirror/state";

const automatic = Decoration.line({ attributes: { dir: "auto" } });
const arabicProse = Decoration.line({
  class: "cm-arabic-prose",
  attributes: { dir: "auto" },
});
const arabicHeading = Decoration.line({
  class: "cm-arabic-prose cm-arabic-heading",
  attributes: { dir: "auto" },
});
const code = Decoration.line({
  class: "cm-source-code",
  attributes: { dir: "ltr" },
});
const inlineCode = Decoration.mark({ class: "cm-source-code" });
const literalSpace = Decoration.mark({ class: "cm-arabic-literal-space" });
const literalSpaces = /^[ \t]+|[ \t]*\t[ \t]*| {2,}|[ \t]+$/g;
const blockCode = /^(FencedCode|CodeBlock)$/;
const heading = /^(ATXHeading[1-6]|SetextHeading[12])$/;

export const arabicScript = /\p{Script=Arabic}/u;

function lineTypography(tree: ReturnType<typeof syntaxTree>, line: Line) {
  for (let node = tree.resolveInner(line.from, 1).cursor(); ;) {
    if (blockCode.test(node.name)) return code;
    if (!node.parent()) break;
  }
  for (const match of line.text.matchAll(/\p{Script=Arabic}+/gu)) {
    for (let node = tree.resolveInner(line.from + match.index, 1).cursor(); ;) {
      if (blockCode.test(node.name)) return code;
      if (node.name === "InlineCode") break;
      if (heading.test(node.name)) return arabicHeading;
      if (!node.parent()) return arabicProse;
    }
  }
  return automatic;
}

function lineDecorations(view: EditorView, preserveSpaces: boolean) {
  const decorations = [];
  let last = -1;
  let style = automatic;
  const tree = syntaxTree(view.state);
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to;) {
      const line = view.state.doc.lineAt(pos);
      if (line.from > last) {
        style = lineTypography(tree, line);
        decorations.push(style.range(line.from));
      }
      if (
        preserveSpaces &&
        (style === arabicProse || style === arabicHeading)
      ) {
        const visibleText = line.text.slice(pos - line.from, to - line.from);
        for (const match of visibleText.matchAll(literalSpaces)) {
          const start = pos + match.index;
          decorations.push(literalSpace.range(start, start + match[0].length));
        }
      }
      last = line.from;
      pos = line.to + 1;
    }
  }
  for (const { from, to } of view.visibleRanges)
    tree.iterate({
      from,
      to,
      enter(node) {
        if (node.name === "InlineCode")
          decorations.push(inlineCode.range(node.from, node.to));
      },
    });
  return Decoration.set(decorations, true);
}

// Resolve each rendered logical line in the browser. CodeMirror must use that
// same direction for cursor movement and coordinates, including wrapped lines.
export const writingDirections = (preserveSpaces: boolean) => [
  EditorView.perLineTextDirection.of(true),
  ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      private inputTimer: ReturnType<typeof setTimeout> | undefined;
      constructor(readonly view: EditorView) {
        this.decorations = lineDecorations(view, preserveSpaces);
      }
      update(update: ViewUpdate) {
        if (
          update.docChanged ||
          update.viewportChanged ||
          syntaxTree(update.startState) !== syntaxTree(update.state)
        )
          this.decorations = lineDecorations(update.view, preserveSpaces);
      }
      beforeInput() {
        if (
          !preserveSpaces ||
          !this.view.contentDOM.querySelector(".cm-arabic-prose")
        )
          return;
        // WebKit rebalances collapsed spaces before reporting a native edit.
        // Preserve them during the default action, then restore the paint style.
        this.view.dom.classList.add("cm-native-input");
        clearTimeout(this.inputTimer);
        this.inputTimer = setTimeout(() => this.afterInput(), 0);
      }
      afterInput() {
        clearTimeout(this.inputTimer);
        this.view.dom.classList.remove("cm-native-input");
      }
      destroy() {
        this.afterInput();
      }
    },
    {
      decorations: (plugin) => plugin.decorations,
      eventObservers: {
        beforeinput() {
          this.beforeInput();
        },
        input() {
          this.afterInput();
        },
      },
    },
  ),
  EditorView.theme({ ".cm-line": { textAlign: "start" } }),
  preserveSpaces
    ? EditorView.editorAttributes.of({ class: "cm-arabic-spacing" })
    : [],
];
