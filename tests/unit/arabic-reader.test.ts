// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { arabicOpening, longArabicDocument } from "../fixtures/arabic.mjs";
import { buildIndex, normalize, renderText } from "../../src/reader/indexer";
import { emptyContext, grammar } from "../../src/reader/grammar";
import { sanitizeReader } from "../../src/reader/safe";
import { sanitizePreview } from "../../src/reader/sanitize";
import { renderSection } from "../../src/reader/render";
import type { Read } from "../../src/reader/model";

function nativeChunks(text: string, width: number): Read {
  const bytes = new TextEncoder().encode(text);
  return async (start) => {
    let end = Math.min(bytes.length, start + width);
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

describe("Arabic reading documents", () => {
  for (const [label, text, width] of [
    ["mixed Arabic blocks", arabicOpening, 97],
    ["long Arabic document", longArabicDocument, 4093],
  ] as const)
    it(`preserves whole-input parsing across UTF-8 pages: ${label}`, async () => {
      const index = await buildIndex(nativeChunks(text, width), () => {});
      expect(index.complete).toBe(true);
      expect(index.sections.every((s) => !s.fallback)).toBe(true);
      const bytes = new TextEncoder().encode(text);
      expect(index.sections.at(-1)?.end).toBe(bytes.length);
      const paged = index.sections
        .map(
          (s) =>
            renderText(
              new TextDecoder("utf-8", { ignoreBOM: true }).decode(
                bytes.subarray(s.start, s.end),
              ),
              index.context,
              [],
              s.start === 0,
            ).html,
        )
        .join("");
      const md = grammar();
      const context = { ...emptyContext(), beginning: true };
      md.parse(normalize(text), context);
      expect(paged).toBe(md.render(normalize(text), context));
    });

  it("preserves text and maps while isolating code, links and individual prose blocks", () => {
    const html = renderText(arabicOpening, emptyContext(), [], true, true).html;
    const original = document.createElement("div");
    original.innerHTML = html;
    const host = document.createElement("div");
    host.innerHTML = sanitizeReader(html);
    expect(host.textContent).toBe(original.textContent);
    for (const block of host.querySelectorAll("p, h1, li, th, td"))
      expect(block.getAttribute("dir")).toBe("auto");
    for (const code of host.querySelectorAll("code, pre:has(code)"))
      expect(code.getAttribute("dir")).toBe("ltr");
    for (const link of host.querySelectorAll("a"))
      expect(link.getAttribute("dir")).toBe("auto");
    expect(
      [...host.querySelectorAll("[data-source-line]")].map((n) =>
        n.getAttribute("data-source-line"),
      ),
    ).toEqual(
      [...original.querySelectorAll("[data-source-line]")].map((n) =>
        n.getAttribute("data-source-line"),
      ),
    );
    expect(host.querySelector("script,style,iframe,img,input")).toBeNull();
    expect(host.textContent).toContain("☑ مهمة عربية مكتملة");
  });

  it("renders checked and unchecked task text without interpreting ordinary prose or code as tasks", () => {
    const html = renderText(
      "- [x] مهمة عربية مكتملة\n- [ ] مهمة عربية قادمة\n\n[x] يبقى نصًا عاديًا.\n\n```text\n- [x] شيفرة حرفية\n```",
      emptyContext(),
    ).html;
    const host = document.createElement("div");
    host.innerHTML = sanitizeReader(html);
    expect([...host.querySelectorAll("li")].map((n) => n.textContent)).toEqual([
      "☑ مهمة عربية مكتملة",
      "☐ مهمة عربية قادمة",
    ]);
    expect(host.querySelector("p")?.textContent).toBe("[x] يبقى نصًا عاديًا.");
    expect(host.querySelector("code")?.textContent).toContain(
      "- [x] شيفرة حرفية",
    );
    expect(host.querySelector("input")).toBeNull();
  });

  it("also isolates Arabic blocks in the bounded opening preview", () => {
    const host = document.createElement("div");
    host.innerHTML = sanitizePreview(
      renderSection(
        "# العربية\n\nهذه فقرة طويلة مع English و١٢٣.\n\nEnglish العربية.",
      ).html,
    );
    expect(
      [...host.querySelectorAll("h1,p")].map((n) => n.getAttribute("dir")),
    ).toEqual(["auto", "auto", "auto"]);
  });
});
