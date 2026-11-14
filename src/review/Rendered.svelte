<script lang="ts">
  import { grammar, emptyContext, escape } from "../reader/grammar";
  import { sanitizeReader } from "../reader/safe";
  let { text }: { text: string } = $props();
  const md = grammar();
  // Changed-block review is deliberately inert: no assets, navigation or rich jobs.
  md.renderer.rules.image = (t, i) =>
    `<span>[Image: ${escape(t[i].content)}]</span>`;
  md.renderer.rules.link_open = () => "<span>";
  md.renderer.rules.link_close = () => "</span>";
  md.renderer.rules.math = (t, i) => `<code>${escape(t[i].content)}</code>`;
  md.renderer.rules.math_block = (t, i) => `<pre>${escape(t[i].content)}</pre>`;
  md.renderer.rules.fence = (t, i) => `<pre>${escape(t[i].content)}</pre>`;
  const html = $derived.by(() => {
    if (new TextEncoder().encode(text).length > 16384)
      return `<pre>${escape(text.slice(0, 4096))}</pre><p>Large block; use Source to inspect all bytes.</p>`;
    const tokens = md.parse(text, { ...emptyContext(), beginning: true });
    if (tokens.reduce((n, t) => n + 1 + (t.children?.length ?? 0), 0) > 1024)
      return `<pre>${escape(text.slice(0, 4096))}</pre><p>Complex block; source excerpt.</p>`;
    return sanitizeReader(
      md.renderer.render(tokens, md.options, { ...emptyContext() }),
    );
  });
</script>

<div class="review-rendered">
  <!-- The sole HTML sink consumes shared sanitization or explicitly escaped app markup. -->
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html html}
</div>
