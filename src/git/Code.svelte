<script lang="ts">
  import { onMount } from "svelte";
  import { intraline } from "./spans";
  let {
    text,
    other = null,
    path,
    side,
    syntaxEnabled = true,
  }: {
    text: string;
    other?: string | null;
    path: string;
    side: "old" | "new";
    syntaxEnabled?: boolean;
  } = $props();
  let marks = $derived(
    other !== null && text !== other
      ? intraline(side === "old" ? text : other, side === "old" ? other : text)[
          side === "old" ? "left" : "right"
        ]
      : null,
  );
  let tokens = $state<{ text: string; className: string }[] | null>(null),
    syntax = $state<typeof import("../renderers/highlight").highlight>();
  const lang = $derived(
    /\.(?:[cm]?js|ts)$/.test(path)
      ? "javascript"
      : path.endsWith(".rs")
        ? "rust"
        : path.endsWith(".json")
          ? "json"
          : /\.(sh|bash)$/.test(path)
            ? "bash"
            : "",
  );
  onMount(() => {
    if (lang)
      void import("../renderers/highlight").then((m) => (syntax = m.highlight));
  });
  $effect(() => {
    const value = text,
      language = lang,
      highlight = syntax;
    tokens = null;
    if (
      !syntaxEnabled ||
      !highlight ||
      !language ||
      marks ||
      value.length > 1000
    )
      return;
    const template = document.createElement("template");
    template.innerHTML = highlight(value, language);
    const result: { text: string; className: string }[] = [];
    const walk = (node: Node, classes: string) => {
      if (node.nodeType === Node.TEXT_NODE)
        result.push({ text: node.textContent ?? "", className: classes });
      else if (node instanceof HTMLElement)
        for (const child of node.childNodes)
          walk(
            child,
            /^hljs-[\w-]+$/.test(node.className) ? node.className : classes,
          );
    };
    for (const n of template.content.childNodes) walk(n, "");
    tokens = result;
  });
</script>

{#if marks}{marks[0]}<mark class:removed={side === "old"}>{marks[1]}</mark
  >{marks[2]}{:else if tokens}{#each tokens as token, i (i)}<span
      class={token.className}>{token.text}</span
    >{/each}{:else}{text}{/if}
