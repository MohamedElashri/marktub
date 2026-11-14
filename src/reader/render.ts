import MarkdownIt from "markdown-it";

export const PREVIEW_BYTES = 64 * 1024;
export const BLOCK_BUDGET = 32;

const markdown = new MarkdownIt({
  html: false,
  linkify: false,
  typographer: false,
  maxNesting: 32,
});

// No network/resource-bearing tags or navigable document links in this bootstrap.
markdown.renderer.rules.image = (tokens, index) =>
  `<span class="image-placeholder">[Image: ${markdown.utils.escapeHtml(tokens[index].content)}]</span>`;
markdown.renderer.rules.link_open = () => '<span class="document-link">';
markdown.renderer.rules.link_close = () => "</span>";

export interface RenderResult {
  html: string;
  blocks: number;
  clipped: boolean;
  parseMs: number;
}

export function renderSection(text: string): RenderResult {
  if (new TextEncoder().encode(text).byteLength > PREVIEW_BYTES) {
    throw new Error("Preview input exceeds the bounded source window.");
  }
  const start = performance.now();
  const env = {};
  const tokens = markdown.parse(text.replace(/^\uFEFF/, ""), env);
  let blocks = 0;
  let end = tokens.length;
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].level === 0 && tokens[i].nesting !== -1) {
      blocks++;
      if (blocks > BLOCK_BUDGET) {
        end = i;
        break;
      }
    }
  }
  const visible = tokens.slice(0, end);
  const tokenCost = visible.reduce(
    (total, token) => total + 1 + (token.children?.length ?? 0),
    0,
  );
  if (
    tokenCost > 1024 ||
    visible.some((token) =>
      token.content.split("\n").some((line) => line.length > 4096),
    )
  ) {
    const excerpt = text.slice(0, 4096).replace(/[\uD800-\uDBFF]$/, "");
    return {
      html: `<pre>${markdown.utils.escapeHtml(excerpt)}</pre>`,
      blocks: 1,
      clipped: true,
      parseMs: performance.now() - start,
    };
  }
  return {
    html: markdown.renderer.render(visible, markdown.options, env),
    blocks: Math.min(blocks, BLOCK_BUDGET),
    clipped: end < tokens.length,
    parseMs: performance.now() - start,
  };
}
