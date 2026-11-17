import { describe, it, expect } from "vitest";
import {
  buildIndex,
  normalize,
  renderText,
  findDocument,
} from "../../src/reader/indexer";
import { grammar, emptyContext } from "../../src/reader/grammar";
import type { Read } from "../../src/reader/model";
function source(text: string, width: number): Read {
  const bytes = new TextEncoder().encode(text);
  return async (start) => {
    let end = Math.min(start + width, bytes.length);
    while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    return {
      start,
      end,
      total: bytes.length,
      text: new TextDecoder("utf-8", { ignoreBOM: true }).decode(
        bytes.subarray(start, end),
      ),
    };
  };
}
const corpus = [
  "p".repeat(5000) + "\n\n\uFEFF# Interior BOM is text\n\nAfter\n",
  'Read [later][ref].\n\n# A\n\n[ref]: chapter.md "later"\n',
  "# Same\n\npara\n\n# Same\n\nother\n",
  "Setext heading\n==============\n\n- outer\n  - nested\n\n    paragraph\n\n- second\n\nAfter.\n",
  "> quoted\n>\n> - item\n>\n> next\n\n```md\n# fenced\n\n[ref]: no\n```\n\nend\n",
  "| a | b |\n|---|---|\n| 文 | 🦀 |\n\n~~strike~~ and https://example.org\n",
  "A footnote[^long].\n\n# Next\n\n[^long]: a later definition\n    continued\n",
  "\uFEFF---\r\ntitle: unchanged\r\n---\r\n\r\n# 文\r\n\r\nArabic العربية\r\n",
  "- [x] task\n- [ ] next\n\n> [!NOTE]\n> note\n\n$$\na^2+b^2=c^2\n$$\n",
];
describe("context-aware reader parity", () => {
  for (const [fixture, text] of corpus.entries())
    for (const width of [19, 64, 257])
      it(`whole-input parity fixture ${fixture}, chunk ${width}`, async () => {
        const idx = await buildIndex(source(text, width), () => {});
        expect(idx.complete).toBe(true);
        const bytes = new TextEncoder().encode(text);
        const combined = idx.sections
          .filter((s) => !s.note)
          .map(
            (section) =>
              renderText(
                new TextDecoder("utf-8", { ignoreBOM: true }).decode(
                  bytes.subarray(section.start, section.end),
                ),
                idx.context,
                [],
                section.start === 0,
              ).html,
          )
          .join("");
        const md = grammar();
        const context = { ...emptyContext(), beginning: true };
        md.parse(normalize(text), context);
        const whole = md.render(normalize(text), context);
        expect(combined).toBe(whole);
        expect(idx.sections[0]?.start).toBe(0);
        expect(idx.sections.at(-1)?.end).toBe(bytes.length);
      });
  it("emits provisional sections before full scan and preserves revision ranges", async () => {
    let early = false;
    const text = "# Heading\n\nparagraph.\n\n".repeat(10000);
    const idx = await buildIndex(source(text, 65536), (i) => {
      early ||= !i.complete && i.sections.length > 0;
    });
    expect(early).toBe(true);
    expect(idx.headings[1].id).toBe("h-heading-1");
    expect(idx.sections.length).toBeGreaterThan(1);
  });
});

it("bounds giant fence source segments and resumes rich parsing after its close", async () => {
  const text =
    "```text\n" +
    "literal # text\n".repeat(24000) +
    "```\n\n# After giant fence\n\nA rendered paragraph.\n";
  const idx = await buildIndex(source(text, 65536), () => {});
  const bytes = new TextEncoder().encode(text);
  expect(idx.complete).toBe(true);
  expect(
    idx.sections
      .filter((s) => s.fallback)
      .every((s) => s.end - s.start <= 16384),
  ).toBe(true);
  expect(idx.headings.some((h) => h.title === "After giant fence")).toBe(true);
  expect(
    idx.sections.every((s, i) =>
      i === 0 ? s.start === 0 : s.start === idx.sections[i - 1].end,
    ),
  ).toBe(true);
  expect(idx.sections.at(-1)?.end).toBe(bytes.length);
});

it("keeps complex giant lists local instead of creating unlimited DOM tokens", async () => {
  const text = "- item\n".repeat(25000) + "\n# After\n\nend\n";
  const idx = await buildIndex(source(text, 65536), () => {});
  expect(idx.sections.some((s) => s.fallback)).toBe(true);
  expect(idx.headings.some((h) => h.title === "After")).toBe(true);
});
it("preserves cross-boundary semantics over seeded variable read widths", async () => {
  const text = corpus.join("\n\n");
  const bytes = new TextEncoder().encode(text);
  let seed = 42;
  const read: Read = async (start) => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    let end = Math.min(bytes.length, start + 16 + (seed % 127));
    while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    return {
      start,
      end,
      total: bytes.length,
      text: new TextDecoder("utf-8", { ignoreBOM: true }).decode(
        bytes.subarray(start, end),
      ),
    };
  };
  const idx = await buildIndex(read, () => {});
  const html = idx.sections
    .filter((s) => !s.note)
    .map(
      (s) =>
        renderText(
          new TextDecoder("utf-8", { ignoreBOM: true }).decode(
            bytes.subarray(s.start, s.end),
          ),
          idx.context,
          [],
          s.start === 0,
        ).html,
    )
    .join("");
  const md = grammar();
  const env = { ...emptyContext(), beginning: true };
  md.parse(normalize(text), env);
  expect(html).toBe(md.render(normalize(text), env));
});

it("finds literal Unicode across source fallback and read boundaries", async () => {
  const text = "x".repeat(4093) + "find文🦀here" + "y".repeat(4000);
  const read = source(text, 97);
  const idx = await buildIndex(read, () => {});
  expect((await findDocument(read, idx, "find文🦀here")).length).toBe(1);
  expect((await findDocument(read, idx, "[missing]")).length).toBe(0);
});

it("indexes a long definition-only tail with continuation titles without guessing a page boundary", async () => {
  const text =
    "[last][r1199]\n\n" +
    Array.from(
      { length: 1200 },
      (_, i) => `[r${i}]: ${"relative".repeat(40)}-${i}.md\n  "title ${i}"\n`,
    ).join("");
  const idx = await buildIndex(source(text, 65536), () => {});
  expect(idx.complete).toBe(true);
  expect(idx.context.references.R1199.title).toBe("title 1199");
  const md = grammar();
  const bytes = new TextEncoder().encode(text);
  const actual = idx.sections
    .map(
      (s) =>
        renderText(
          new TextDecoder().decode(bytes.subarray(s.start, s.end)),
          idx.context,
        ).html,
    )
    .join("");
  expect(actual).toBe(md.render(text, { ...emptyContext(), beginning: true }));
});

it("records native logical line boundaries across BOM, Unicode and mixed EOL chunks", async () => {
  let ending = 0;
  const text =
    "\uFEFF" +
    Array.from(
      { length: 100 },
      (_, i) =>
        `## Section ${i}\n\n${"العربية 日本語 🦀 text ".repeat(12)}\n\n`,
    )
      .join("")
      .replace(/\n/g, () => ["\r\n", "\n", "\r"][ending++ % 3]);
  const index = await buildIndex(source(text, 257), () => {});
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder("utf-8", { ignoreBOM: true });
  const lineAt = (byte: number) =>
    1 +
    (decoder.decode(bytes.subarray(0, byte)).match(/\r\n|\r|\n/g)?.length ?? 0);
  expect(index.sections.length).toBeGreaterThan(2);
  for (const section of index.sections.filter((s) => !s.note)) {
    expect(section.line).toBe(lineAt(section.start));
    expect(section.endLine).toBe(lineAt(section.end));
  }
});
