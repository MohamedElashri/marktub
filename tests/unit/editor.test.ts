import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { history, undo, redo } from "@codemirror/commands";
import { formattingKind } from "../../src/editor/visual";
import { format, tableCell } from "../../src/editor/author";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { ensureSyntaxTree } from "@codemirror/language";
import fixture from "../fixtures/fidelity.json";

describe("Source-backed formatting and edits", () => {
  it("preserves Markdown-looking text inside fences and indented code", () => {
    const source =
      "# Heading\n\n```md\n# Literal heading\n- Literal list\n```\n\n    # indented code\n\n- outer\n  - inner\n";
    let state = EditorState.create({ doc: source, extensions: [markdown()] });
    // Parsing has a scheduling budget; this probe asserts a completed tree.
    expect(ensureSyntaxTree(state, source.length, 1000)).not.toBeNull();
    state = state.update({}).state;
    const kind = (text: string) =>
      formattingKind(state, source.indexOf(text), text);
    expect(kind("# Heading")).toBe("heading");
    expect(kind("# Literal heading")).toBeNull();
    expect(kind("- Literal list")).toBeNull();
    expect(kind("    # indented code")).toBeNull();
    expect(kind("  - inner")).toBe("list");
  });

  it("formats two Unicode headings in one reversible transaction", () => {
    const source = "# Heading 🦀\n\nUnselected\n\n# العنوان\n";
    let state = EditorState.create({
      doc: source,
      selection: EditorSelection.create([
        EditorSelection.cursor(2),
        EditorSelection.cursor(source.indexOf("العنوان")),
      ]),
      extensions: [
        history(),
        markdown(),
        EditorState.allowMultipleSelections.of(true),
      ],
    });
    state = state.update(format(state, "heading2")).state;
    expect(state.doc.toString()).toBe(
      "## Heading 🦀\n\nUnselected\n\n## العنوان\n",
    );
    undo({
      state,
      dispatch: (transaction) => {
        state = transaction.state;
      },
    });
    expect(state.doc.toString()).toBe(source);
  });
  it("uses shared base UTF-16 coordinates and matches the byte-provenance fixture", () => {
    const state = EditorState.create({
      doc: fixture.original.replace(/^\uFEFF/, ""),
      extensions: [history()],
    });
    const next = state.update({ changes: fixture.changes }).state;
    expect(next.doc.toString()).toBe(
      fixture.expected.replace(/^\uFEFF/, "").replace(/\r\n?|\n/g, "\n"),
    );
  });

  it("patches one escaped-pipe table cell, then undoes/redoes without formatting siblings", () => {
    const source =
      "# Heading 🦀\n\n- nested العربية\n\n| first | second |\n| --- | --- |\n| `one \\| literal` | two |\n";
    let state = EditorState.create({
      doc: source,
      extensions: [history(), markdown({ base: markdownLanguage })],
    });
    state = state.update({
      changes: tableCell(state, source.indexOf("| first"), 2, 1, "文"),
    }).state;
    expect(state.doc.toString()).toBe(source.replace("two", "文"));
    const target = {
      get state() {
        return state;
      },
      dispatch: (transaction: { state: EditorState }) => {
        state = transaction.state;
      },
    };
    expect(undo(target)).toBe(true);
    expect(state.doc.toString()).toBe(source);
    expect(redo(target)).toBe(true);
    expect(state.doc.toString()).toBe(source.replace("two", "文"));
  });
});
