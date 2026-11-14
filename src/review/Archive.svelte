<script lang="ts">
  import * as api from "./bridge";
  import { errorMessage } from "../bridge";
  let dialog: HTMLDialogElement, confirmation: HTMLDialogElement;
  let field = $state<HTMLTextAreaElement>();
  let entries = $state<api.Entry[]>([]),
    failure = $state(""),
    notice = $state(""),
    report = $state(""),
    pending = $state<api.Entry | null>(null),
    busy = $state(false),
    snippets = $state(false);
  export async function show() {
    dialog.showModal();
    await refresh();
  }
  async function refresh() {
    busy = true;
    try {
      entries = await api.list();
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function exportRecord(entry: api.Entry, save = false) {
    busy = true;
    try {
      if (save) {
        if (await api.archiveSave(entry.id, snippets, "json"))
          notice =
            "Archived feedback exported. No path authority was restored.";
      } else report = await api.archiveReport(entry.id, snippets);
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function remove() {
    if (!pending) return;
    busy = true;
    try {
      await api.remove(pending.id);
      pending = null;
      confirmation.close();
      report = "";
      await refresh();
      notice = "Review data deleted. Source files are unchanged.";
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="review-archive"
  aria-label="Stored document reviews"
>
  <header>
    <h2>Stored reviews</h2>
    <button onclick={() => dialog.close()}>Close stored reviews</button>
  </header>
  <p>
    Private review records are separate from the library index. Opening this
    list restores no source path permissions. Archive exports always redact
    paths; current anchor status is not evaluated without a live review capture.
  </p>
  <p>Up to 128 records; shared private-store budget 512 MiB.</p>
  {#if failure}<p role="alert">{failure}</p>{/if}{#if notice}<p role="status">
      {notice}
    </p>{/if}<label
    ><input
      type="checkbox"
      bind:checked={snippets}
      onchange={() => (report = "")}
    />Include source quotes/context in archive exports</label
  >{#each entries as entry (entry.id)}<section>
      <strong>{entry.name}</strong>
      <nav>
        <button onclick={() => exportRecord(entry)} disabled={busy}
          >Prepare archived feedback</button
        ><button onclick={() => exportRecord(entry, true)} disabled={busy}
          >Export archived feedback…</button
        ><button
          onclick={() => {
            pending = entry;
            confirmation.showModal();
          }}
          disabled={busy}>Delete stored review…</button
        >
      </nav>
    </section>{/each}{#if !entries.length && !busy}<p>
      No stored document reviews.
    </p>{/if}{#if report}<textarea
      aria-label="Archived review feedback"
      readonly
      value={report}
      bind:this={field}
    ></textarea><button
      onclick={() => {
        try {
          if (field) api.copyPrepared(report, field);
          notice = "Archived feedback copied.";
        } catch (e) {
          failure = errorMessage(e);
        }
      }}>Copy archived feedback</button
    >{/if}
</dialog>
<dialog bind:this={confirmation} aria-label="Confirm review deletion">
  <h2>Delete this review data?</h2>
  <p>
    {pending?.name}: all captured baselines, comments, replies and decisions
    will be deleted. Export first if you need a copy. The source document is
    unchanged.
  </p>
  <button
    onclick={() => {
      pending = null;
      confirmation.close();
    }}>Keep review data</button
  ><button onclick={remove} disabled={busy}>Delete review data</button>
</dialog>
