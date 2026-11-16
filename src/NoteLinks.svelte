<script lang="ts">
  import * as api from "./library";
  import { releaseDocument, errorMessage, type Descriptor } from "./bridge";
  import { noteRelationships, resolveNote, type NoteReference } from "./notes";
  let {
    documentId,
    onopen,
  }: {
    documentId: string;
    onopen: (document: Descriptor, link?: string) => Promise<void>;
  } = $props();
  let dialog: HTMLDialogElement;
  let catalogue = $state<api.NoteCatalogue | null>(null),
    busy = $state(false),
    error = $state("");
  let choices = $state<api.NoteCapture[]>([]),
    fragment = $state("");
  let serial = 0;
  const relationships = $derived(
    catalogue ? noteRelationships(catalogue) : null,
  );
  function close() {
    serial++;
    busy = false;
    dialog.close();
  }
  async function load(request: number) {
    busy = true;
    error = "";
    choices = [];
    catalogue = null;
    try {
      const result = await api.noteCatalogue(documentId);
      if (request !== serial) return null;
      catalogue = result;
      return result;
    } catch (e) {
      if (request === serial) error = errorMessage(e);
      return null;
    } finally {
      if (request === serial) busy = false;
    }
  }
  export async function show() {
    dialog.showModal();
    await load(++serial);
  }
  async function choose(note: api.NoteCapture, anchor = "") {
    if (busy) return;
    const request = ++serial;
    busy = true;
    error = "";
    try {
      const document = await api.open(note.item.id);
      if (request !== serial) {
        await releaseDocument(document.id);
        return;
      }
      await onopen(
        document,
        anchor ? "#" + encodeURIComponent(anchor) : undefined,
      );
      if (request === serial) close();
    } catch (e) {
      if (request === serial) error = errorMessage(e);
    } finally {
      if (request === serial) busy = false;
    }
  }
  async function followReference(reference: NoteReference, refresh = false) {
    if (busy) return;
    dialog.showModal();
    const current = refresh || !catalogue ? await load(++serial) : catalogue;
    if (!current) return;
    const resolved = resolveNote(current, current.active, reference);
    if (resolved.definite && resolved.matches.length === 1) {
      await choose(resolved.matches[0], resolved.fragment);
      return;
    }
    choices = resolved.matches;
    fragment = resolved.fragment;
    error = choices.length
      ? "Choose the note you meant. This name is ambiguous or the folder scan is incomplete."
      : "No matching note in the scanned folder. Check the name, authorize the folder, or refresh the links.";
  }
  export async function follow(target: string) {
    await followReference({ target, wiki: true }, true);
  }
</script>

<dialog
  bind:this={dialog}
  aria-label="Note links"
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
>
  <div class="library-title">
    <h2>Note links</h2>
    <button onclick={close}>Close links</button>
  </div>
  <p>
    Links in saved Markdown. Unsaved edits stay in your editor and are not
    indexed.
  </p>
  <button onclick={() => load(++serial)} disabled={busy}>Refresh links</button>
  {#if busy}<p role="status">Reading authorized notes…</p>{/if}
  {#if error}<p role="status">{error}</p>{/if}
  {#if choices.length}<section aria-label="Matching notes">
      <h3>Choose a note</h3>
      {#each choices as note (note.item.id)}<button
          class="note-link-result"
          disabled={busy}
          onclick={() => choose(note, fragment)}>{note.path}</button
        >{/each}
    </section>{/if}
  {#if catalogue && relationships}
    <p class="library-help" role="status">
      {catalogue.root.name} · {catalogue.notes.length} notes scanned. {catalogue.complete
        ? "Folder scan complete."
        : "Partial scan; missing results are possible."}
      {catalogue.skipped
        ? `${catalogue.skipped} notes skipped for content.`
        : ""}
    </p>
    <details class="library-policy">
      <summary>Scope and limits</summary>
      <p>
        Up to 200 visible saved Markdown notes, 64 KiB per note and 128 links
        per note. Hidden, ignored and symlinked files are excluded. {relationships.clipped
          ? "Some links or current-note content exceed these limits."
          : ""}
      </p>
    </details>
    <section aria-label="Outgoing note links">
      <h3>Links from this note</h3>
      {#each relationships.outgoing as reference, index (index)}<button
          class="note-link-result"
          disabled={busy}
          onclick={() => followReference(reference)}>{reference.target}</button
        >{:else}<p>No indexed outgoing note links.</p>{/each}
    </section>
    <section aria-label="Backlinks">
      <h3>Links to this note</h3>
      {#each relationships.backlinks as note (note.item.id)}<button
          class="note-link-result"
          disabled={busy}
          onclick={() => choose(note)}>{note.path}</button
        >{:else}<p>No matching backlinks in the scanned notes.</p>{/each}
    </section>
  {/if}
</dialog>
