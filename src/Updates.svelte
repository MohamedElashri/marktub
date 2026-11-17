<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { errorMessage } from "./bridge";
  let { onrestart }: { onrestart: () => Promise<boolean> } = $props();
  type Status = {
    phase: string;
    currentVersion: string;
    platform: string;
    canInstall: boolean;
    version: string | null;
    notes: string;
    received: number;
    total: number | null;
    error: string | null;
  };
  let dialog: HTMLDialogElement;
  let status = $state<Status | null>(null);
  let error = $state("");
  let installing = $state(false);
  let handedOff = $state(false);
  let focus: HTMLElement | null = null;
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const busy = $derived(
    status?.phase === "checking" || status?.phase === "downloading",
  );
  const linux = $derived(status?.platform === "linux-x86_64-deb");
  async function refresh(request = generation) {
    const next = await invoke<Status>("application_update_status");
    if (request !== generation || !dialog.open) return;
    status = next;
    clearTimeout(timer);
    if (["checking", "downloading"].includes(next.phase))
      timer = setTimeout(() => {
        void refresh(request).catch((e) => {
          error = errorMessage(e);
        });
      }, 250);
  }
  export async function show() {
    focus = document.activeElement as HTMLElement | null;
    generation++;
    error = "";
    dialog.showModal();
    try {
      await refresh();
    } catch (e) {
      error = errorMessage(e);
    }
  }
  async function close() {
    if (installing) return;
    try {
      await invoke("cancel_application_update");
      generation++;
      clearTimeout(timer);
      dialog.close();
      focus?.focus();
    } catch (e) {
      error = errorMessage(e);
    }
  }
  async function action(command: string) {
    const request = generation;
    error = "";
    if (command === "check_application_update") handedOff = false;
    if (
      status &&
      ["check_application_update", "download_application_update"].includes(
        command,
      )
    ) {
      status = {
        ...status,
        phase:
          command === "check_application_update" ? "checking" : "downloading",
        error: null,
      };
      clearTimeout(timer);
      timer = setTimeout(() => {
        void refresh(request).catch((e) => {
          error = errorMessage(e);
        });
      }, 100);
    }
    try {
      await invoke(command);
      if (request !== generation) return;
      if (command === "open_application_installer") handedOff = true;
      await refresh(request);
    } catch (e) {
      if (request === generation) {
        error = errorMessage(e);
        await refresh(request);
      }
    }
  }
  async function restart() {
    installing = true;
    error = "";
    dialog.close();
    try {
      if (!(await onrestart())) dialog.showModal();
    } catch (e) {
      error = errorMessage(e);
      dialog.showModal();
    } finally {
      installing = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="updates-dialog"
  aria-label="Application updates"
  oncancel={(event) => {
    event.preventDefault();
    void close();
  }}
>
  <header>
    <h2>Application updates</h2>
    <button
      aria-label="Close updates"
      disabled={installing}
      onclick={() => void close()}>×</button
    >
  </header>
  <p>
    Updates are checked, downloaded and installed only when you ask. Nothing
    runs automatically.
  </p>
  {#if status}
    <p>Installed version: {status.currentVersion}</p>
    {#if status.phase === "current"}<p role="status">
        No newer stable version is available.
      </p>{/if}
    {#if status.version && status.phase !== "current"}
      <h3>Marktub {status.version}</h3>
      {#if status.notes}<pre class="update-notes">{status.notes}</pre>{/if}
    {/if}
    {#if !status.canInstall}<p>
        This build supports checking only. Install a released macOS app or
        Ubuntu DEB package to enable downloads and installation.
      </p>{/if}
    {#if status.phase === "checking"}<p role="status">
        Checking for updates…
      </p>{/if}
    {#if status.phase === "downloading"}
      <label
        >Downloading: {(status.received / 1024 / 1024).toFixed(1)} MB
        <progress
          value={status.total ? status.received : undefined}
          max={status.total ?? 1}
        ></progress>
      </label>
    {/if}
    {#if status.phase === "ready"}
      <p role="status">
        Download verified. Nothing has been installed by Marktub.
      </p>
      {#if linux}<p>
          The system installer will ask for approval. After it finishes, restart
          here to use the update. The package is kept in Downloads.
        </p>{/if}
    {/if}
    {#if error || status.error}<p class="error" role="alert">
        {error || status.error}
      </p>{/if}
    <footer class="dialog-actions">
      {#if busy}<button onclick={() => action("cancel_application_update")}
          >Cancel request</button
        >
      {:else}
        <button onclick={() => action("check_application_update")}
          >Check for updates</button
        >
        {#if status.phase === "available" && status.canInstall}<button
            onclick={() => action("download_application_update")}
            >Download update</button
          >{/if}
        {#if status.phase === "ready"}
          {#if linux}<button
              onclick={() => action("open_application_installer")}
              >Open system installer</button
            >{/if}
          <button disabled={linux && !handedOff} onclick={restart}
            >{linux
              ? "Restart after installation"
              : "Install and restart"}</button
          >
        {/if}
      {/if}
      <button onclick={() => void close()}>Close</button>
    </footer>
  {:else if error}<p role="alert">{error}</p>{/if}
</dialog>

<style>
  .updates-dialog {
    width: min(36rem, calc(100vw - 3rem));
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  h2 {
    margin: 0;
  }
  p {
    line-height: 1.5;
  }
  .update-notes {
    white-space: pre-wrap;
    font: inherit;
    max-height: 12rem;
    overflow: auto;
  }
  progress {
    display: block;
    width: 100%;
    margin-block: 0.75rem;
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
</style>
