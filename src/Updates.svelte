<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import appIcon from "../src-tauri/icons/app.svg";
  import { errorMessage } from "./bridge";

  let {
    active,
    restartDisabled,
    onrestart,
  }: {
    active: boolean;
    restartDisabled: boolean;
    onrestart: () => Promise<void>;
  } = $props();

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
  type Action =
    | "check_application_update"
    | "download_application_update"
    | "cancel_application_update"
    | "open_application_installer";

  let status = $state<Status | null>(null);
  let error = $state("");
  let actionInFlight = $state<Action | null>(null);
  let restarting = $state(false);
  let installerVersion = $state<string | null>(null);
  let generation = 0;
  let actionId = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const busy = $derived(
    status?.phase === "checking" ||
      status?.phase === "downloading" ||
      actionInFlight === "check_application_update" ||
      actionInFlight === "download_application_update",
  );
  const linux = $derived(status?.platform.startsWith("linux-") ?? false);
  const installing = $derived(status?.phase === "installing");
  const manualHandoff = $derived(
    status?.phase === "ready" && installerVersion === status.version,
  );

  function poll(request: number, delay: number) {
    clearTimeout(timer);
    timer = setTimeout(() => void refresh(request), delay);
  }

  async function refresh(request = generation) {
    try {
      const next = await invoke<Status>("application_update_status");
      if (request !== generation || !active) return;
      status = next;
      if (busy || installing || restarting) poll(request, 250);
      else clearTimeout(timer);
    } catch (e) {
      if (request === generation && active) error = errorMessage(e);
    }
  }

  $effect(() => {
    if (!active) return;
    const request = ++generation;
    void refresh(request);
    return () => {
      ++generation;
      clearTimeout(timer);
    };
  });

  async function action(command: Action) {
    const request = ++actionId;
    const version = status?.version ?? null;
    error = "";
    actionInFlight = command;
    if (command === "check_application_update") installerVersion = null;
    if (busy) poll(generation, 100);
    try {
      await invoke(command);
      if (request === actionId && command === "open_application_installer")
        installerVersion = version;
    } catch (e) {
      if (request === actionId) error = errorMessage(e);
    } finally {
      if (request === actionId) {
        actionInFlight = null;
        if (active) await refresh();
      }
    }
  }

  async function restart(installLinux = false) {
    restarting = true;
    error = "";
    try {
      if (installLinux) {
        poll(generation, 100);
        await invoke("apply_linux_application_update");
        await refresh();
      }
      await onrestart();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      restarting = false;
      if (active) await refresh();
    }
  }
</script>

{#if active}
  <section class="app-updates" aria-label="Application updates">
    <header class="app-identity">
      <img src={appIcon} alt="" width="56" height="56" />
      <div>
        <h3>Marktub</h3>
        {#if status}<p>Version {status.currentVersion}</p>{/if}
      </div>
    </header>
    <h4>Application updates</h4>
    {#if status}
      {#if status.phase === "current"}<p role="status">
          You're on the latest stable version.
        </p>{/if}
      {#if status.version && status.phase !== "current"}
        <p><strong>Marktub {status.version}</strong></p>
        {#if status.notes}<pre class="update-notes">{status.notes}</pre>{/if}
      {/if}
      {#if !status.canInstall}<p>
          Downloads and installation are available in released macOS apps and
          Linux DEB and RPM packages.
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
        <p role="status">Update downloaded and ready to install.</p>
        {#if linux}<p>
            Your system may ask you to authorize the installation.
          </p>{/if}
      {/if}
      {#if installing}<p role="status">
          Waiting for the system package installer…
        </p>{/if}
      {#if status.phase === "installed"}<p role="status">
          Update installed. Restart Marktub to use it.
        </p>{/if}
    {/if}
    {#if error || status?.error}<p class="error" role="alert">
        {error || status?.error}
      </p>{/if}
    <div class="update-actions">
      {#if busy}
        <button
          disabled={actionInFlight === "cancel_application_update"}
          onclick={() => action("cancel_application_update")}
          >Cancel request</button
        >
      {:else}
        {#if status?.phase !== "installed"}
          <button
            disabled={!!actionInFlight || restarting || installing}
            onclick={() => action("check_application_update")}
            >Check for updates</button
          >
        {/if}
        {#if status?.phase === "available" && status.canInstall}<button
            disabled={!!actionInFlight}
            onclick={() => action("download_application_update")}
            >Download update</button
          >{/if}
        {#if status?.phase === "ready" || status?.phase === "installed"}
          <button
            class="primary"
            disabled={!!actionInFlight || restarting || restartDisabled}
            onclick={() =>
              restart(linux && status?.phase === "ready" && !manualHandoff)}
            >{status.phase === "installed"
              ? "Restart Marktub"
              : manualHandoff
                ? "Restart after installation"
                : "Install and restart"}</button
          >
          {#if linux && status.phase === "ready" && (status.error || manualHandoff)}
            <button
              disabled={!!actionInFlight || restarting}
              onclick={() => action("open_application_installer")}
              >Open system installer</button
            >
          {/if}
        {/if}
      {/if}
    </div>
  </section>
{/if}

<style>
  .app-updates {
    min-width: 0;
  }
  .app-identity {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .app-identity h3,
  .app-identity p {
    margin: 0;
  }
  p {
    line-height: 1.5;
  }
  h4 {
    margin-block: 1.5rem 0.75rem;
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
  .update-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .update-actions button {
    border-color: var(--line);
  }
</style>
