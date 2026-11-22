<script lang="ts">
  import JobOutput from "./JobOutput.svelte";
  import type { Fence, InlineRun } from "./fence";

  let {
    fence,
    getRun,
    onrun,
    oncancel,
    onclear,
    isBusy,
  }: {
    fence: Fence;
    getRun: (fence: Fence) => InlineRun | undefined;
    onrun: (fence: Fence) => void;
    oncancel: (fence: Fence) => void;
    onclear: (fence: Fence) => void;
    isBusy: () => boolean;
  } = $props();
  const run = $derived(getRun(fence));
  const result = $derived(run?.result);
  const running = $derived(result?.status === "running");
  const shell = $derived(fence.kind === "shell");
</script>

<div
  class="inline-fence"
  role="group"
  aria-label={shell ? "Shell code block" : "SQL code block"}
>
  <div class="inline-actions">
    <span class="inline-label">{shell ? "Shell" : "SQLite · Read only"}</span>
    {#if running}
      <button onclick={() => oncancel(fence)}>Cancel job</button>
    {:else}
      <button disabled={isBusy() || run?.pending} onclick={() => onrun(fence)}
        >{run?.pending ? "Preparing…" : shell ? "Run shell" : "Run SQL"}</button
      >
    {/if}
  </div>
  {#if result || run?.error}
    <div class="inline-result">
      <div class="inline-actions">
        <span class="inline-label">{shell ? "Shell output" : "SQL result"}</span
        >
        {#if result}<span class="inline-state" role="status"
            >{result.status}</span
          >{/if}
        {#if !running}<button onclick={() => onclear(fence)}
            >Clear output</button
          >{/if}
      </div>
      {#if run?.error}<p class="inline-error" role="alert">{run.error}</p>{/if}
      {#if result}
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
    padding: 0 9px;
    color: var(--ink);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 7px;
    font: 13px/1.5 var(--ui-font, system-ui);
  }
  .inline-actions {
    display: flex;
    gap: 9px;
    align-items: center;
    flex-wrap: wrap;
    min-block-size: 38px;
  }
  .inline-actions button {
    margin-inline-start: auto;
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
  .inline-state + button {
    margin-inline-start: 0;
  }
  .inline-result {
    padding-block: 2px 8px;
    border-top: 1px solid var(--line);
  }
  .inline-message,
  .inline-error {
    margin: 0;
  }
  .inline-error {
    color: var(--removed, #922c33);
  }
</style>
