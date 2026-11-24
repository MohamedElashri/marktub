<script lang="ts">
  import { errorMessage } from "../bridge";
  import * as api from "../library";

  let preview = $state(""),
    previewName = $state(""),
    previewHtml = $state(""),
    previewError = $state("");
  let previewRequest = 0;
  let previewDialog: HTMLDialogElement;

  export async function show(item: api.Item) {
    if (item.kind !== "file") return;
    const request = ++previewRequest;
    previewName = item.name;
    preview = "Loading preview…";
    previewHtml = "";
    previewError = "";
    previewDialog.showModal();
    try {
      const [text, { renderSection }, { sanitizePreview }] = await Promise.all([
        api.preview(item.id),
        import("../reader/render"),
        import("../reader/sanitize"),
      ]);
      if (request !== previewRequest || !previewDialog?.open) return;
      previewHtml = sanitizePreview(renderSection(text).html);
      preview = text;
    } catch (error) {
      if (request === previewRequest && previewDialog?.open)
        previewError = errorMessage(error);
    }
  }
</script>

<dialog
  bind:this={previewDialog}
  class="quick-preview"
  aria-label="Quick preview"
>
  <div class="library-title">
    <h2>{previewName}</h2>
    <button onclick={() => previewDialog.close()}>Close preview</button>
  </div>
  <p>
    First 64 KiB, at most 32 blocks. Context-limited preview; open the file for
    the complete reader.
  </p>
  {#if previewError}<p role="alert">{previewError}</p>
  {:else if previewHtml}<article class="prose">
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html previewHtml}
    </article>{:else}<pre>{preview}</pre>{/if}
</dialog>
