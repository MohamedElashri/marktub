<script lang="ts">
  let { name }: { name: string } = $props();
  const paths: Record<string, string> = {
    undo: "M7 4 3 8l4 4M3 8h8a5 5 0 0 1 0 10",
    redo: "m13 4 4 4-4 4m4-4H9a5 5 0 0 0 0 10",
    bold: "M6 3h5a3.5 3.5 0 0 1 0 7H6m0 0h6a3.5 3.5 0 0 1 0 7H6V3",
    italic: "M9 3h7M4 17h7M13 3 7 17",
    code: "m7 5-5 5 5 5m6-10 5 5-5 5",
    link: "m8 12 4-4m-5 5-1 1a3.5 3.5 0 0 1-5-5l4-4a3.5 3.5 0 0 1 5 0m0 0 1-1a3.5 3.5 0 0 1 5 5l-4 4a3.5 3.5 0 0 1-5 0",
    heading1: "M3 4v12M11 4v12M3 10h8m4 2 2-2v6m-2 0h4",
    list: "M7 5h11M7 10h11M7 15h11M2 5h.1M2 10h.1M2 15h.1",
    ordered:
      "m2 3 1-1v5m-1 0h2m-2 5a1 1 0 1 1 2 0l-2 4h2M8 5h10M8 10h10M8 15h10",
    task: "M8 3H3v14h14v-7M6 8l3 3 8-8",
    quote: "M3 5h5v6H3V5Zm0 6v4l5-4m4-6h5v6h-5V5Zm0 6v4l5-4",
    table: "M2 3h16v14H2V3Zm0 5h16M2 12h16M8 3v14",
    image: "M2 3h16v14H2V3Zm1 12 5-5 4 4 3-3 3 3M12 6h.1",
    math: "M16 3H4l7 7-7 7h12",
    mermaid:
      "M8 2h5v4H8V2Zm2.5 4v4M4 10h13M4 10v3m13-3v3M2 13h5v4H2v-4Zm12 0h5v4h-5v-4Z",
    fold: "M3 3h14M3 17h14m-10-9 3 3 3-3",
    unfold: "M3 3h14M3 17h14m-10-9 3-3 3 3m-6 4 3 3 3-3",
    find: "M8 3a5 5 0 1 0 0 10A5 5 0 0 0 8 3Zm4 9 5 5",
    focus: "M7 2H2v5m11-5h5v5M2 13v5h5m11-5v5h-5",
    folder: "M2 5h6l2 2h8v10H2V5Z",
    file: "M4 2h8l4 4v12H4V2Zm8 0v5h4M7 11h6M7 14h6",
  };
</script>

<svg
  class="ui-icon"
  width="18"
  height="18"
  viewBox="0 0 20 20"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"><path d={paths[name] ?? paths.file} /></svg
>
