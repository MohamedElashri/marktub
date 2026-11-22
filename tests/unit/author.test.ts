import { describe, it, expect } from "vitest";
import { EditorState, EditorSelection } from "@codemirror/state";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { emptyContext, grammar } from "../../src/reader/grammar";
import {
  format,
  tableAt,
  tableCell,
  tableShape,
  imageText,
  imageSpan,
  richSpan,
} from "../../src/editor/author";
const state = (doc: string, at = 0) =>
  EditorState.create({
    doc,
    selection: { anchor: at },
    extensions: [
      markdown({ base: markdownLanguage }),
      EditorState.allowMultipleSelections.of(true),
    ],
  });
const source =
  "# Header 🦀\n\n| first | second |\n| :--- | ---: |\n| `a\\|b` | العربية |\n\nTail";
describe("source preserving author adapters", () => {
  it("formats independent Unicode selections in one transaction", () => {
    let s = state("文 and 🦀");
    s = s.update({
      selection: EditorSelection.create([
        EditorSelection.range(0, 1),
        EditorSelection.range(6, 8),
      ]),
      effects: [],
    }).state;
    const next = s.update(format(s, "bold")).state;
    expect(next.doc.toString()).toBe("**文** and **🦀**");
    expect(
      next.selection.ranges.map((r) => next.doc.sliceString(r.from, r.to)),
    ).toEqual(["文", "🦀"]);
  });
  it.each([
    ["bold", "**", "strong_open"],
    ["italic", "*", "em_open"],
  ])(
    "applies %s inside edge spaces and preserves a backward selection",
    (kind, mark, token) => {
      const doc = "قبل  العربية 🦀  after";
      const from = doc.indexOf("  العربية");
      const to = doc.indexOf("after");
      let s = state(doc);
      s = s.update({ selection: EditorSelection.range(to, from) }).state;
      const next = s.update(format(s, kind)).state;
      const { anchor, head } = next.selection.main;
      expect(next.doc.toString()).toBe(`قبل  ${mark}العربية 🦀${mark}  after`);
      expect(anchor).toBeGreaterThan(head);
      expect(next.doc.sliceString(head, anchor)).toBe("العربية 🦀");
      const inline = grammar().parseInline(next.doc.toString(), {
        ...emptyContext(),
      });
      expect(inline[0].children?.map((part) => part.type)).toContain(token);
    },
  );
  it("preserves nested indentation and refuses fenced text", () => {
    const s = state("  - words", 4);
    expect(s.update(format(s, "ordered")).state.doc.toString()).toBe(
      "  1. words",
    );
    expect(() =>
      format(state("```md\n# literal\n```", 9), "heading1"),
    ).toThrow();
  });
  it("uses grammar-verified escaped pipes/code and preserves surrounding spans", () => {
    const s = state(source);
    const at = source.indexOf("| first");
    const patch = tableCell(s, at, 2, 1, "文 | next");
    expect(s.update({ changes: patch }).state.doc.toString()).toBe(
      source.replace("العربية", "文 \\| next"),
    );
    expect(tableAt(s, at).cols).toBe(2);
  });
  it("adds and deletes columns without changing siblings or invalidating delimiters", () => {
    let s = state(source);
    const at = source.indexOf("| first");
    s = s.update({ changes: tableShape(s, at, "column-add", 0) }).state;
    expect(tableAt(s, at).cols).toBe(3);
    s = s.update({ changes: tableShape(s, at, "column-delete", 2) }).state;
    expect(s.doc.toString()).toBe(source);
  });
  it("groups row and alignment patches inside only the table span", () => {
    const s = state(source),
      at = source.indexOf("| first");
    const p = tableShape(s, at, "center", 1);
    expect(s.doc.sliceString(p.from, p.to)).toBe("---:");
    expect(s.update({ changes: p }).state.doc.toString()).toContain(
      "| :--- | :---: |",
    );
    expect(
      tableAt(s.update({ changes: tableShape(s, at, "row-add", 2) }).state, at)
        .rows.length,
    ).toBe(4);
  });
  it("refuses ambiguous/ragged/unescaped code-pipe tables", () => {
    for (const doc of [
      "| a | b |\n| --- | --- |\n| `a|b` | c |",
      "| a | b |\n| --- | --- |\n| c |",
    ])
      expect(() => tableAt(state(doc), 0)).toThrow();
  });
  it("preserves even-backslash pipe semantics in replacements", () => {
    const s = state(source),
      at = source.indexOf("| first");
    const next = s.update({ changes: tableCell(s, at, 2, 1, "a\\\\|b") }).state;
    expect(tableAt(next, at).cols).toBe(2);
  });
  it("writes image attributes only when opted in and rejects authority-bearing URI input", () => {
    expect(imageText("assets/文 crab.png", "a]b", "caption")).toBe(
      '![a\\]b](assets/文%20crab.png "caption")',
    );
    expect(
      imageText("a.png", "a", "", 200, 100).endsWith("{width=200 height=100}"),
    ).toBe(true);
    expect(imageText("assets/photo (1).png", "", "")).toContain(
      "photo%20%281%29.png",
    );
    expect(() => imageText("https://bad/a.png", "", "")).toThrow();
    expect(() => imageText("a.png", "", "", 0, 100)).toThrow();
  });
  it("targets the chosen image, escaped titles and opt-in dimensions without guessing reference syntax", () => {
    const line = String.raw`![one](a.png) ![two](b.png "say \"hello\""){height=120}`;
    const parsed = imageSpan(state(line), line.indexOf("![two]"));
    expect(parsed?.path).toBe("b.png");
    expect(parsed?.height).toBe(120);
    expect(imageSpan(state("![one][ref]"), 0)).toBeNull();
  });
  it.each([
    String.raw`\![literal](keep.png)`,
    "`![literal](keep.png)`",
    "~~~md\n![literal](keep.png)\n~~~",
  ])("does not offer image edits for %s", (doc) =>
    expect(imageSpan(state(doc), doc.indexOf("keep.png"))).toBeNull(),
  );
  it("resolves the caret between adjacent images to the following image", () => {
    const doc = "Previous line\n![one](a.png)![two](b.png){height=120}";
    const from = doc.indexOf("![two]");
    expect(imageSpan(state(doc), from)).toMatchObject({
      path: "b.png",
      from,
      to: doc.length,
      height: 120,
    });
  });
  it("patches rich bodies without rewriting math delimiters or Mermaid fences", () => {
    for (const doc of [
      "Math $$a^2$$.",
      "~~~~mermaid\ngraph TD\nA --> B\n~~~~",
    ]) {
      const kind = doc.startsWith("Math") ? "math" : "mermaid";
      const at = doc.indexOf(kind === "math" ? "a^2" : "graph TD");
      const s = state(doc, at);
      const span = richSpan(s, kind, at)!;
      expect(
        s
          .update({ changes: { ...span, insert: "changed" } })
          .state.doc.toString(),
      ).toBe(
        kind === "math" ? "Math $$changed$$." : "~~~~mermaid\nchanged\n~~~~",
      );
    }
  });
  it("refuses rich controls inside unrelated code instead of guessing a fence", () => {
    for (const doc of ["```text\n$literal$\n```", "Some `code $literal$`."]) {
      const at = doc.indexOf("literal");
      expect(() => richSpan(state(doc, at), "math", at)).toThrow();
      expect(() => richSpan(state(doc, at), "mermaid", at)).toThrow();
    }
  });
});
