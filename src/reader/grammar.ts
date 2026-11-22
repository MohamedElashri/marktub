import MarkdownIt from "markdown-it";
import { installWikiLinks } from "./wikilinks";
import type Token from "markdown-it/lib/token.mjs";
import type { Context, Rich } from "./model";
export interface Environment extends Context {
  [key: string]: unknown;
  [key: symbol]: unknown;
  rich?: Rich[];
  heading?: (title: string, level: number) => string;
}
export const emptyContext = (): Context => ({
  references: Object.create(null),
  notes: Object.create(null),
});
export const escape = (text: string) =>
  text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
export const noteId = (id: string) =>
  "h-note-" +
  Array.from(id)
    .map((c) => c.codePointAt(0)!.toString(16))
    .join("-");
export function grammar() {
  const md = new MarkdownIt({
    html: false,
    linkify: true,
    typographer: false,
    maxNesting: 32,
  });
  installWikiLinks(md);
  // Pinned markdown-it rule instrumentation: definitions normally emit no
  // token. Give them zero-output spans so streaming retains a complete final
  // definition (including a continuation title/body), just like a paragraph.
  type BlockRule = Parameters<typeof md.block.ruler.at>[1];
  const reference = (
    md.block.ruler as unknown as {
      __rules__: { name: string; fn: BlockRule }[];
    }
  ).__rules__.find((r) => r.name === "reference")?.fn;
  if (!reference) throw Error("Pinned Markdown reference rule is unavailable.");
  md.block.ruler.at("reference", (state, start, end, silent) => {
    const found = reference(state, start, end, silent);
    if (found && !silent) {
      const token = state.push("definition", "", 0);
      token.map = [start, state.line];
    }
    return found;
  });
  md.renderer.rules.definition = () => "";
  md.renderer.rules.source_only = (tokens, i) =>
    `<p>Oversized footnote definition; source view.</p><pre>${escape(tokens[i].content)}</pre>`;
  md.block.ruler.before("hr", "frontmatter", (state, start, end, silent) => {
    if (
      start !== 0 ||
      !state.env.beginning ||
      state.src.slice(0, state.eMarks[0]).trim() !== "---"
    )
      return false;
    if (silent) return true;
    let next = 1;
    while (
      next < end &&
      !/^(?:---|\.\.\.)\s*$/.test(
        state.src.slice(state.bMarks[next], state.eMarks[next]),
      )
    )
      next++;
    const token = state.push("frontmatter", "", 0);
    token.content = state.getLines(0, Math.min(next + 1, end), 0, false);
    token.map = [0, Math.min(next + 1, end)];
    state.line = Math.min(next + 1, end);
    return true;
  });
  md.renderer.rules.frontmatter = (tokens, i) =>
    `<details class="frontmatter"><summary>Front matter (preserved as source)</summary><pre>${escape(tokens[i].content)}</pre></details>`;
  md.renderer.rules.blockquote_open = (tokens, i) => {
    const label = tokens[i].meta?.callout as string | undefined;
    return label
      ? `<blockquote class="callout callout-${label}"><strong class="callout-label" dir="ltr">${label}</strong>`
      : "<blockquote>";
  };
  md.block.ruler.before("reference", "note", (state, start, end, silent) => {
    if (state.sCount[start] - state.blkIndent >= 4) return false;
    const first = state.src.slice(
      state.bMarks[start] + state.tShift[start],
      state.eMarks[start],
    );
    const m = /^\[\^([^\]\s]{1,80})\]:\s?(.*)$/.exec(first);
    if (!m) return false;
    if (silent) return true;
    let body = m[2],
      next = start + 1;
    while (next < end && state.sCount[next] >= 4) {
      body +=
        "\n" + state.src.slice(state.bMarks[next] + 4, state.eMarks[next]);
      next++;
    }
    const env = state.env as unknown as Environment;
    env.notes ??= Object.create(null);
    if (Object.keys(env.notes).length >= 1024)
      throw Error("Footnote count exceeds the context budget.");
    if (body.length > 16384) {
      const fallback = state.push("source_only", "", 0);
      fallback.content = state.getLines(start, next, state.blkIndent, false);
      fallback.map = [start, next];
      state.line = next;
      return true;
    }
    if (!Object.hasOwn(env.notes, m[1])) env.notes[m[1]] = body;
    const definition = state.push("definition", "", 0);
    definition.map = [start, next];
    state.line = next;
    return true;
  });
  md.inline.ruler.before("link", "note", (state, silent) => {
    const m = /^\[\^([^\]\s]{1,80})\]/.exec(state.src.slice(state.pos));
    if (
      !m ||
      !Object.hasOwn((state.env as unknown as Environment).notes ?? {}, m[1])
    )
      return false;
    if (!silent) {
      const t = state.push("note_ref", "", 0);
      t.content = m[1];
    }
    state.pos += m[0].length;
    return true;
  });
  md.renderer.rules.note_ref = (tokens, i) =>
    `<a href="#${noteId(tokens[i].content)}" class="footnote-ref">[${escape(tokens[i].content)}]</a>`;
  md.inline.ruler.before("escape", "math", (state, silent) => {
    const from = state.pos,
      display =
        state.src.startsWith("\\[", from) || state.src.startsWith("$$", from);
    const slash =
      state.src.startsWith("\\[", from) || state.src.startsWith("\\(", from);
    if (!slash && state.src[from] !== "$") return false;
    const marker = slash ? (display ? "\\]" : "\\)") : display ? "$$" : "$";
    const width = slash || display ? 2 : 1;
    if (
      !slash &&
      (/\s/.test(state.src[from + 1] ?? " ") ||
        /\d/.test(state.src[from - 1] ?? ""))
    )
      return false;
    let end = from + width;
    while (
      (end = state.src.indexOf(marker, end)) >= 0 &&
      state.src[end - 1] === "\\"
    )
      end += marker.length;
    if (
      end < 0 ||
      end - from > 8192 ||
      (!slash && /\s/.test(state.src[end - 1]))
    )
      return false;
    if (!silent) {
      const t = state.push("math", "", 0);
      t.content = state.src.slice(from + width, end);
      t.meta = { display, from, to: end + marker.length };
    }
    state.pos = end + marker.length;
    return true;
  });
  md.block.ruler.before("fence", "math_block", (state, start, end, silent) => {
    const line = state.src.slice(
      state.bMarks[start] + state.tShift[start],
      state.eMarks[start],
    );
    if (!/^\$\$\s*$/.test(line)) return false;
    if (silent) return true;
    let next = start + 1;
    while (
      next < end &&
      !/^\s*\$\$\s*$/.test(
        state.src.slice(state.bMarks[next], state.eMarks[next]),
      )
    )
      next++;
    const t = state.push("math_block", "", 0);
    t.content = state.getLines(start + 1, next, state.blkIndent, false);
    t.map = [start, Math.min(next + 1, end)];
    state.line = Math.min(next + 1, end);
    return true;
  });
  const placeholder = (
    kind: Rich["kind"],
    source: string,
    display: boolean,
    environment: unknown,
  ) => {
    const env = environment as Environment;
    env.rich ??= [];
    const i = env.rich.push({ kind, source, display }) - 1;
    return `<${display ? "div" : "span"} data-rich="${i}" class="rich-placeholder">${kind === "math" ? "Math" : "Diagram"}: ${escape(source.slice(0, 160))}</${display ? "div" : "span"}>`;
  };
  const mapped = (
    token: { attrGet: (name: string) => string | number | null },
    html: string,
  ) => {
    const line = token.attrGet("data-source-line");
    if (line === null) return html;
    const end = token.attrGet("data-source-end-line");
    const kind = token.attrGet("data-run-kind");
    const run = kind
      ? ` data-run-kind="${escape(String(kind))}" data-run-source="${escape(String(token.attrGet("data-run-source")))}"`
      : "";
    return `<div data-source-line="${escape(String(line))}" data-source-end-line="${escape(String(end))}"${run}>${html}</div>`;
  };
  md.renderer.rules.math = (t, i, _o, e) =>
    mapped(t[i], placeholder("math", t[i].content, !!t[i].meta?.display, e));
  md.renderer.rules.math_block = (t, i, _o, e) =>
    mapped(t[i], placeholder("math", t[i].content, true, e));
  md.renderer.rules.fence = (t, i, _o, e) =>
    mapped(
      t[i],
      t[i].info.trim() === "mermaid"
        ? placeholder("mermaid", t[i].content, true, e)
        : `<pre><code class="language-${escape(t[i].info.split(/\s/)[0])}">${escape(t[i].content)}</code></pre>\n`,
    );
  md.renderer.rules.image = (t, i) => {
    const token = t[i];
    const src = token.attrGet("src") ?? "";
    return `<button class="image-request" data-image="${escape(String(src))}"${token.meta?.width ? ` data-width="${token.meta.width}"` : ""}${token.meta?.height ? ` data-height="${token.meta.height}"` : ""}>Load image: ${escape(token.content || "image")}</button>`;
  };
  md.core.ruler.after("inline", "image_dimensions", (state) => {
    for (const block of state.tokens) {
      const children = block.children ?? [];
      for (let i = 0; i < children.length - 1; i++)
        if (children[i].type === "image" && children[i + 1].type === "text") {
          const m =
            /^\{(width|height)=(\d{1,4})(?:[ \t]+(width|height)=(\d{1,4}))?\}/.exec(
              children[i + 1].content,
            );
          if (!m) continue;
          const entries = [[m[1], m[2]], ...(m[3] ? [[m[3], m[4]]] : [])];
          if (
            entries.some(
              ([, value]) => Number(value) < 1 || Number(value) > 4096,
            )
          )
            continue;
          children[i].meta = Object.fromEntries(entries);
          children[i + 1].content = children[i + 1].content.slice(m[0].length);
        }
    }
  });
  md.core.ruler.after("inline", "reading_blocks", (state) => {
    for (let i = 0; i < state.tokens.length; i++) {
      const block = state.tokens[i];
      if (block.type === "th_open" || block.type === "td_open") {
        const align = /^text-align:(left|center|right);?$/.exec(
          String(block.attrGet("style") ?? ""),
        );
        if (align) block.attrSet("align", align[1]);
      }
      if (
        block.type === "blockquote_open" &&
        state.tokens[i + 1]?.type === "paragraph_open"
      ) {
        const inline = state.tokens[i + 2];
        const marker =
          /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\](?=\s|$)/.exec(
            inline?.content ?? "",
          );
        const children = inline?.children;
        if (inline?.type === "inline" && marker && children) {
          block.meta = { ...block.meta, callout: marker[1].toLowerCase() };
          block.attrJoin("class", `callout callout-${marker[1].toLowerCase()}`);
          // Keep inline.content and math source offsets untouched. Only remove
          // the rendered marker, including a marker parsed as a reference link.
          if (children[0]?.type === "text") {
            children[0].content = children[0].content
              .slice(marker[0].length)
              .replace(/^[ \t]+/, "");
            if (!children[0].content) children.shift();
          } else if (
            children[0]?.type === "link_open" &&
            children[1]?.content === `!${marker[1]}` &&
            children[2]?.type === "link_close"
          ) {
            children.splice(0, 3);
          }
          if (
            children[0]?.type === "softbreak" ||
            children[0]?.type === "hardbreak"
          )
            children.shift();
        }
      }
      if (i < 2) continue;
      const first = block.children?.[0];
      if (
        block.type === "inline" &&
        state.tokens[i - 2].type === "list_item_open" &&
        state.tokens[i - 1].type === "paragraph_open" &&
        first?.type === "text" &&
        /^\[[ xX]\]\s/.test(block.content)
      )
        // Keep checkboxes inert. Replace Markdown syntax before rendering so
        // the checked marker's Latin x cannot determine an Arabic item's dir.
        first.content = first.content.replace(/^\[([ xX])\]\s/, (_m, c) =>
          c === " " ? "☐ " : "☑ ",
        );
    }
  });
  md.renderer.rules.heading_open = (tokens, i, _o, env) => {
    const title = tokens[i + 1]?.content ?? "";
    const id = (env as unknown as Environment).heading?.(
      title,
      Number(tokens[i].tag.slice(1)),
    );
    return `<${tokens[i].tag}${id ? ` id="${id}"` : ""}>`;
  };
  return md;
}
export function topStarts(tokens: Token[]): number[] {
  return [
    ...new Set(
      tokens
        .filter((t) => t.level === 0 && t.nesting !== -1 && t.map)
        .map((t) => t.map![0]),
    ),
  ];
}
