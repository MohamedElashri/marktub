<script lang="ts">
  import { tick } from "svelte";
  import { errorMessage } from "./bridge";
  let {
    oncreate,
    oncreated,
  }: {
    oncreate: (title: string, template: string) => Promise<void>;
    oncreated: () => void;
  } = $props();
  let dialog: HTMLDialogElement, titleInput: HTMLInputElement;
  let title = $state(""),
    template = $state("note"),
    busy = $state(false),
    error = $state("");
  const templates = [
    ["note", "Note", "Notes and next steps."],
    ["meeting", "Meeting", "Attendees, agenda, decisions and action items."],
    ["journal", "Daily journal", "Priorities, notes and reflection."],
  ];
  export async function show() {
    error = "";
    title = "";
    dialog.showModal();
    await tick();
    titleInput.focus();
  }
  async function create() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      await oncreate(title.trim() || "Untitled", template);
      dialog.close();
      await tick();
      oncreated();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  aria-label="New note from template"
  oncancel={(event) => {
    if (busy) event.preventDefault();
  }}
>
  <form
    onsubmit={(event) => {
      event.preventDefault();
      void create();
    }}
  >
    <h2>Start with a template</h2>
    <p>Create a local draft. Choose its file and folder when you save.</p>
    <label
      >Note title<input
        bind:this={titleInput}
        bind:value={title}
        maxlength="160"
        placeholder="Untitled"
        disabled={busy}
      /></label
    >
    <fieldset disabled={busy}>
      <legend>Template</legend>
      {#each templates as [id, label, description] (id)}
        <label class="note-template"
          ><input
            type="radio"
            name="note-template"
            value={id}
            bind:group={template}
          /><span><strong>{label}</strong><small>{description}</small></span
          ></label
        >
      {/each}
    </fieldset>
    {#if error}<p role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <button type="button" onclick={() => dialog.close()} disabled={busy}
        >Cancel</button
      ><button type="submit" class="primary" disabled={busy}
        >{busy ? "Creating…" : "Create note"}</button
      >
    </div>
  </form>
</dialog>
