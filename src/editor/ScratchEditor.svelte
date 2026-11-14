<script lang="ts">
  import { onMount } from "svelte";
  import { Compartment, EditorState } from "@codemirror/state";
  import {
    EditorView,
    keymap,
    lineNumbers,
    drawSelection,
  } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
  import { markdown } from "@codemirror/lang-markdown";
  import { liveFormatting } from "./visual";

  let {
    text,
    visual,
    onchange,
  }: { text: string; visual: boolean; onchange: (changed: boolean) => void } =
    $props();
  let host: HTMLDivElement;
  let view: EditorView | undefined;
  const formatting = new Compartment();

  export function snapshot(): string {
    return view?.state.doc.toString() ?? text;
  }

  onMount(() => {
    const original = EditorState.create({
      doc: text.replace(/^\uFEFF/, ""),
    }).doc;
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: text.replace(/^\uFEFF/, ""),
        extensions: [
          markdown(),
          history(),
          lineNumbers(),
          drawSelection(),
          EditorView.lineWrapping,
          keymap.of([...defaultKeymap, ...historyKeymap]),
          EditorView.contentAttributes.of({
            "aria-label": "Scratch Markdown editor",
            spellcheck: "false",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange(!update.state.doc.eq(original));
          }),
          formatting.of(visual ? liveFormatting : []),
        ],
      }),
    });
    return () => {
      view?.destroy();
      view = undefined;
    };
  });

  $effect(() => {
    const enabled = visual;
    view?.dispatch({
      effects: formatting.reconfigure(enabled ? liveFormatting : []),
    });
  });
</script>

<div class="editor-host" bind:this={host}></div>
