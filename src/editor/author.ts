import {
  EditorSelection,
  type EditorState,
  type ChangeSpec,
} from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { grammar, emptyContext } from "../reader/grammar";
export type Patch = { from: number; to: number; insert: string };
export function writableBlock(state: EditorState, at: number) {
  const tree = ensureSyntaxTree(state, at, 25);
  if (!tree) return false;
  let node = tree.resolveInner(at, 1);
  for (; node; node = node.parent!)
    if (
      [
        "FencedCode",
        "CodeBlock",
        "HTMLBlock",
        "InlineCode",
        "CodeText",
      ].includes(node.name)
    )
      return false;
  return true;
}
export function format(
  state: EditorState,
  kind: string,
): { changes: ChangeSpec; selection?: EditorSelection } {
  if (
    kind === "bold" ||
    kind === "italic" ||
    kind === "code" ||
    kind === "link"
  ) {
    const changes: Patch[] = [];
    let delta = 0;
    const ranges = state.selection.ranges.map((r) => {
      if (
        !writableBlock(state, r.from) ||
        state.doc.sliceString(r.from, r.to).includes("\n")
      )
        throw Error("Select one inline span outside a code/HTML block.");
      const selected = state.doc.sliceString(r.from, r.to);
      const emphasis = kind === "bold" || kind === "italic";
      const text = emphasis ? selected.trim() : selected;
      if (selected && !text) throw Error("Select text to apply emphasis.");
      const mark = kind === "bold" ? "**" : kind === "italic" ? "*" : "`";
      if (kind === "code" && text.includes("`"))
        throw Error("Backtick-containing code is source-only.");
      const left = kind === "link" ? "[" : mark,
        right = kind === "link" ? "](local-path)" : mark;
      const from = emphasis
        ? r.from + selected.length - selected.trimStart().length
        : r.from;
      const to = from + text.length;
      changes.push({ from, to, insert: left + text + right });
      const start = from + delta + left.length;
      const end = start + text.length;
      delta += left.length + right.length;
      return r.anchor > r.head
        ? EditorSelection.range(end, start)
        : EditorSelection.range(start, end);
    });
    return {
      changes,
      selection: EditorSelection.create(ranges, state.selection.mainIndex),
    };
  }
  const seen = new Set<number>(),
    changes: Patch[] = [];
  for (const r of state.selection.ranges)
    for (
      let p = state.doc.lineAt(r.from).number;
      p <= state.doc.lineAt(r.to).number;
      p++
    ) {
      if (seen.has(p)) continue;
      seen.add(p);
      const line = state.doc.line(p);
      if (!writableBlock(state, line.from + Math.min(line.length, 2)))
        throw Error("This block stays editable in Source.");
      const prefix =
        /^(\s*)(?:#{1,6} |[-+*] (?:\[[ xX]\] )?|\d+[.)] |> )?/.exec(
          line.text,
        )![0];
      const indent = /^\s*/.exec(line.text)![0];
      const marker = kind.startsWith("heading")
        ? "#".repeat(Number(kind.slice(-1))) + " "
        : kind === "quote"
          ? "> "
          : kind === "task"
            ? "- [ ] "
            : kind === "ordered"
              ? "1. "
              : "- ";
      changes.push({
        from: line.from,
        to: line.from + prefix.length,
        insert: indent + marker,
      });
    }
  return { changes };
}
export function cellSpans(line: string) {
  if (
    line.length > 8192 ||
    !line.trim().startsWith("|") ||
    !line.trim().endsWith("|")
  )
    throw Error(
      "Use an outer-pipe GFM table (maximum 8,192 characters per row).",
    );
  const pipes: number[] = [];
  let ticks = 0;
  for (let i = 0; i < line.length; i++) {
    if (line[i] === "\\") {
      i++;
      continue;
    }
    if (line[i] === "`") {
      let n = 1;
      while (line[i + n] === "`") n++;
      ticks = ticks === n ? 0 : ticks === 0 ? n : ticks;
      i += n - 1;
      continue;
    }
    if (line[i] === "|") {
      if (ticks) throw Error("Escape pipes inside table code spans as \\|.");
      pipes.push(i);
    }
  }
  if (ticks) throw Error("Unclosed table code span; edit it in Source.");
  return pipes.slice(0, -1).map((p, i) => {
    const raw = line.slice(p + 1, pipes[i + 1]);
    const pad = /^\s*/.exec(raw)![0].length;
    return {
      from: p + 1 + pad,
      to: p + 1 + pad + raw.trim().length,
      text: raw.trim(),
      rawFrom: p + 1,
      rawTo: pipes[i + 1],
    };
  });
}
export function tableAt(state: EditorState, at: number) {
  const doc = state.doc;
  let first = doc.lineAt(at).number,
    last = first;
  while (
    first > 1 &&
    doc
      .line(first - 1)
      .text.trim()
      .startsWith("|")
  )
    first--;
  while (
    last < doc.lines &&
    doc
      .line(last + 1)
      .text.trim()
      .startsWith("|")
  )
    last++;
  if (
    last - first < 1 ||
    last - first > 100 ||
    !writableBlock(state, doc.line(first).from + 1)
  )
    throw Error(
      "Choose a GFM table outside a code block (up to 101 total rows).",
    );
  const lines = Array.from({ length: last - first + 1 }, (_, i) =>
    doc.line(first + i),
  );
  const rows = lines.map((l) => cellSpans(l.text));
  const cols = rows[0].length;
  if (
    cols < 1 ||
    cols > 16 ||
    rows.some((r) => r.length !== cols) ||
    !rows[1].every((c) => /^:?-{3,}:?$/.test(c.text))
  )
    throw Error("Ambiguous table shape/alignment; edit it in Source.");
  const parsed = grammar().parse(lines.map((l) => l.text).join("\n"), {
    ...emptyContext(),
    beginning: false,
  });
  if (
    parsed[0]?.type !== "table_open" ||
    parsed.filter((t) => t.type === "th_open").length !== cols
  )
    throw Error("The display parser does not agree with this table span.");
  return { from: lines[0].from, to: lines.at(-1)!.to, lines, rows, cols };
}
export function tableCell(
  state: EditorState,
  at: number,
  row: number,
  col: number,
  text: string,
): Patch {
  const t = tableAt(state, at);
  const c = t.rows[row]?.[col];
  if (!c || row === 1 || /[\r\n\0]/.test(text))
    throw Error("Choose a data/header cell; line breaks are source-only.");
  // Require the same GFM escape rules as the display grammar.
  let escaped = "",
    slashes = 0;
  for (const c of text) {
    if (c === "|" && slashes % 2 === 0) escaped += "\\";
    escaped += c;
    slashes = c === "\\" ? slashes + 1 : 0;
  }
  if (cellSpans(`|${escaped}|`).length !== 1)
    throw Error("Ambiguous pipe escaping; edit in Source.");
  return {
    from: t.lines[row].from + c.from,
    to: t.lines[row].from + c.to,
    insert: escaped,
  };
}
export function tableShape(
  state: EditorState,
  at: number,
  action: "left" | "center" | "right",
  index: number,
): Patch;
export function tableShape(
  state: EditorState,
  at: number,
  action: string,
  index: number,
): Patch | Patch[];
export function tableShape(
  state: EditorState,
  at: number,
  action: string,
  index: number,
): Patch | Patch[] {
  const t = tableAt(state, at);
  if (action === "row-add") {
    if (t.rows.length >= 101) throw Error("Table row budget reached.");
    const line = t.lines[Math.min(t.lines.length - 1, Math.max(1, index))];
    return {
      from: line.to,
      to: line.to,
      insert:
        "\n" +
        "|" +
        Array.from({ length: t.cols }, () => "   ").join("|") +
        "|",
    };
  }
  if (action === "row-delete") {
    if (index < 2 || t.rows.length <= 3)
      throw Error("Keep the header, delimiter and at least one data row.");
    const line = t.lines[index];
    return {
      from: line.from,
      to: line.to < state.doc.length ? line.to + 1 : line.to,
      insert: "",
    };
  }
  if (action === "column-add") {
    if (t.cols >= 16) throw Error("Table column budget reached.");
    return t.lines.map((line, i) => {
      const at = line.from + line.text.lastIndexOf("|") + 1;
      return { from: at, to: at, insert: i === 1 ? " --- |" : "   |" };
    });
  }
  if (action === "column-delete") {
    if (t.cols <= 1 || index < 0 || index >= t.cols)
      throw Error("Keep at least one column.");
    return t.lines.map((line, i) => ({
      from: line.from + t.rows[i][index].rawFrom - 1,
      to: line.from + t.rows[i][index].rawTo,
      insert: "",
    }));
  }
  if (["left", "center", "right"].includes(action)) {
    const c = t.rows[1][index];
    if (!c) throw Error("Choose a column.");
    return {
      from: t.lines[1].from + c.from,
      to: t.lines[1].from + c.to,
      insert:
        action === "left" ? ":---" : action === "right" ? "---:" : ":---:",
    };
  }
  throw Error("Unknown table operation.");
}
export function imageText(
  path: string,
  alt: string,
  title: string,
  width: number | null = null,
  height: number | null = null,
) {
  if (
    !path ||
    path.length > 2048 ||
    /[\r\n\0]/.test(path) ||
    /^(?:[a-z][\w+.-]*:|\/\/)/i.test(path)
  )
    throw Error("Choose a local image path.");
  const label = alt
    .replace(/\\/g, "\\\\")
    .replace(/[[\]]/g, "\\$&")
    .replace(/[\r\n]/g, " ");
  const target = path.replace(/[% #?<>()[\]"\\]/g, (c) =>
    encodeURIComponent(c).replace(
      /[!'()*]/g,
      (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
    ),
  );
  const caption = title
    ? ' "' +
      title
        .replace(/\\/g, "\\\\")
        .replace(/"/g, '\\"')
        .replace(/[\r\n]/g, " ") +
      '"'
    : "";
  for (const n of [width, height])
    if (n !== null && (!Number.isInteger(n) || n < 1 || n > 4096))
      throw Error("Image dimensions must be 1–4,096 pixels.");
  return (
    `![${label}](${target}${caption})` +
    (width !== null || height !== null
      ? `{${width !== null ? `width=${width}` : ""}${width !== null && height !== null ? " " : ""}${height !== null ? `height=${height}` : ""}}`
      : "")
  );
}

// Exact inline spelling admitted by the image controls; references/angle paths
// and other spellings stay visible in Source instead of guessing a patch.
export function imageSpan(state: EditorState, at: number) {
  const line = state.doc.lineAt(at);
  const column = at - line.from;
  const pattern =
    /!\[((?:\\.|[^\]\\\n])*)\]\(([^\s)]+)(?:\s+"((?:\\.|[^"\\\n])*)")?\)(?:\{((?:(?:width|height)=\d+\s*){1,2})\})?/g;
  const matches = [...line.text.matchAll(pattern)];
  const m =
    matches.find((m) => column >= m.index && column < m.index + m[0].length) ??
    matches.find((m) => column === m.index + m[0].length);
  if (!m) return null;
  const from = line.from + m.index;
  const to = from + m[0].length;
  const imageEnd = m[4] ? to - m[4].length - 2 : to;
  let node = ensureSyntaxTree(state, imageEnd, 25)?.resolveInner(from + 1, 1);
  while (node && node.name !== "Image") node = node.parent ?? undefined;
  if (node?.from !== from || node.to !== imageEnd) return null;
  const token = grammar()
    .parseInline(m[0], { ...emptyContext() })[0]
    ?.children?.find((t) => t.type === "image");
  if (!token) return null;
  let path: string;
  try {
    path = decodeURIComponent(m[2]);
  } catch {
    return null;
  }
  const dimensions: Record<string, number> = {};
  for (const d of (m[4] ?? "").matchAll(/(width|height)=(\d+)/g)) {
    if (dimensions[d[1]]) return null;
    dimensions[d[1]] = Number(d[2]);
  }
  return {
    from,
    to,
    path,
    alt: m[1].replace(/\\([\\[\]])/g, "$1"),
    title: (m[3] ?? "").replace(/\\(["\\])/g, "$1"),
    width: dimensions.width,
    height: dimensions.height,
  };
}

// Only the verified body is replaced: keep delimiters, fence spelling and EOL pieces.
export function richSpan(
  state: EditorState,
  kind: string,
  from: number,
  to = from,
) {
  if (kind === "mermaid") {
    const tree = ensureSyntaxTree(state, to, 25);
    if (!tree) throw Error("Parsing is incomplete. Edit this block in Source.");
    let node = tree.resolveInner(from, 1);
    for (; node; node = node.parent!) {
      if (node.name !== "FencedCode") continue;
      const text = state.doc.sliceString(node.from, node.to);
      const m = /^(`{3,}|~{3,})mermaid[\t ]*\n([\s\S]*?)\n\1[\t ]*$/.exec(text);
      if (!m || to > node.to || text.length > 16384)
        throw Error("This fence spelling remains editable in Source.");
      const start = node.from + text.indexOf("\n") + 1;
      return { from: start, to: start + m[2].length, body: m[2] };
    }
  }
  if (!writableBlock(state, from) || !writableBlock(state, to))
    throw Error("This code/HTML context remains editable in Source.");
  if (kind === "math") {
    const line = state.doc.lineAt(from);
    for (const m of line.text.matchAll(
      /(?<![\\\w])(\${1,2})([^$\n]{1,8192})\1/g,
    )) {
      const start = line.from + m.index!;
      if (start > from || start + m[0].length < to) continue;
      const tokens = grammar().parseInline(m[0], { ...emptyContext() })[0]
        ?.children;
      if (
        tokens?.length !== 1 ||
        tokens[0].type !== "math" ||
        tokens[0].content !== m[2]
      )
        throw Error("The display parser does not agree with this expression.");
      return {
        from: start + m[1].length,
        to: start + m[0].length - m[1].length,
        body: m[2],
      };
    }
  }
  return null;
}
