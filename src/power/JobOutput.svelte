<script lang="ts">
  import type { Result } from "./bridge";
  let { result }: { result: Result } = $props();
  let rowAt = $state(0);
  const rows = $derived(result.table?.rows ?? []);
  const start = $derived(
    Math.min(rowAt, Math.max(0, Math.floor((rows.length - 1) / 25) * 25)),
  );
</script>

{#if result.output}
  <p class="job-meta">Exit code {result.output.code ?? "signal"}</p>
  {#if result.output.stdout}
    <pre aria-label="Job stdout" dir="auto">{result.output.stdout}</pre>
  {/if}
  {#if result.output.stderr}
    <p class="job-meta">Standard error</p>
    <pre class="job-stderr" aria-label="Job stderr" dir="auto">{result.output
        .stderr}</pre>
  {/if}
  {#if !result.output.stdout && !result.output.stderr}<p class="job-meta">
      No output
    </p>{/if}
{/if}
{#if result.table}
  <p class="job-meta">
    {rows.length}
    {rows.length === 1 ? "row" : "rows"} · {result.table.changes}
    {result.table.changes === 1 ? "change" : "changes"}{result.table.truncated
      ? " · results truncated"
      : ""}
  </p>
  {#if result.table.columns.length}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scrollable results region supports keyboard scrolling.) -->
    <div
      class="job-table"
      tabindex="0"
      role="region"
      aria-label="Query results"
    >
      <table>
        <thead
          ><tr
            >{#each result.table.columns as column, index (index)}
              <th scope="col" dir="auto">{column}</th>
            {/each}</tr
          ></thead
        >
        <tbody
          >{#each rows.slice(start, start + 25) as row, index (start + index)}
            <tr
              >{#each row as cell, index (index)}<td dir="auto">{cell}</td
                >{/each}</tr
            >
          {/each}</tbody
        >
      </table>
    </div>
  {/if}
  {#if rows.length > 25}
    <div class="job-pagination">
      <button disabled={start === 0} onclick={() => (rowAt = start - 25)}
        >Previous rows</button
      >
      <span
        >{start + 1}–{Math.min(start + 25, rows.length)} of {rows.length}</span
      >
      <button
        disabled={start + 25 >= rows.length}
        onclick={() => (rowAt = start + 25)}>Next rows</button
      >
    </div>
  {/if}
{/if}
{#each result.warnings as warning, index (index)}<p class="job-meta">
    {warning}
  </p>{/each}

<style>
  .job-meta {
    margin: 8px 0;
    font-size: 12px;
    color: var(--quiet);
  }
  pre,
  th,
  td {
    text-align: start;
    unicode-bidi: plaintext;
  }
  pre {
    margin: 6px 0 0;
    max-block-size: 16rem;
    padding: 10px 12px;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    background: var(--code);
    border: 1px solid var(--line);
    border-radius: 6px;
    font: 13px/1.5 var(--source-font, monospace);
  }
  .job-stderr {
    border-inline-start: 2px solid var(--removed, #922c33);
  }
  .job-table {
    overflow: auto;
    max-block-size: 20rem;
    border: 1px solid var(--line);
    border-radius: 6px;
  }
  .job-table table {
    margin: 0;
    width: max-content;
    min-width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 7px 10px;
    border-bottom: 1px solid var(--line);
    max-inline-size: 26rem;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  th {
    background: var(--surface);
    position: sticky;
    top: 0;
  }
  .job-pagination {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }
</style>
