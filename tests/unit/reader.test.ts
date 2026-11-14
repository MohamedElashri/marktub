import { describe, expect, it } from "vitest";
import MarkdownIt from "markdown-it";
import { renderSection } from "../../src/reader/render";

describe("reader architecture probes", () => {
  it("proves naive page parsing loses forward link definitions", () => {
    const md = new MarkdownIt();
    const first = "Read [the note][later].\n\n";
    const second = "[later]: chapter.md\n";
    expect(md.render(first + second)).toContain('<a href="chapter.md">');
    expect(md.render(first) + md.render(second)).not.toBe(
      md.render(first + second),
    );
  });

  it("proves naive page parsing loses an open fence", () => {
    const md = new MarkdownIt();
    const first = "```md\n# not a heading\n";
    const second = "still code\n```\n";
    expect(md.render(first + second)).not.toBe(
      md.render(first) + md.render(second),
    );
  });

  it("bounds top-level rendered blocks and source input", () => {
    const result = renderSection("paragraph\n\n".repeat(200));
    expect(result.blocks).toBe(32);
    expect(result.clipped).toBe(true);
    expect(() => renderSection("x".repeat(65537))).toThrow("source window");
  });
});
