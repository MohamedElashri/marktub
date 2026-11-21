<script lang="ts">
  import JobOutput from "./JobOutput.svelte";
  import type { Database } from "./bridge";
  import type { Fence, InlineRun } from "./fence";

  let {
    fence,
    getRun,
    onrun,
    oncancel,
    onclear,
    isBusy,
    needsSave,
    getDatabase,
    onselectdatabase,
    onresetdatabase,
  }: {
    fence: Fence;
    getRun: (fence: Fence) => InlineRun | undefined;
    onrun: (fence: Fence) => void;
    oncancel: (fence: Fence) => void;
    onclear: (fence: Fence) => void;
    isBusy: () => boolean;
    needsSave: () => boolean;
    getDatabase: () => Database | undefined;
    onselectdatabase: () => void;
    onresetdatabase: () => void;
  } = $props();
  const run = $derived(getRun(fence));
  const result = $derived(run?.result);
  const running = $derived(result?.status === "running");
  const shell = $derived(fence.kind === "shell");
  const database = $derived(getDatabase());
  const databaseName = $derived(database?.name.split(/[\\/]/).at(-1));
  const hasOutput = $derived(
    !!(
      result?.output?.stdout ||
      result?.output?.stderr ||
      result?.table?.columns.length
    ),
  );
  let outputOpen = $state(true);
  let copied = $state("");
  async function copyOutput() {
    if (!result) return;
    const csv = (row: string[]) =>
      row.map((cell) => `"${cell.replaceAll('"', '""')}"`).join(",");
    const text = result.table
      ? [result.table.columns, ...result.table.rows].map(csv).join("\r\n")
      : [result.output?.stdout, result.output?.stderr]
          .filter(Boolean)
          .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      copied = result.table ? "Rows copied" : "Output copied";
    } catch {
      copied = "Clipboard unavailable";
    }
  }
</script>

<div
  class="inline-fence"
  role="group"
  aria-label={shell ? "Shell code block" : "SQL code block"}
>
  <div class="inline-actions">
    <div class="inline-context">
      <span class="inline-label">{shell ? "Shell" : "SQLite"}</span>
      {#if !shell}
        <span class="inline-database" title={database?.name}>
          <bdi>{databaseName ?? "Temporary database"}</bdi> · Read only
        </span>
      {/if}
    </div>
    {#if !shell && !needsSave() && !running}
      <button disabled={isBusy()} onclick={onselectdatabase}
        >{database ? "Change database…" : "Choose database…"}</button
      >
      {#if database}<button disabled={isBusy()} onclick={onresetdatabase}
          >Use temporary database</button
        >{/if}
    {/if}
    <button
      class:primary={!running}
      disabled={!running && (isBusy() || run?.pending)}
      onclick={() => {
        if (running) oncancel(fence);
        else {
          copied = "";
          outputOpen = true;
          onrun(fence);
        }
      }}
      >{running
        ? "Cancel job"
        : run?.pending
          ? "Preparing…"
          : needsSave()
            ? "Save copy & run…"
            : shell
              ? "Run shell"
              : "Run SQL"}</button
    >
  </div>
  {#if result || run?.error}
    <div class="inline-result">
      <div class="inline-actions">
        <span class="inline-label">{shell ? "Shell output" : "SQL result"}</span
        >
        {#if result}<span class="inline-state" role="status"
            >{result.status}</span
          >{/if}
        {#if hasOutput}
          <button
            aria-expanded={outputOpen}
            onclick={() => (outputOpen = !outputOpen)}
            >{outputOpen ? "Hide output" : "Show output"}</button
          >
          <button onclick={copyOutput}
            >{result?.table ? "Copy rows as CSV" : "Copy output"}</button
          >
        {/if}
        {#if !running}<button onclick={() => onclear(fence)}
            >Clear output</button
          >{/if}
      </div>
      {#if copied}<p class="inline-feedback" role="status">{copied}</p>{/if}
      {#if run?.error}<p class="inline-error" role="alert">{run.error}</p>{/if}
      {#if result && outputOpen}
        <p class="inline-message">{result.message}</p>
        <JobOutput {result} />
      {/if}
    </div>
  {/if}
</div>

<style>
  .inline-fence {
    display: grid;
    margin: 0 0 1.25rem;
    padding: 0 12px;
    color: var(--ink);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 0 0 7px 7px;
    font: 13px/1.5 var(--ui-font, system-ui);
  }
  .inline-actions {
    display: flex;
    gap: 9px;
    align-items: center;
    flex-wrap: wrap;
    min-block-size: 42px;
    padding-block: 5px;
  }
  .inline-context {
    display: grid;
    min-inline-size: 0;
    margin-inline-end: auto;
  }
  .inline-database {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--quiet);
    font-size: 12px;
  }
  .inline-actions button:not(.primary) {
    color: var(--accent);
  }
  .inline-label {
    font-weight: 600;
  }
  .inline-state {
    margin-inline-start: auto;
    color: var(--quiet);
    text-transform: capitalize;
  }
  .inline-result {
    padding-block: 2px 8px;
    border-top: 1px solid var(--line);
  }
  .inline-message,
  .inline-error,
  .inline-feedback {
    margin: 0;
  }
  .inline-error {
    color: var(--removed, #922c33);
  }
  .inline-feedback {
    color: var(--quiet);
  }
</style>
