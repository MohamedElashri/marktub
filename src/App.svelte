<script lang="ts">
  import { onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import type GitWorkbench from "./git/Workbench.svelte";
  import type Preferences from "./Preferences.svelte";
  import type CommandPalette from "./CommandPalette.svelte";
  import {
    definitions,
    matches,
    binding,
    shortcutLabel,
    type Command,
  } from "./commands";
  import BusyStatus from "./ui/BusyStatus.svelte";
  import type { Values } from "./preferences";
  import type ReviewArchive from "./review/Archive.svelte";
  import DocumentPane from "./DocumentPane.svelte";
  import type Library from "./Library.svelte";
  import * as bridge from "./bridge";
  import * as library from "./library";
  import { sessionSelection } from "./tabs";
  type Tab = {
    key: number;
    name: string;
    dirty: boolean;
    document: string | null;
    location?: string | null;
    pane?: DocumentPane;
  };
  let tabs = $state<Tab[]>([
    { key: 0, name: "New tab", dirty: false, document: null },
  ]);
  let active = $state(0),
    serial = 0;
  let PreferencesComponent = $state<typeof Preferences | null>(null),
    PaletteComponent = $state<typeof CommandPalette | null>(null);
  let GitComponent = $state<typeof GitWorkbench | null>(null),
    gitPanel = $state<GitWorkbench>();
  let ReviewArchiveComponent = $state<typeof ReviewArchive | null>(null),
    reviewArchive = $state<ReviewArchive>();
  async function showReviewArchive() {
    ReviewArchiveComponent ??= (await import("./review/Archive.svelte"))
      .default;
    await tick();
    await reviewArchive?.show();
  }
  async function showGit() {
    GitComponent ??= (await import("./git/Workbench.svelte")).default;
    await tick();
    await gitPanel?.show();
  }
  let preferencesPanel = $state<Preferences>(),
    palettePanel = $state<CommandPalette>();
  let previewValues = $state<Values | null>(null);
  const currentValues = $derived(tabs[active]?.pane?.preferences() ?? {});
  const commands = $derived.by(() => {
    const state = tabs[active]?.pane?.commandState();
    return definitions.map(([id, label, group, defaultBinding]) => {
      const hasDocument = !!state?.document;
      const read = hasDocument && ["read", "split"].includes(state?.mode ?? "");
      let enabled = !state?.busy;
      let reason = "Wait for the current operation";
      if (["save", "save_as"].includes(id) && !state?.editing) {
        enabled = false;
        reason = "Start Source first";
      }
      if (
        [
          "power",
          "source",
          "read",
          "split",
          "visual",
          "reload",
          "compare",
          "review",
        ].includes(id) &&
        !hasDocument
      ) {
        enabled = false;
        reason = "Open a document first";
      }
      if (
        (id === "outline" && !read) ||
        (["focus", "find"].includes(id) && !read && !state?.editing)
      ) {
        enabled = false;
        reason = "Available in Read mode";
      }
      if (["split", "visual"].includes(id) && !state?.authorBudget) {
        enabled = false;
        reason = "Split and Visual admit ordinary documents up to 2 MiB";
      }
      if (id === "new_tab" && tabs.length >= 4) {
        enabled = false;
        reason = "Four-tab limit reached";
      }
      if (["next_tab", "previous_tab"].includes(id) && tabs.length < 2) {
        enabled = false;
        reason = "Open another tab";
      }
      if (
        (id === "back" && historyAt <= 0) ||
        (id === "forward" && historyAt + 1 >= history.length)
      ) {
        enabled = false;
        reason = "No history in that direction";
      }
      return {
        id,
        label,
        group,
        default: defaultBinding,
        enabled,
        reason,
        run: () => runCommand(id),
      } as Command;
    });
  });
  async function refreshPreferences() {
    await Promise.all(tabs.map((t) => t.pane?.refreshPreferences()));
  }
  async function showPreferences() {
    PreferencesComponent ??= (await import("./Preferences.svelte")).default;
    await tick();
    await preferencesPanel?.show(
      tabs[active]?.document === "recovery"
        ? null
        : (tabs[active]?.document ?? null),
    );
  }
  async function showPalette() {
    PaletteComponent ??= (await import("./CommandPalette.svelte")).default;
    await tick();
    await palettePanel?.show();
  }
  async function runCommand(id: string) {
    try {
      switch (id) {
        case "review_records":
          return showReviewArchive();
        case "git":
          return showGit();
        case "preferences":
          return showPreferences();
        case "palette":
          return showPalette();
        case "quick_open":
        case "search_workspaces":
          return showLibrary(true);
        case "browser":
          if (sidebar) {
            sidebar = false;
            return library.configure(false, panel?.viewName() ?? "list");
          }
          return showLibrary();
        case "new_tab":
          if (tabs.length < 4) {
            tabs.push({
              key: ++serial,
              name: "New tab",
              dirty: false,
              document: null,
            });
            active = tabs.length - 1;
          }
          return;
        case "close_tab":
          return closeTab(active);
        case "next_tab":
          active = (active + 1) % tabs.length;
          return;
        case "previous_tab":
          active = (active + tabs.length - 1) % tabs.length;
          return;
        case "back":
          return navigate(-1);
        case "forward":
          return navigate(1);
        default:
          return tabs[active]?.pane?.command(id);
      }
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  let sidebar = $state(false);
  let LibraryComponent = $state<typeof Library | null>(null);
  let panel = $state<Library>();
  let message = $state("");
  let recentFiles = $state<library.Hint[]>([]);
  let folderBusy = $state(false);
  let history = $state<library.Hint[]>([]),
    historyAt = $state(-1);
  let closing = false;
  let sessionTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const selection = sessionSelection(
      tabs.map((tab) => tab.document),
      active,
    );
    clearTimeout(sessionTimer);
    if (selection.ids.length && !closing)
      sessionTimer = setTimeout(() => {
        void library
          .session(selection.ids, selection.active)
          .catch((error) => (message = bridge.errorMessage(error)));
      }, 400);
    return () => clearTimeout(sessionTimer);
  });
  async function showLibrary(quick = false) {
    LibraryComponent ??= (await import("./Library.svelte")).default;
    sidebar = true;
    tabs[active]?.pane?.revealNavigation();
    await tick();
    if (quick) panel?.quickOpen();
    await library
      .configure(true, (await library.state()).view)
      .catch((e) => (message = bridge.errorMessage(e)));
  }
  async function chooseFolder() {
    if (folderBusy) return;
    folderBusy = true;
    try {
      await showLibrary();
      await panel?.chooseFolder();
      recentFiles = (await library.state()).recent.slice(0, 6);
    } catch (error) {
      message = bridge.errorMessage(error);
    } finally {
      folderBusy = false;
    }
  }
  const openingTabs = new SvelteSet<number>();
  async function openTab(descriptor: bridge.Descriptor, link?: string) {
    const existing = tabs.findIndex(
      (t) =>
        t.document === descriptor.id ||
        (!!descriptor.location && t.location === descriptor.location),
    );
    if (existing >= 0) {
      if (tabs[existing].document !== descriptor.id)
        await bridge.releaseDocument(descriptor.id);
      active = existing;
      return;
    }
    const available = (tab: Tab) =>
      !tab.document &&
      closingTab !== tab.key &&
      !openingTabs.has(tab.key) &&
      !tab.pane?.commandState().busy;
    let tab = available(tabs[active])
      ? tabs[active]
      : (tabs.find(available) ?? null);
    if (!tab) {
      if (tabs.length >= 4) {
        await bridge.releaseDocument(descriptor.id);
        message = "Four tabs are open. Close one before opening another.";
        return;
      }
      tab = {
        key: ++serial,
        name: descriptor.name,
        dirty: false,
        document: descriptor.id,
      };
      tabs.push(tab);
    }
    const key = tab.key;
    openingTabs.add(key);
    active = tabs.findIndex((t) => t.key === key);
    let adopted = false;
    try {
      await tick();
      const pane = tabs.find((tab) => tab.key === key)?.pane;
      if (!pane)
        throw Error("The target tab closed before the document opened.");
      await pane.load(descriptor, link);
      adopted = true;
      await tick();
      await saveSession();
    } catch (error) {
      if (!adopted) await bridge.releaseDocument(descriptor.id);
      message = bridge.errorMessage(error);
    } finally {
      openingTabs.delete(key);
    }
  }
  async function saveSession() {
    const selection = sessionSelection(
      tabs.map((tab) => tab.document),
      active,
    );
    await library.session(selection.ids, selection.active);
  }
  async function restoreSession(hints: library.Hint[], index: number) {
    if (tabs.some((t) => t.document)) {
      message =
        "Close current document tabs before restoring the previous session.";
      return;
    }
    if (hints.some((h) => !h.authorized)) {
      message = "Reselect every containing folder before restoring these tabs.";
      return;
    }
    for (let i = 0; i < tabs.length; i++) {
      active = i;
      await tick();
      if (!(await tabs[i].pane?.requestClose())) {
        message = "Wait for the current file operation before restoring tabs.";
        return;
      }
    }
    tabs = [{ key: ++serial, name: "New tab", dirty: false, document: null }];
    active = 0;
    await tick();
    try {
      for (const hint of hints)
        await openTab(await library.open(hint.id, true));
      active = Math.max(0, Math.min(index, tabs.length - 1));
      await saveSession();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  let closingTab = $state<number | null>(null);
  async function closeTab(index: number) {
    const tab = tabs[index];
    if (!tab || closingTab !== null) return;
    if (openingTabs.has(tab.key)) {
      message =
        "Wait for this document to finish opening before closing its tab.";
      return;
    }
    closingTab = tab.key;
    const previous = tabs[active]?.key;
    let removed = false;
    try {
      active = index;
      if (!(await tab.pane?.requestClose())) return;
      const preserve =
        tabs[active]?.key === tab.key ? previous : tabs[active]?.key;
      const at = tabs.findIndex((t) => t.key === tab.key);
      if (at < 0) return;
      tabs.splice(at, 1);
      removed = true;
      if (!tabs.length)
        tabs.push({
          key: ++serial,
          name: "New tab",
          dirty: false,
          document: null,
        });
      const preserved = tabs.findIndex((tab) => tab.key === preserve);
      active = preserved >= 0 ? preserved : Math.min(at, tabs.length - 1);
      await saveSession().catch(
        (error) => (message = bridge.errorMessage(error)),
      );
    } finally {
      if (!removed && tabs[active]?.key === tab.key && previous !== tab.key) {
        const prior = tabs.findIndex((tab) => tab.key === previous);
        if (prior >= 0) active = prior;
      }
      closingTab = null;
    }
  }
  async function closeAll() {
    if (closing) return;
    closing = true;
    try {
      if ((await library.state()).job) {
        message =
          "Cancel the library job and wait for its result before quitting.";
        await showLibrary();
        return;
      }
      await saveSession();
      for (let i = 0; i < tabs.length; i++) {
        active = i;
        await tick();
        if (!(await tabs[i].pane?.requestClose())) return;
      }
      await bridge.closeApplication();
    } catch (error) {
      message = bridge.errorMessage(error);
    } finally {
      closing = false;
    }
  }
  const suppressed = new SvelteSet<string>();
  let navigationQueue = Promise.resolve();
  let nativeOpenQueue = Promise.resolve();
  let nativeOpening = $state(false);
  function receiveNativeOpen() {
    nativeOpenQueue = nativeOpenQueue
      .then(async () => {
        nativeOpening = true;
        const results = await bridge.takeOsOpen();
        const errors: string[] = [];
        for (const result of results) {
          if (result.error) errors.push(result.error);
          if (result.document) await openTab(result.document);
        }
        if (errors.length) message = errors.join(" ");
      })
      .catch((error) => {
        message = bridge.errorMessage(error);
      })
      .finally(() => {
        nativeOpening = false;
      });
  }
  function recordNavigation(id: string) {
    if (suppressed.delete(id)) return;
    navigationQueue = navigationQueue
      .then(async () => {
        if (!tabs.some((tab) => tab.document === id)) return;
        await bridge.rememberDocument(id);
        const state = await library.state();
        recentFiles = state.recent.slice(0, 6);
        const hint = state.recent[0];
        if (!hint) return;
        if (history[historyAt]?.id === hint.id) {
          history[historyAt] = hint;
          return;
        }
        history = history.slice(0, historyAt + 1);
        history.push(hint);
        if (history.length > 100) history.shift();
        historyAt = history.length - 1;
      })
      .catch((error) => {
        if (tabs.some((tab) => tab.document === id))
          message = bridge.errorMessage(error);
      });
  }
  async function recent(hint: library.Hint, record = true) {
    try {
      const descriptor = await library.open(hint.id, true);
      suppressed.add(descriptor.id);
      await openTab(descriptor);
      suppressed.delete(descriptor.id);
      if (record && history[historyAt]?.id !== hint.id) {
        history = history.slice(0, historyAt + 1);
        history.push(hint);
        historyAt = history.length - 1;
      }
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  async function navigate(delta: number) {
    const next = historyAt + delta;
    if (next < 0 || next >= history.length) return;
    historyAt = next;
    await recent(history[next], false);
  }
  onMount(() => {
    const osOpen = listen("os-open-available", receiveNativeOpen);
    void osOpen
      .then(receiveNativeOpen)
      .catch((e) => (message = bridge.errorMessage(e)));
    const stop = listen("close-requested", () => void closeAll());
    const drop = listen<library.DropOffer[]>("library-drop", async (event) => {
      if (tabs[active]?.pane?.droppedImage(event.payload)) return;
      await showLibrary();
      panel?.dropped(event.payload);
    });
    const keys = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      const modal =
        window.document.querySelector<HTMLDialogElement>("dialog[open]");
      if (modal) {
        if (
          modal.getAttribute("aria-label") === "Preferences" &&
          matches(event, "Mod+S") &&
          !(event.target instanceof HTMLInputElement && event.target.readOnly)
        ) {
          event.preventDefault();
          void preferencesPanel?.apply();
        }
        return;
      }
      const command = commands.find((command) =>
        matches(event, binding(command, currentValues)),
      );
      if (command) {
        event.preventDefault();
        event.stopPropagation();
        if (command.enabled) void command.run();
        else message = command.reason ?? "Unavailable in this context";
      }
    };
    window.addEventListener("keydown", keys, true);
    void library
      .state()
      .then((s) => {
        recentFiles = s.recent.slice(0, 6);
        if (s.sidebar) void showLibrary();
      })
      .catch((e) => (message = bridge.errorMessage(e)));
    return () => {
      void stop.then((fn) => fn());
      void osOpen.then((fn) => fn());
      void drop.then((fn) => fn());
      window.removeEventListener("keydown", keys, true);
    };
  });
</script>

<svelte:head
  ><title>{tabs[active]?.name ?? "Marktub"} — Marktub</title></svelte:head
>
<div class="workbench" class:has-library={sidebar}>
  <nav class="tabbar" aria-label="Open documents">
    <button
      aria-expanded={sidebar}
      onclick={() => {
        if (sidebar) {
          sidebar = false;
          void library
            .configure(false, panel?.viewName() ?? "list")
            .catch((e) => (message = bridge.errorMessage(e)));
        } else void showLibrary();
      }}>Library</button
    >
    <button
      aria-label="Back in file history"
      disabled={historyAt <= 0}
      onclick={() => navigate(-1)}>‹</button
    >
    <button
      aria-label="Forward in file history"
      disabled={historyAt + 1 >= history.length}
      onclick={() => navigate(1)}>›</button
    >
    <div class="tabs" role="tablist" aria-label="Document tabs">
      {#each tabs as tab, index (tab.key)}<div class="tab">
          <button
            role="tab"
            id={`document-tab-${tab.key}`}
            aria-controls={`document-panel-${tab.key}`}
            tabindex={index === active ? 0 : -1}
            aria-selected={index === active}
            title={`${tab.name}${tab.dirty ? " — unsaved changes" : ""}`}
            onkeydown={async (event) => {
              if (
                !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)
              )
                return;
              event.preventDefault();
              active =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? tabs.length - 1
                    : (index +
                        (event.key === "ArrowRight" ? 1 : -1) +
                        tabs.length) %
                      tabs.length;
              await tick();
              window.document
                .getElementById(`document-tab-${tabs[active].key}`)
                ?.focus();
            }}
            onclick={() => (active = index)}
            >{tab.name}{tab.dirty ? " *" : ""}</button
          ><button
            aria-label={`Close ${tab.name}`}
            disabled={closingTab !== null ||
              openingTabs.has(tab.key) ||
              tab.pane?.commandState().busy}
            onclick={() => closeTab(index)}>×</button
          >
        </div>{/each}
    </div>
    <button
      aria-label="New tab"
      disabled={tabs.length >= 4}
      onclick={() => {
        tabs.push({
          key: ++serial,
          name: "New tab",
          dirty: false,
          document: null,
        });
        active = tabs.length - 1;
      }}>+</button
    >
    <button onclick={showGit}>Git</button>
    <button onclick={showPalette}>Commands</button>
    <button onclick={() => showLibrary(true)}
      >Quick open <kbd
        aria-label={shortcutLabel(
          String(currentValues["shortcut.quick_open"]?.value ?? "Mod+P"),
        ).spoken}
        >{shortcutLabel(
          String(currentValues["shortcut.quick_open"]?.value ?? "Mod+P"),
        ).visual}</kbd
      ></button
    >
  </nav>
  <BusyStatus
    active={nativeOpening && !tabs[active]?.pane?.commandState().busy}
    label="Opening file…"
  />
  {#if message}<div class="notice" role="status">
      {message}<button
        aria-label="Dismiss workbench message"
        onclick={() => (message = "")}>×</button
      >
    </div>{/if}
  <div class="workbench-body">
    {#if LibraryComponent}<aside class="library-host" hidden={!sidebar}>
        <LibraryComponent
          bind:this={panel}
          onopen={openTab}
          onrecent={recent}
          onrestore={restoreSession}
          onrecentchange={(hints) => (recentFiles = hints.slice(0, 6))}
        />
      </aside>{/if}
    <div class="document-host">
      {#each tabs as tab, index (tab.key)}<div
          class="document-tab"
          id={`document-panel-${tab.key}`}
          role="tabpanel"
          aria-labelledby={`document-tab-${tab.key}`}
          hidden={index !== active}
        >
          <DocumentPane
            bind:this={tab.pane}
            ongit={showGit}
            onarchive={showReviewArchive}
            {recentFiles}
            {folderBusy}
            onrecent={async (hint) => {
              if (hint.authorized) await recent(hint);
              else await chooseFolder();
            }}
            onfolder={chooseFolder}
            active={index === active}
            onpreferences={showPreferences}
            previewSettings={index === active ? previewValues : null}
            onstate={(name, dirty, id, location) => {
              if (id && id !== "recovery" && tab.document !== id)
                recordNavigation(id);
              tab.location = location;
              tab.name = name;
              tab.dirty = dirty;
              tab.document = id;
            }}
          />
        </div>{/each}
    </div>
  </div>
</div>

{#if PreferencesComponent}<PreferencesComponent
    bind:this={preferencesPanel}
    onpreview={(values) => (previewValues = values)}
    oncommit={refreshPreferences}
  />{/if}
{#if PaletteComponent}<PaletteComponent
    bind:this={palettePanel}
    {commands}
    values={currentValues}
    onrun={(command) => {
      void command.run();
    }}
  />{/if}

{#if GitComponent}<GitComponent
    bind:this={gitPanel}
    editing={tabs.some((t) => t.pane?.commandState().editing)}
    onopen={async (descriptor, line) => {
      await openTab(descriptor);
      await tabs[active]?.pane?.sourceLineAt(line);
    }}
  />{/if}

{#if ReviewArchiveComponent}<ReviewArchiveComponent
    bind:this={reviewArchive}
  />{/if}
