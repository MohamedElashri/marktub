import { describe, expect, it } from "vitest";
import { renderSection } from "../../src/reader/render";

describe("bounded library preview", () => {
  it("renders references and code from the preview with inert links and images", () => {
    const result = renderSection(
      "Read [the note][later].\n\n![Illustration](https://example.org/image.png)\n\n" +
        "[later]: chapter.md\n\n```md\n# Inside code\n```\n",
    );
    expect(result.html).toContain(
      '<span class="document-link">the note</span>',
    );
    expect(result.html).toContain("[Image: Illustration]");
    expect(result.html).toContain(
      '<pre><code class="language-md"># Inside code',
    );
    expect(result.html).not.toMatch(/<(a|img|h1)\b/);
    expect(result.clipped).toBe(false);
  });

  it("bounds top-level blocks and measures the input window in UTF-8 bytes", () => {
    const result = renderSection("paragraph\n\n".repeat(200));
    expect(result.blocks).toBe(32);
    expect(result.clipped).toBe(true);
    expect(() => renderSection("🦀".repeat(16_385))).toThrow("source window");
  });

  it("falls back to escaped text without splitting a Unicode scalar", () => {
    const text = "<script>" + "x".repeat(4087) + "🦀" + "x".repeat(500);
    const result = renderSection(text);
    expect(result.clipped).toBe(true);
    expect(result.blocks).toBe(1);
    expect(result.html).toBe(`<pre>&lt;script&gt;${"x".repeat(4087)}</pre>`);
  });
});
