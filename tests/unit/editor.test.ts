import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { history, undo, redo } from "@codemirror/commands";
import { tableCellRange, formattingKind } from "../../src/editor/visual";
import { markdown } from "@codemirror/lang-markdown";
import { ensureSyntaxTree } from "@codemirror/language";
import fixture from "../fixtures/fidelity.json";

describe("CodeMirror source-backed probes", () => {
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

  it("changes a heading and nested Unicode list item with one reversible transaction", () => {
    const source = "# Heading 🦀\n\n- outer\n  - inner العربية\n";
    let state = EditorState.create({ doc: source, extensions: [history()] });
    const from = source.indexOf("inner");
    state = state.update({
      changes: [
        { from: 2, to: 9, insert: "Title" },
        { from, to: from + 5, insert: "child" },
      ],
    }).state;
    expect(state.doc.toString()).toBe(
      source.replace("Heading", "Title").replace("inner", "child"),
    );
    undo({
      state,
      dispatch: (transaction) => {
        state = transaction.state;
      },
    });
    expect(state.doc.toString()).toBe(source);
    expect(tableCellRange("|   | next |", 0)).toEqual({ from: 4, to: 4 });
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
      "# Heading 🦀\n\n- nested العربية\n\n| one \\| literal | two |\n";
    const lineStart = source.indexOf("| one");
    const cell = tableCellRange(source.slice(lineStart), 1);
    let state = EditorState.create({ doc: source, extensions: [history()] });
    state = state.update({
      changes: {
        from: lineStart + cell.from,
        to: lineStart + cell.to,
        insert: "文",
      },
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

  it("refuses ambiguous table code spans", () => {
    expect(() => tableCellRange("| `a|b` | c |", 0)).toThrow("Code spans");
  });
});
