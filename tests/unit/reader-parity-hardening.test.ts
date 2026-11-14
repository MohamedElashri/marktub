import { expect, it } from "vitest";
import { input, compare } from "../support/reader-parity";
import { buildIndex } from "../../src/reader/indexer";
import { noteId } from "../../src/reader/grammar";

it("matches whole parsing across adversarial block pairs and partial terminal lines", async () => {
  const blocks = [
    "paragraph\ncontinuation\n",
    "- first\n\n  continuation\n- next\n",
    "> quote\n>\n> - nested\n",
    "heading\n---\n",
    "```md\n# literal\n```\n",
    '[r]: later.md\n  "continued title"\n',
    "[use][r] and note[^a].\n",
    "[^a]: note\n    continued\n",
    "| a | b |\n|---|---|\n| x | y |\n",
    "$$\nx+y\n$$\n",
  ];
  for (const first of blocks)
    for (const second of blocks)
      for (const width of [5, 11, 31])
        await compare(first + "\n" + second + "\n# Final\n", width);
});

it("preserves whole semantics at every CRLF split, with BOM and Unicode", async () => {
  const text =
    "\uFEFF# 文\r\n\r\nparagraph\r\ncontinued\r\n\r\n- one\r\n- two\r\n\r\n# End\r\n";
  for (let width = 5; width < 40; width++) await compare(text, width);
});

it("compares rich source and forward definitions across actual section boundaries", async () => {
  const text =
    "# First\n\n$x$ and [later][r].\n\n" +
    "padding paragraph\n\n".repeat(300) +
    "# Last\n\n$$y$$\n\n[r]: next.md\n";
  for (const width of [19, 257, 65536]) await compare(text, width);
});

it("retains a long completed line without backtracking or an arbitrary rich cutoff", async () => {
  const text =
    "x".repeat(131000) + "\n\n# After\n\n" + "padding\n\n".repeat(200);
  await compare(text, 65536);
});

it("allocates distinct ordered anchors when literal headings collide with duplicate suffixes", async () => {
  const text = "# Same\n\n# Same\n\n# Same-1\n\n# Same\n";
  for (const width of [7, 65536]) {
    const idx = await compare(text, width);
    expect(idx.headings.map((h) => h.id)).toEqual([
      "h-same",
      "h-same-1",
      "h-same-1-1",
      "h-same-2",
    ]);
  }
});

it("finalizes unique heading and footnote anchors even for a late footnote definition", async () => {
  const text =
    "# Note-61\n\nfootnote[^a]\n\n" +
    "padding\n\n".repeat(700) +
    "[^a]: final note\n";
  for (const width of [19, 65536]) {
    const idx = await compare(text, width);
    expect(new Set(idx.headings.map((h) => h.id)).size).toBe(
      idx.headings.length,
    );
    expect(idx.headings.find((h) => h.title === "Note-61")?.id).toBe(
      "h-note-61-1",
    );
    expect(idx.headings.find((h) => idx.sections[h.section].note)?.id).toBe(
      noteId("a"),
    );
  }
});

it("does not finalize or export when the footnote appendix exceeds the index budget", async () => {
  const text = "# heading\n\n".repeat(100000) + "[^a]: note\n";
  let complete = false;
  await expect(
    buildIndex(input(text, 65536), (idx) => {
      complete ||= idx.complete;
    }),
  ).rejects.toThrow("Footnote appendix");
  expect(complete).toBe(false);
});
