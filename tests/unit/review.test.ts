import { describe, it, expect } from "vitest";
import { blocks, changedBlocks } from "../../src/review/blocks";
import { markdownReport } from "../../src/review/bridge";
describe("saved review block spans", () => {
  it("preserves UTF-8 BOM and mixed CRLF/CR byte provenance", () => {
    const text = "\uFEFF# 文\r\n\r\nHello 🦀.\r\n\rNext.\n";
    const data = new TextEncoder().encode(text);
    const result = blocks(text);
    expect(result.length).toBeGreaterThan(2);
    for (const b of result)
      expect(
        new TextDecoder("utf-8", { ignoreBOM: true }).decode(
          data.slice(b.start, b.end),
        ),
      ).toBe(b.quote);
    expect(result[1].headingStart).toBe(0);
    expect(result[1].headingEnd).toBe(10);
  });
  it("shows only changed blocks while preserving repeated unchanged paragraphs", () => {
    const old = "# A\n\nSame.\n\nSame.\n\nEnd.\n";
    expect(changedBlocks(old, old)).toEqual([]);
    const result = changedBlocks(old, old.replace("End.", "New."));
    expect(result.map((v) => v.block.quote.trim())).toEqual(["End.", "New."]);
    expect(result.map((v) => v.kind)).toEqual(["removed", "added"]);
  });
  it("keeps fenced content and nested lists as source-backed blocks", () => {
    const text =
      "# Plan\n\n- a\n  - b\n\n```js\n# fake heading\n<script>bad()</script>\n```\n";
    const result = blocks(text);
    expect(result.map((b) => b.kind)).toEqual([
      "heading",
      "bullet_list",
      "fence",
    ]);
    expect(result[2].heading).toBe("Plan");
    expect(result[2].quote).toContain("<script>");
  });
  it("matches block sequence with linear working memory", () => {
    const old = Array.from(
      { length: 500 },
      (_, i) => `Paragraph ${i}.\n\n`,
    ).join("");
    const newer = old.replace("Paragraph 250.", "Changed 250.");
    expect(changedBlocks(old, newer).map((c) => c.block.quote.trim())).toEqual([
      "Paragraph 250.",
      "Changed 250.",
    ]);
  });
  it("uses a portable fence longer than any untrusted comment fence", () => {
    const json = JSON.stringify({ text: "```\n</script>\n````" });
    const md = markdownReport(json);
    expect(md).toContain("`````json");
    expect(md).toContain(json);
  });
});
