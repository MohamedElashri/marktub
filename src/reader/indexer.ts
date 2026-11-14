import { grammar, emptyContext, noteId } from "./grammar";
import type { Environment } from "./grammar";
import type { Index, Read, Section, Heading, Context } from "./model";
import { appendMath, emptyMathPlan, mathExpressions } from "./math-context";
const md = grammar();
const encoder = new TextEncoder();
export const MAX_BLOCK = 256 * 1024;
export const SECTION_TARGET = 4096;
function envClone(context: Context): Environment {
  return { ...structuredClone(context) };
}
function lines(text: string) {
  const offsets = [0];
  for (let i = 0; i < text.length; i++)
    if (text[i] === "\n") offsets.push(i + 1);
  return offsets;
}
export function normalize(text: string, beginning = true) {
  return (beginning ? text.replace(/^\uFEFF/, "") : text).replace(
    /\r\n?/g,
    "\n",
  );
}
// Work on raw UTF-8 spans. Markdown-it normalizes terminators internally; offsets
// translate its logical line maps back through original byte lengths.
function rawLines(text: string) {
  const rows = text.match(/[^\r\n]*(?:\r\n|\r|\n|$)/g) ?? [];
  const offsets = [0];
  for (const row of rows) if (row) offsets.push(offsets.at(-1)! + row.length);
  return offsets;
}
function headingId(
  title: string,
  used: Set<string>,
  duplicates: Map<string, number>,
) {
  const slug =
    title
      .toLowerCase()
      .replace(/[^\p{L}\p{N}_ -]/gu, "")
      .trim()
      .replace(/\s+/g, "-")
      .slice(0, 120) || "section";
  let n = duplicates.get(slug) ?? 0;
  let id = `h-${slug}${n ? "-" + n : ""}`;
  while (used.has(id)) id = `h-${slug}-${++n}`;
  duplicates.set(slug, n + 1);
  used.add(id);
  return id;
}
export async function buildIndex(
  read: Read,
  publish: (index: Index) => void,
  cancelled = () => false,
): Promise<Index> {
  const index: Index = {
    sections: [],
    headings: [],
    context: emptyContext(),
    complete: false,
    scanned: 0,
    total: 0,
  };
  let pending = "",
    base = 0,
    cursor = 0;
  let overflow: { type: string; marker: string; size: number } | undefined;
  let group: Section | undefined;
  const duplicates = new Map<string, number>();
  const headingIds = new Set<string>();
  const math = emptyMathPlan();
  const emit = (text: string, start: number, fallback?: string) => {
    if (!fallback && encoder.encode(text).length > 16384) {
      const tokens = md.parse(normalize(text, start === 0), {
        ...emptyContext(),
        beginning: start === 0,
      });
      if (
        tokens.reduce((sum, t) => sum + 1 + (t.children?.length ?? 0), 0) > 1024
      ) {
        emit(
          text,
          start,
          "Complex block exceeds the 1,024-token display budget; source view.",
        );
        return;
      }
    }

    if (fallback && text.length > 4096) {
      let offset = 0,
        byte = start;
      while (offset < text.length) {
        let end = Math.min(text.length, offset + 4096);
        if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1])) end--;
        const part = text.slice(offset, end);
        emit(part, byte, fallback);
        byte += encoder.encode(part).length;
        offset = end;
      }
      return;
    }
    const end = start + encoder.encode(text).length;
    if (!text) return;
    if (
      group &&
      !fallback &&
      !group.fallback &&
      end - group.start <= SECTION_TARGET
    ) {
      group.end = end;
    } else {
      group = { start, end, ...(fallback ? { fallback } : {}) };
      index.sections.push(group);
    }
    if (index.sections.length > 100000)
      throw new Error(
        "Reader index capacity reached. Already indexed sections remain available.",
      );
    if (!fallback) {
      const env: Environment = { ...emptyContext() };
      env.beginning = start === 0;
      const tokens = md.parse(normalize(text, start === 0), env);
      const formulas = mathExpressions(tokens);
      if (formulas.length) {
        group.mathStart ??= math.expressions.length;
        appendMath(math, formulas);
        index.math = math;
      }
      for (const [key, value] of Object.entries(env.references ?? {}))
        if (!Object.hasOwn(index.context.references, key))
          index.context.references[key] = value;
      for (const [key, value] of Object.entries(env.notes))
        if (!Object.hasOwn(index.context.notes, key))
          index.context.notes[key] = value;
      for (let i = 0; i < tokens.length; i++)
        if (tokens[i].type === "heading_open") {
          const title = tokens[i + 1]?.content ?? "";
          if (index.headings.length >= 100000)
            throw Error(
              "Heading index exceeds 100,000 entries. Indexed sections remain available; export is disabled.",
            );
          index.headings.push({
            title,
            id: headingId(title, headingIds, duplicates),
            section: index.sections.length - 1,
            level: Number(tokens[i].tag.slice(1)),
          });
        }
    }
  };
  function checkContext() {
    if (
      Object.keys(index.context.references).length > 10000 ||
      Object.keys(index.context.notes).length > 1024 ||
      JSON.stringify(index.context).length > 4 * 1024 * 1024
    )
      throw Error(
        "Document context exceeds the 4 MiB / 10,000-reference / 1,024-footnote budget. Indexed sections remain available; export is disabled.",
      );
  }
  const finish = () => {
    checkContext();
    const notes = Object.keys(index.context.notes);
    if (
      index.sections.length + notes.length > 100000 ||
      index.headings.length + notes.length > 100000
    )
      throw Error(
        "Footnote appendix exceeds the 100,000-section/heading index budget. Indexed sections remain available; export is disabled.",
      );
    // Late footnotes can reserve an ID used by a provisional heading. Finalize
    // all heading IDs in document order with those anchors already reserved.
    const reserved = new Set(notes.map(noteId));
    const finalDuplicates = new Map<string, number>();
    for (const heading of index.headings)
      heading.id = headingId(heading.title, reserved, finalDuplicates);
    for (const id of notes) {
      index.headings.push({
        title: `Footnote ${id}`,
        id: noteId(id),
        level: 3,
        section: index.sections.length,
      });
      index.sections.push({ start: index.total, end: index.total, note: id });
      const section = index.sections.at(-1)!;
      const formulas = mathExpressions(
        md.parse(
          `### Footnote ${id}\n\n${index.context.notes[id]}`,
          envClone(index.context),
        ),
      );
      if (formulas.length) {
        section.mathStart = math.expressions.length;
        appendMath(math, formulas);
        index.math = math;
      }
    }
    index.complete = true;
    publish(index);
    return index;
  };
  while (!cancelled()) {
    const page = await read(cursor);
    if (cancelled()) break;
    index.total = page.total;
    pending += page.text;
    cursor = page.end;
    index.scanned = cursor;
    const eof = cursor >= page.total;
    if (overflow) {
      const rows = rawLines(pending);
      let boundary = -1;
      for (let i = 0; i < rows.length - 1; i++) {
        const line = pending
          .slice(rows[i], rows[i + 1])
          .replace(/[\r\n]+$/, "");
        if (overflow.type === "fence") {
          const close = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(line);
          if (
            close &&
            close[1][0] === overflow.marker &&
            close[1].length >= overflow.size
          ) {
            boundary = rows[i + 1];
            break;
          }
        } else if (
          overflow.type === "math_block" &&
          /^\s*\$\$\s*$/.test(line)
        ) {
          boundary = rows[i + 1];
          break;
        } else if (overflow.type === "paragraph" && !line.trim()) {
          boundary = rows[i + 1];
          break;
        } else if (
          overflow.type === "frontmatter" &&
          /^(---|\.\.\.)\s*$/.test(line)
        ) {
          boundary = rows[i + 1];
          break;
        } else if (overflow.type !== "fence" && /^#{1,6} /.test(line)) {
          boundary = rows[i];
          break;
        }
      }
      if (boundary >= 0) {
        emit(
          pending.slice(0, boundary),
          base,
          "Oversized block continuation; source view.",
        );
        base += encoder.encode(pending.slice(0, boundary)).length;
        pending = pending.slice(boundary);
        overflow = undefined;
      } else {
        let end = eof ? pending.length : rows[Math.max(0, rows.length - 2)];
        if (end === 0 && pending.length > 65536) end = pending.length;
        emit(
          pending.slice(0, end),
          base,
          "Oversized block continuation; source view.",
        );
        base += encoder.encode(pending.slice(0, end)).length;
        pending = pending.slice(end);
        publish(index);
        if (eof) return finish();
        continue;
      }
    }
    // Block parsing carries the entire final top-level block, including nested
    // containers/fences/setext candidates. Inline context is resolved after scan.
    // A partial last line can look like a new block and then turn into a
    // reference continuation/title, list or Setext marker in the next read.
    // Do not use it as evidence that the preceding block is complete. Retain a
    // terminal CR too, since its LF may arrive in the next native range.
    let parseEnd = pending.length;
    if (!eof) {
      const from = pending.length - (pending.endsWith("\r") ? 2 : 1);
      parseEnd =
        from < 0
          ? 0
          : Math.max(
              pending.lastIndexOf("\r", from),
              pending.lastIndexOf("\n", from),
            ) + 1;
    }
    const logical = normalize(pending.slice(0, parseEnd), base === 0);
    const env: Environment = { ...emptyContext() };
    env.beginning = base === 0;
    const tokens = md.parse(logical, env);
    const starts = [
      ...new Set(
        tokens
          .filter((t) => t.level === 0 && t.nesting !== -1 && t.map)
          .map((t) => t.map![0]),
      ),
    ];
    const offsets = rawLines(pending);
    let consumed = 0,
      consumedBytes = 0;
    const safe = eof ? offsets.length - 1 : (starts.at(-1) ?? 0);
    for (let j = 0; j < starts.length; j++) {
      const from = starts[j],
        to = starts[j + 1] ?? safe;
      if (to <= from || from >= safe) break;
      const end = offsets[to] ?? pending.length;
      const text = pending.slice(consumed, end);
      emit(text, base + consumedBytes);
      consumedBytes += encoder.encode(text).length;
      consumed = end;
    }
    if (eof) {
      if (consumed < pending.length)
        emit(pending.slice(consumed), base + consumedBytes);
      pending = "";
    } else if (consumed) {
      base += consumedBytes;
      pending = pending.slice(consumed);
    }
    if (!eof && encoder.encode(pending).length > MAX_BLOCK) {
      const first = md.parse(
        normalize(pending, base === 0),
        envClone(index.context),
      )[0];
      const marker = /^ {0,3}(`{3,}|~{3,})/.exec(pending);
      overflow = {
        type: first?.type ?? "paragraph",
        marker: marker?.[1][0] ?? "",
        size: marker?.[1].length ?? 0,
      };
      const rows = rawLines(pending);
      const end = rows[Math.max(0, rows.length - 2)];
      emit(
        pending.slice(0, end),
        base,
        "Oversized block; source view for this span.",
      );
      base += encoder.encode(pending.slice(0, end)).length;
      pending = pending.slice(end);
    }
    if (eof) {
      return finish();
    }
    checkContext();
    publish(index);
    if (page.end <= page.start) throw new Error("Reader did not advance.");
  }
  return index;
}
export function renderText(
  text: string,
  context: Context,
  headings: Heading[] = [],
  beginning = true,
  sourceMap = false,
) {
  const env: Environment = {
    ...structuredClone(context),
    rich: [],
    beginning,
    heading: () => headings.shift()?.id ?? "",
  };
  const normalized = normalize(text, beginning);
  const tokens = md.parse(normalized, env);
  for (const token of tokens)
    if (sourceMap && token.map && token.nesting !== -1)
      token.attrSet("data-source-line", String(token.map[0]));
  const html = md.renderer.render(tokens, md.options, env);
  return { html, rich: env.rich ?? [] };
}
export { lines };

export async function findDocument(
  read: Read,
  index: Index,
  query: string,
  cancelled = () => false,
) {
  const matches: { section: number; excerpt: string }[] = [];
  if (!query || query.length > 200) return matches;
  const pattern = new RegExp(
    query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    "giu",
  );
  const seen = new Set<number>();
  let cursor = 0,
    tail = "";
  const limit = index.scanned;
  while (cursor < limit && !cancelled()) {
    const page = await read(cursor);
    if (page.end <= cursor) throw Error("Search did not advance.");
    const text = tail + page.text;
    pattern.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(text))) {
      const byte =
        page.start -
        encoder.encode(tail).length +
        encoder.encode(text.slice(0, match.index)).length;
      let lo = 0,
        hi = index.sections.length;
      while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (index.sections[mid].end <= byte) lo = mid + 1;
        else hi = mid;
      }
      if (lo < index.sections.length && !seen.has(lo)) {
        seen.add(lo);
        matches.push({
          section: lo,
          excerpt: text.slice(Math.max(0, match.index - 30), match.index + 100),
        });
      }
      if (matches.length >= 200) return matches;
    }
    tail = query.length > 1 ? text.slice(-(query.length - 1)) : "";
    if (tail && /[\uDC00-\uDFFF]/.test(tail[0])) tail = tail.slice(1);
    cursor = page.end;
  }
  return matches;
}
