// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import {
  authorMathSpans,
  mathPlanForText,
} from "../../src/reader/math-context";
import { arabicOpening, longArabicDocument } from "../fixtures/arabic.mjs";
import { buildIndex, normalize, renderText } from "../../src/reader/indexer";
import { emptyContext, grammar } from "../../src/reader/grammar";
import { sanitizeReader } from "../../src/reader/safe";
import { sanitizePreview } from "../../src/reader/sanitize";
import { readingDirections } from "../../src/reader/direction";
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
      expect(link.hasAttribute("dir")).toBe(false);
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
      "- [x] مهمة عربية مكتملة\n- [ ] مهمة عربية قادمة\n\n[x] يبقى نصًا عاديًا.\n\n```text\n- [x] شيفرة حرفية\n```\n\n- # [x] عنوان حرفي",
      emptyContext(),
    ).html;
    const host = document.createElement("div");
    host.innerHTML = sanitizeReader(html);
    expect(
      [...host.querySelectorAll("li")].map((n) => n.textContent?.trim()),
    ).toEqual(["☑ مهمة عربية مكتملة", "☐ مهمة عربية قادمة", "[x] عنوان حرفي"]);
    expect(host.querySelector("h1")?.textContent).toBe("[x] عنوان حرفي");
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
    const blocks = [...host.querySelectorAll("h1,p")];
    expect(blocks).toHaveLength(3);
    for (const block of blocks) expect(block.getAttribute("dir")).toBe("auto");
  });
});

it("renders callout labels once and preserves literal markers outside the first quote paragraph", () => {
  const text =
    "> [!NOTE] هذه ملاحظة عربية $x+1$.\n\n> # [!TIP] عنوان حرفي\n\n[!NOTE]: note.md\n";
  const result = renderText(text, emptyContext());
  const host = document.createElement("div");
  host.innerHTML = sanitizeReader(result.html);
  expect(host.querySelectorAll(".callout")).toHaveLength(1);
  expect(host.querySelector(".callout p")?.textContent).not.toContain(
    "[!NOTE]",
  );
  expect(host.querySelector(".callout > strong")?.getAttribute("dir")).toBe(
    "ltr",
  );
  expect(host.querySelector("blockquote h1")?.textContent).toBe(
    "[!TIP] عنوان حرفي",
  );
});

it("preserves the closed Markdown table alignments through sanitization", () => {
  const result = renderText(
    "| عربي | English | رقم |\n| :--- | :---: | ---: |\n| أ | B | ٣ |",
    emptyContext(),
  );
  const host = document.createElement("div");
  host.innerHTML = sanitizeReader(result.html);
  expect(
    [...host.querySelectorAll("th")].map((n) => n.getAttribute("align")),
  ).toEqual(["left", "center", "right"]);
  expect(
    [...host.querySelectorAll("td")].map((n) => n.getAttribute("align")),
  ).toEqual(["left", "center", "right"]);
  expect(host.querySelector("[style]")).toBeNull();
});

it("retains exact original math spans when consuming a callout label", () => {
  const text = "> [!NOTE] نص عربي مع $x+1$ هنا.\n";
  const spans = authorMathSpans(text, mathPlanForText(text));
  expect(spans).toHaveLength(1);
  expect(text.slice(spans[0].from, spans[0].to)).toBe("$x+1$");
});

it("normalizes directions idempotently without changing text or source maps", () => {
  const html = renderText(arabicOpening, emptyContext(), [], true, true).html;
  const once = sanitizeReader(html);
  expect(sanitizeReader(once)).toBe(once);
});

it.each(["missing", "misresolved"])(
  "keeps linked Arabic headings automatic when detached :dir is %s",
  (selector) => {
    // A fresh template document exercises detection independently of prior pages.
    const template = document.implementation
      .createHTMLDocument()
      .createElement("template");
    template.innerHTML =
      '<h1><a href="note.md">العربية</a></h1><p><code>x + 1</code> نص عربي.</p>';
    const matches = Element.prototype.matches;
    const spy = vi
      .spyOn(Element.prototype, "matches")
      .mockImplementation(function (this: Element, query: string) {
        if (query.startsWith(":dir(")) {
          if (selector === "missing")
            throw new DOMException("Unsupported selector", "SyntaxError");
          return false;
        }
        return matches.call(this, query);
      });
    try {
      readingDirections(template.content);
      expect(spy).toHaveBeenCalledWith(":dir(rtl)");
      expect(template.content.querySelector("h1")?.getAttribute("dir")).toBe(
        "auto",
      );
      expect(template.content.querySelector("a")?.hasAttribute("dir")).toBe(
        false,
      );
      expect(template.content.querySelector("code")?.getAttribute("dir")).toBe(
        "ltr",
      );
      expect(template.content.textContent).toBe("العربيةx + 1 نص عربي.");
    } finally {
      spy.mockRestore();
    }
  },
);

it("preserves automatic direction in literal source fallbacks and LTR in code fences", () => {
  const host = document.createElement("div");
  host.innerHTML = sanitizeReader(
    "<pre>English\nالعربية</pre><pre><code>العربية English</code></pre>",
  );
  const [source, code] = host.querySelectorAll("pre");
  expect(source.getAttribute("dir")).toBe("auto");
  expect(source.textContent).toBe("English\nالعربية");
  expect(code.getAttribute("dir")).toBe("ltr");
});
