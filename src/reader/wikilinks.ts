import type MarkdownIt from "markdown-it";

export function wikiHref(target: string) {
  if (
    !target ||
    target.length > 512 ||
    Array.from(target).some(
      (c) =>
        c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127 || "\\:?[]".includes(c),
    )
  )
    return null;
  const [path, ...anchor] = target.split("#");
  if (path.startsWith("/")) return null;
  try {
    const href = path.split("/").map(encodeURIComponent).join("/");
    return (
      (path ? href + (/\.(?:md|markdown)$/i.test(path) ? "" : ".md") : "") +
      (anchor.length ? "#" + encodeURIComponent(anchor.join("#")) : "")
    );
  } catch {
    return null;
  }
}

export function installWikiLinks(md: InstanceType<typeof MarkdownIt>) {
  md.inline.ruler.before("link", "wiki_link", (state, silent) => {
    const start = state.pos;
    if (
      state.linkLevel ||
      state.src.slice(start, start + 2) !== "[[" ||
      state.src[start - 1] === "!"
    )
      return false;
    const end = state.src.indexOf("]]", start + 2);
    if (end < 0 || end - start > 768) return false;
    const body = state.src.slice(start + 2, end);
    if (body.includes("\n") || body.includes("\r")) return false;
    const separator = body.indexOf("|");
    const target = (separator < 0 ? body : body.slice(0, separator)).trim();
    const label = separator < 0 ? target : body.slice(separator + 1).trim();
    const href = wikiHref(target);
    if (!href || !label) return false;
    if (!silent) {
      const token = state.push("wiki_link", "a", 0);
      token.content = label;
      token.meta = { target, href };
    }
    state.pos = end + 2;
    return true;
  });
  md.renderer.rules.wiki_link = (tokens, index) => {
    const token = tokens[index];
    return `<a class="wiki-link" href="${md.utils.escapeHtml(String(token.meta?.href ?? ""))}" data-note-link="${md.utils.escapeHtml(String(token.meta?.target ?? ""))}">${md.utils.escapeHtml(token.content)}</a>`;
  };
}
