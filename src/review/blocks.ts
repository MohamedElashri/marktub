import { grammar, emptyContext } from "../reader/grammar";
export interface Block {
  start: number;
  end: number;
  line: number;
  kind: string;
  quote: string;
  headingStart: number | null;
  headingEnd: number | null;
  heading: string;
}
const md = grammar();
export function blocks(source: string): Block[] {
  const lines = [0],
    units = [0];
  const encoder = new TextEncoder();
  let byte = 0,
    from = 0;
  for (const m of source.matchAll(/\r\n|\r|\n/g)) {
    const end = m.index! + m[0].length;
    byte += encoder.encode(source.slice(from, end)).length;
    lines.push(byte);
    units.push(end);
    from = end;
  }
  const size = encoder.encode(source).length;
  const tokens = md.parse(source.replace(/^\uFEFF/, ""), {
    ...emptyContext(),
    beginning: true,
  });
  const out: Block[] = [];
  let hs: number | null = null,
    he: number | null = null,
    heading = "Document";
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (t.level !== 0 || t.nesting === -1 || !t.map) continue;
    const [a, b] = t.map;
    const start = lines[a] ?? size,
      end = lines[b] ?? size;
    if (start >= end || out.at(-1)?.start === start) continue;
    const isHeading = t.type === "heading_open";
    out.push({
      start,
      end,
      line: a + 1,
      kind: t.type.replace(/_open$/, ""),
      quote: source.slice(units[a] ?? source.length, units[b] ?? source.length),
      headingStart: hs,
      headingEnd: he,
      heading,
    });
    if (isHeading) {
      hs = start;
      he = end;
      heading = tokens[i + 1]?.content ?? "Heading";
    }
    if (out.length > 4000)
      throw Error(
        "Review admits at most 4,000 top-level blocks. Saved source comparisons remain available.",
      );
  }
  return out;
}
export interface Change {
  kind: "added" | "removed" | "moved";
  block: Block;
}
// Hirschberg LCS: exact block ordering with linear working memory. Matching
// identical display blocks never assigns stable identity to comment anchors.
function common(a: string[], b: string[]): [number, number][] {
  const result: [number, number][] = [];
  const lengths = (x: string[], y: string[]) => {
    let previous = new Uint16Array(y.length + 1),
      next = new Uint16Array(y.length + 1);
    for (const item of x) {
      for (let j = 0; j < y.length; j++)
        next[j + 1] =
          item === y[j] ? previous[j] + 1 : Math.max(previous[j + 1], next[j]);
      [previous, next] = [next, previous];
      next.fill(0);
    }
    return previous;
  };
  const walk = (x: string[], y: string[], ax: number, by: number) => {
    while (x.length && y.length && x[0] === y[0]) {
      result.push([ax++, by++]);
      x = x.slice(1);
      y = y.slice(1);
    }
    if (!x.length || !y.length) return;
    if (x.length === 1) {
      const n = y.indexOf(x[0]);
      if (n >= 0) result.push([ax, by + n]);
      return;
    }
    const mid = Math.floor(x.length / 2),
      left = lengths(x.slice(0, mid), y),
      right = lengths(x.slice(mid).reverse(), [...y].reverse());
    let split = 0,
      best = -1;
    for (let i = 0; i <= y.length; i++) {
      const n = left[i] + right[y.length - i];
      if (n > best) {
        best = n;
        split = i;
      }
    }
    walk(x.slice(0, mid), y.slice(0, split), ax, by);
    walk(x.slice(mid), y.slice(split), ax + mid, by + split);
  };
  walk(a, b, 0, 0);
  return result;
}
export function changedBlocks(left: string, right: string): Change[] {
  if (left === right) return [];
  const a = blocks(left),
    b = blocks(right),
    key = (v: Block) => v.kind + "\0" + v.quote;
  const pairs = common(a.map(key), b.map(key)),
    old = new Set(pairs.map((p) => p[0])),
    next = new Set(pairs.map((p) => p[1]));
  return [
    ...a
      .filter((_, i) => !old.has(i))
      .map((block) => ({ kind: "removed" as const, block })),
    ...b
      .filter((_, i) => !next.has(i))
      .map((block) => ({ kind: "added" as const, block })),
  ];
}
