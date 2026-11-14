import { expect } from "vitest";
import { buildIndex, normalize, renderText } from "../../src/reader/indexer";
import { emptyContext, grammar } from "../../src/reader/grammar";
import type { Environment } from "../../src/reader/grammar";
import type { Read, Rich } from "../../src/reader/model";

export function input(text: string, width: number): Read {
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
// Rich placeholder numbers are local to each render call. Compare their actual
// source/kind/display semantics, including expressions on unmounted pages.
function semanticHtml(html: string, rich: Rich[]) {
  return html.replace(
    /data-rich="(\d+)"/g,
    (_match, n) =>
      `data-rich="${encodeURIComponent(JSON.stringify(rich[Number(n)]))}"`,
  );
}
export async function compare(text: string, width: number) {
  const idx = await buildIndex(input(text, width), () => {});
  expect(idx.complete).toBe(true);
  expect(idx.sections.some((s) => s.fallback)).toBe(false);
  const md = grammar();
  const env: Environment = { ...emptyContext(), beginning: true, rich: [] };
  const tokens = md.parse(normalize(text), env);
  const wholeHeadings = tokens.flatMap((token, i) =>
    token.type === "heading_open"
      ? [{ title: tokens[i + 1].content, level: Number(token.tag.slice(1)) }]
      : [],
  );
  const headings = idx.headings.filter((h) => !idx.sections[h.section]?.note);
  expect(headings.map(({ title, level }) => ({ title, level }))).toEqual(
    wholeHeadings,
  );
  expect(idx.context, `chunk ${width}: ${JSON.stringify(text)}`).toEqual({
    references: env.references,
    notes: env.notes,
  });
  let next = 0;
  env.heading = () => headings[next++].id;
  const whole = semanticHtml(
    md.renderer.render(tokens, md.options, env),
    env.rich!,
  );
  const bytes = new TextEncoder().encode(text);
  const paged = idx.sections
    .flatMap((section, i) => {
      if (section.note) return [];
      const rendered = renderText(
        new TextDecoder("utf-8", { ignoreBOM: true }).decode(
          bytes.subarray(section.start, section.end),
        ),
        idx.context,
        headings.filter((h) => h.section === i),
        section.start === 0,
      );
      return [semanticHtml(rendered.html, rendered.rich)];
    })
    .join("");
  expect(paged, `chunk ${width}: ${JSON.stringify(text)}`).toBe(whole);
  return idx;
}
