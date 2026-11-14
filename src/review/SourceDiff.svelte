<script lang="ts">
  import type { Comparison } from "./bridge";
  let { comparison }: { comparison: Comparison } = $props();
  let at = $state(0),
    offset = $state(0);
  const h = $derived(
    comparison.hunks[Math.min(at, comparison.hunks.length - 1)],
  );
  const old = $derived(comparison.left.split(/(?<=\n)/));
  const next = $derived(comparison.right.split(/(?<=\n)/));
  const length = $derived(h ? Math.max(h.oldLines, h.newLines) : 0);
  $effect(() => {
    void comparison;
    at = 0;
    offset = 0;
  });
</script>

<div class="review-source-diff" aria-label="Saved source diff">
  <nav>
    <button
      disabled={at === 0}
      onclick={() => {
        at--;
        offset = 0;
      }}>Previous source hunk</button
    ><span
      >{h
        ? `${at + 1} / ${comparison.hunks.length}`
        : "No source changes"}</span
    ><button
      disabled={at >= comparison.hunks.length - 1}
      onclick={() => {
        at++;
        offset = 0;
      }}>Next source hunk</button
    >
  </nav>
  {#if h}<p>
      −{h.oldStart + 1},{h.oldLines} +{h.newStart + 1},{h.newLines} · Original saved
      LF coordinates; CR/BOM are shown explicitly.
    </p>
    <div class="review-source-columns">
      <div>
        <strong>Baseline source</strong
        >{#each old.slice(h.oldStart + offset, h.oldStart + Math.min(h.oldLines, offset + 100)) as line, i (i)}<pre><small
              >{h.oldStart + offset + i + 1} − </small>{line
              .replaceAll("\r", "␍")
              .replaceAll("\uFEFF", "⟪BOM⟫")
              .slice(0, 1024)}</pre>{/each}
      </div>
      <div>
        <strong>Current saved source</strong
        >{#each next.slice(h.newStart + offset, h.newStart + Math.min(h.newLines, offset + 100)) as line, i (i)}<pre><small
              >{h.newStart + offset + i + 1} + </small>{line
              .replaceAll("\r", "␍")
              .replaceAll("\uFEFF", "⟪BOM⟫")
              .slice(0, 1024)}</pre>{/each}
      </div>
    </div>
    <nav>
      <button
        disabled={offset === 0}
        onclick={() => (offset = Math.max(0, offset - 100))}
        >Previous source lines</button
      ><span>{offset + 1}–{Math.min(length, offset + 100)} / {length}</span
      ><button disabled={offset + 100 >= length} onclick={() => (offset += 100)}
        >Next source lines</button
      >
    </nav>
    <p>
      At most 100 lines per side, 1,024 characters per line. Whole saved
      captures remain intact.
    </p>{/if}
</div>
