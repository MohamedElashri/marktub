import DOMPurify from "dompurify";
import { sanitizeReader, safeLink } from "../../src/reader/safe";

// Frozen previous reader pipeline for paired native preparation measurements.
// This module is imported only by the E2E entry; release checks reject its marker.
function previous(html: string) {
  const clean = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
      "script",
      "iframe",
      "object",
      "embed",
      "foreignObject",
      "form",
      "input",
      "video",
      "audio",
      "link",
      "style",
      "base",
      "image",
      "animate",
      "animateTransform",
      "animateMotion",
      "set",
      "feImage",
      "meta",
      "img",
    ],
    FORBID_ATTR: ["style", "name", "src", "srcset", "target"],
    ALLOW_DATA_ATTR: true,
    SANITIZE_NAMED_PROPS: false,
  });
  const host = document.createElement("template");
  host.innerHTML = clean;
  for (const el of host.content.querySelectorAll<HTMLElement>("*")) {
    if (el.id && !/^h-[\p{L}\p{N}_-]+$/u.test(el.id)) el.removeAttribute("id");
    const href = el.getAttribute("href");
    if (href && !safeLink(href)) el.removeAttribute("href");
    else if (href?.startsWith("#") && !href.startsWith("#h-"))
      el.setAttribute("href", "#h-" + href.slice(1));
  }
  for (const code of host.content.querySelectorAll(
    "code, pre:has(code), [data-rich]",
  ))
    code.setAttribute("dir", "ltr");
  for (const link of host.content.querySelectorAll("a"))
    link.setAttribute("dir", "auto");
  const groups = [
    ...host.content.querySelectorAll("blockquote, ul, ol, table"),
  ].map((group) => {
    group.setAttribute("dir", "auto");
    let direction = "auto";
    try {
      direction = group.matches(":dir(rtl)") ? "rtl" : "ltr";
    } catch {
      /* Previous compatibility path. */
    }
    group.removeAttribute("dir");
    return { group, direction };
  });
  for (const { group, direction } of groups)
    group.setAttribute("dir", direction);
  for (const block of host.content.querySelectorAll(
    "p, h1, h2, h3, h4, h5, h6, li, dt, dd, th, td, pre:not(:has(code))",
  ))
    block.setAttribute("dir", "auto");
  return host.innerHTML;
}

export function readerCost() {
  const html = `<blockquote><ul>${Array.from({ length: 64 }, (_, i) => `<li><p><a href="note.md">العربية ${i}</a> with English <code>x + 1</code></p></li>`).join("")}</ul></blockquote>`;
  const before: number[] = [],
    after: number[] = [];
  let previousOutput = "",
    currentOutput = "";
  const measure = (run: () => void, samples: number[], keep: boolean) => {
    const start = performance.now();
    run();
    if (keep) samples.push(performance.now() - start);
  };
  for (let i = 0; i < 25; i++) {
    const old = () =>
      measure(
        () => {
          previousOutput = previous(html);
        },
        before,
        i >= 5,
      );
    const current = () =>
      measure(
        () => {
          currentOutput = sanitizeReader(html);
        },
        after,
        i >= 5,
      );
    if (i % 2) {
      current();
      old();
    } else {
      old();
      current();
    }
  }
  const text = (html: string) => {
    const host = document.createElement("template");
    host.innerHTML = html;
    return host.content.textContent;
  };
  const summarize = (values: number[]) => {
    const sorted = [...values].sort((a, b) => a - b);
    return {
      samples: values,
      medianMs: sorted[Math.floor(sorted.length / 2)],
      p95Ms: sorted[Math.ceil(sorted.length * 0.95) - 1],
    };
  };
  return {
    scope:
      "Paired native WebView HTML preparation, 64 linked mixed-language items; not layout, startup, memory or physical paint",
    inputCharacters: html.length,
    textPreserved: text(previousOutput) === text(currentOutput),
    before: summarize(before),
    after: summarize(after),
  };
}

declare global {
  interface Window {
    MARKTUB_TEST_READER_COST: typeof readerCost;
  }
}
window.MARKTUB_TEST_READER_COST = readerCost;
