import {
  Decoration,
  EditorView,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";

const automatic = Decoration.line({ attributes: { dir: "auto" } });
const code = Decoration.line({ attributes: { dir: "ltr" } });

function directions(view: EditorView) {
  const lines = [];
  let last = -1;
  const tree = syntaxTree(view.state);
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to;) {
      const line = view.state.doc.lineAt(pos);
      if (line.from > last) {
        let node = tree.resolveInner(line.from, 1);
        while (node.parent && !["FencedCode", "CodeBlock"].includes(node.name))
          node = node.parent;
        lines.push(
          (["FencedCode", "CodeBlock"].includes(node.name)
            ? code
            : automatic
          ).range(line.from),
        );
      }
      last = line.from;
      pos = line.to + 1;
    }
  }
  return Decoration.set(lines);
}

// Resolve each rendered logical line in the browser. CodeMirror must use that
// same direction for cursor movement and coordinates, including wrapped lines.
export const writingDirections = [
  EditorView.perLineTextDirection.of(true),
  ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = directions(view);
      }
      update(update: ViewUpdate) {
        if (
          update.docChanged ||
          update.viewportChanged ||
          syntaxTree(update.startState) !== syntaxTree(update.state)
        )
          this.decorations = directions(update.view);
      }
    },
    { decorations: (plugin) => plugin.decorations },
  ),
  EditorView.theme({ ".cm-line": { textAlign: "start" } }),
];
