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
  import Icon from "./ui/Icon.svelte";
  import NewNote from "./NewNote.svelte";
  import TabBar from "./TabBar.svelte";
  import ResizeHandle from "./ui/ResizeHandle.svelte";
  import { libraryWidth, librarySize } from "./workspace";
  import { literals, put } from "./preferences";
  import type { Values } from "./preferences";
  import type ReviewArchive from "./review/Archive.svelte";
  import DocumentPane from "./DocumentPane.svelte";
  import type Library from "./Library.svelte";
  import * as bridge from "./bridge";
  import * as library from "./library";
  import { sessionSelection, moveTab, TAB_LIMIT } from "./tabs";
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
  const readingFocus = $derived(
    tabs[active]?.pane?.commandState().readingFocus ?? false,
  );
  function emptyTabAvailable(tab: Tab) {
    return (
      !tab.document &&
      !closingTabs.includes(tab.key) &&
      !openingTabs.has(tab.key) &&
      !tab.pane?.commandState().busy
    );
  }
  const canCreateNote = $derived(
    tabs.length < TAB_LIMIT || tabs.some(emptyTabAvailable),
  );
  const newNoteShortcut = $derived(
    shortcutLabel(String(currentValues["shortcut.new_note"]?.value ?? "Mod+N")),
  );
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
      if (id === "new_tab" && tabs.length >= TAB_LIMIT) {
        enabled = false;
        reason = `${TAB_LIMIT}-tab limit reached`;
      }
      if (["new_note", "note_template"].includes(id) && !canCreateNote) {
        enabled = false;
        reason = `All ${TAB_LIMIT} tabs are in use. Close a tab first.`;
      }
      if (
        ["move_tab_left", "move_tab_right"].includes(id) &&
        (closingTabs.length > 0 ||
          (id === "move_tab_left" ? active === 0 : active === tabs.length - 1))
      ) {
        enabled = false;
        reason = "The tab is already at this edge, or is closing";
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
  async function showPreferences(category = "Appearance") {
    await layoutWrite;
    PreferencesComponent ??= (await import("./Preferences.svelte")).default;
    await tick();
    await preferencesPanel?.show(
      tabs[active]?.document === "recovery"
        ? null
        : (tabs[active]?.document ?? null),
      category,
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
        case "updates":
          return showPreferences("About");
        case "preferences":
          return showPreferences();
        case "palette":
          return showPalette();
        case "quick_open":
          return showLibrary(true);
        case "search_workspaces":
          return showLibrary(true, true);
        case "browser":
          if (readingFocus) return showLibrary();
          if (sidebar) return hideLibrary(libraryDrawer);
          return showLibrary();
        case "example":
          return openTab(await bridge.openExample());
        case "new_note":
          return createNote("Untitled", "blank");
        case "note_template":
          return templatePanel.show();
        case "new_tab":
          return newTab();
        case "move_tab_left":
        case "move_tab_right":
          return reorderTab(
            tabs[active].key,
            active + (id === "move_tab_left" ? -1 : 1),
          );
        case "close_tab":
          return closeTabs([tabs[active].key]);
        case "next_tab":
          return selectTab((active + 1) % tabs.length);
        case "previous_tab":
          return selectTab((active + tabs.length - 1) % tabs.length);
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
  function newTab() {
    if (closingTabs.length || tabs.length >= TAB_LIMIT) return;
    tabs.push({ key: ++serial, name: "New tab", dirty: false, document: null });
    selectTab(tabs.length - 1);
  }
  let templatePanel: NewNote;
  async function createNote(title: string, template: string) {
    let tab = emptyTabAvailable(tabs[active])
      ? tabs[active]
      : tabs.find(emptyTabAvailable);
    if (!tab) {
      if (tabs.length >= TAB_LIMIT)
        throw Error("Close a tab before creating another note.");
      newTab();
      tab = tabs[active];
    }
    openingTabs.add(tab.key);
    active = tabs.indexOf(tab);
    try {
      await tick();
      await tab.pane!.createNote(title, template);
      await revealDocumentOnNarrowScreens();
    } finally {
      openingTabs.delete(tab.key);
    }
  }
  let tabAnnouncement = $state("");
  function reorderTab(key: number, destination: number) {
    if (closingTabs.length > 0) return;
    const moved = moveTab(tabs, active, key, destination);
    if (moved.tabs === tabs) return;
    tabs = moved.tabs;
    active = moved.active;
    const index = tabs.findIndex((tab) => tab.key === key);
    tabAnnouncement = `${tabs[index].name} moved to position ${index + 1} of ${tabs.length}.`;
  }
  let viewportWidth = $state(1024),
    draftWidth = $state<number | null>(null);
  const sidebarWidth = $derived(
    libraryWidth(
      Number(
        previewValues?.["library.width"]?.value ??
          draftWidth ??
          currentValues["library.width"]?.value ??
          librarySize.default,
      ),
      viewportWidth,
    ),
  );

  let layoutWrite = Promise.resolve(),
    layoutRevision = 0;
  function resizeLibrary(width: number) {
    const next = libraryWidth(width, viewportWidth),
      revision = ++layoutRevision;
    draftWidth = next;
    let saved = false;
    layoutWrite = layoutWrite
      .then(async () => {
        const session = await bridge.getSettings(null, "session");
        const scope =
          literals(session.text)["library.width"] === undefined
            ? "global"
            : "session";
        const previous =
          scope === "session"
            ? session
            : await bridge.getSettings(null, "global");
        const result = await bridge.updateSettings(
          null,
          scope,
          put(previous.text, "library.width", String(next)),
        );
        if (result.diagnostics.length)
          throw Error(result.diagnostics.map((d) => d.message).join(" "));
        await refreshPreferences();
        saved = true;
      })
      .catch((error) => {
        message = `Library resized for this session. Could not save its width: ${bridge.errorMessage(error)}`;
      })
      .finally(() => {
        if (revision === layoutRevision) {
          if (saved) draftWidth = null;
        }
      });
  }
  let sidebar = $state(false);
  const libraryDrawer = $derived(
    sidebar && !readingFocus && viewportWidth <= 700,
  );
  let libraryToggle = $state<HTMLButtonElement>();
  let libraryHost = $state<HTMLElement>();
  $effect(() => {
    if (!libraryDrawer || !libraryHost) return;
    const host = libraryHost;
    void tick().then(() => {
      if (
        libraryDrawer &&
        !host.contains(document.activeElement) &&
        !document.querySelector("dialog[open]")
      )
        host.querySelector<HTMLButtonElement>(".library-close")?.focus();
    });
  });
  let LibraryComponent = $state<typeof Library | null>(null);
  let panel = $state<Library>();
  let message = $state("");
  let recentInfo = $state<
    Pick<library.LibraryState, "recent" | "canClearRecent">
  >({
    recent: [],
    canClearRecent: false,
  });
  const recentFiles = $derived(recentInfo.recent.slice(0, 6));
  function showRecents({ recent, canClearRecent }: typeof recentInfo) {
    recentInfo = { recent, canClearRecent };
  }
  async function refreshRecents() {
    showRecents(await library.state());
  }
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
    if (selection.ids.length && !closing && !closingTabs.length)
      sessionTimer = setTimeout(() => {
        void library
          .session(selection.ids, selection.active)
          .catch((error) => (message = bridge.errorMessage(error)));
      }, 400);
    return () => clearTimeout(sessionTimer);
  });
  async function showLibrary(quick = false, content = false) {
    LibraryComponent ??= (await import("./Library.svelte")).default;
    sidebar = true;
    tabs[active]?.pane?.revealNavigation();
    await tick();
    if (quick) await panel?.quickOpen(content);
    const state = await library.state();
    showRecents(state);
    if (sidebar)
      await library
        .configure(true, state.view)
        .catch((e) => (message = bridge.errorMessage(e)));
  }
  async function chooseFolder() {
    if (folderBusy) return;
    folderBusy = true;
    try {
      await showLibrary();
      await panel?.chooseFolder();
      await refreshRecents();
    } catch (error) {
      message = bridge.errorMessage(error);
    } finally {
      folderBusy = false;
    }
  }
  async function hideLibrary(restoreFocus = false) {
    sidebar = false;
    const persist = library
      .configure(false, panel?.viewName() ?? "list")
      .catch((error) => (message = bridge.errorMessage(error)));
    if (restoreFocus) {
      await tick();
      if (!sidebar) libraryToggle?.focus({ preventScroll: true });
    }
    await persist;
  }
  function selectTab(index: number) {
    if (libraryDrawer) void hideLibrary();
    active = index;
  }
  async function revealDocumentOnNarrowScreens() {
    if (libraryDrawer) await hideLibrary();
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
      if (link?.includes("#")) await tabs[existing].pane?.revealLink(link);
      await revealDocumentOnNarrowScreens();
      return;
    }
    let tab = emptyTabAvailable(tabs[active])
      ? tabs[active]
      : (tabs.find(emptyTabAvailable) ?? null);
    if (!tab) {
      if (tabs.length >= TAB_LIMIT) {
        await bridge.releaseDocument(descriptor.id);
        message = `${TAB_LIMIT} tabs are open. Close one before opening another.`;
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
      await revealDocumentOnNarrowScreens();
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
    if (hints.length > TAB_LIMIT) {
      message = `Restore up to ${TAB_LIMIT} tabs at a time.`;
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
  let closingTabs = $state<number[]>([]);
  async function closeTabs(keys: number[]) {
    if (!keys.length || closingTabs.length || closing) return;
    if (
      tabs.some(
        (tab) =>
          keys.includes(tab.key) &&
          (openingTabs.has(tab.key) || tab.pane?.commandState().busy),
      )
    ) {
      message = "Wait for the current file operation before closing tabs.";
      return;
    }
    closingTabs = keys;
    let removed = false;
    try {
      for (const key of keys) {
        const index = tabs.findIndex((tab) => tab.key === key);
        const tab = tabs[index];
        if (!tab) continue;
        const previous = tabs[active]?.key;
        active = index;
        await tick();
        if (!(await tab.pane?.requestClose())) {
          const prior = tabs.findIndex((entry) => entry.key === previous);
          if (tabs[active]?.key === key && prior >= 0) active = prior;
          break;
        }
        const preserve =
          tabs[active]?.key === key ? previous : tabs[active]?.key;
        const at = tabs.findIndex((entry) => entry.key === key);
        if (at < 0) continue;
        tabs.splice(at, 1);
        removed = true;
        if (!tabs.length)
          tabs.push({
            key: ++serial,
            name: "New tab",
            dirty: false,
            document: null,
          });
        const preserved = tabs.findIndex((entry) => entry.key === preserve);
        active = preserved >= 0 ? preserved : Math.min(at, tabs.length - 1);
      }
    } finally {
      if (removed)
        await saveSession().catch(
          (error) => (message = bridge.errorMessage(error)),
        );
      closingTabs = [];
    }
  }
  async function closeAll(update = false): Promise<boolean> {
    if (closing || closingTabs.length || nativeOpening || openingTabs.size)
      return false;
    closing = true;
    try {
      if (update) await bridge.prepareApplicationUpdate();
      if ((await library.state()).job) {
        message =
          "Cancel the library job and wait for its result before quitting.";
        await showLibrary();
        return false;
      }
      await saveSession();
      for (let i = 0; i < tabs.length; i++) {
        active = i;
        await tick();
        if (!(await tabs[i].pane?.requestClose())) return false;
      }
      if (update) await bridge.installApplicationUpdate();
      else await bridge.closeApplication();
      return true;
    } catch (error) {
      message = bridge.errorMessage(error);
      if (update) throw error;
      return false;
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
        const record = await bridge.rememberDocument(id);
        const state = await library.state();
        showRecents(state);
        const hint = state.recent.find((entry) => entry.id === record);
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
  async function clearRecent() {
    const clear = navigationQueue.then(async () => {
      await library.clearRecent();
      history = [];
      historyAt = -1;
      await refreshRecents();
    });
    navigationQueue = clear.catch(() => {});
    await clear;
  }
  async function recent(hint: library.Hint, record = true) {
    try {
      const descriptor = await library.open(hint.id, true);
      suppressed.add(descriptor.id);
      await openTab(descriptor);
      suppressed.delete(descriptor.id);
      await refreshRecents().catch(() => {});
      if (record && history[historyAt]?.id !== hint.id) {
        history = history.slice(0, historyAt + 1);
        history.push(hint);
        historyAt = history.length - 1;
      }
    } catch (error) {
      message = bridge.errorMessage(error);
      await refreshRecents().catch(() => {});
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
      if (event.key === "Escape" && libraryDrawer) {
        event.preventDefault();
        event.stopPropagation();
        void hideLibrary(true);
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
    const focus = () => void refreshRecents().catch(() => {});
    window.addEventListener("focus", focus);
    void library
      .state()
      .then((s) => {
        showRecents(s);
        if (s.sidebar) void showLibrary();
      })
      .catch((e) => (message = bridge.errorMessage(e)));
    return () => {
      void stop.then((fn) => fn());
      void osOpen.then((fn) => fn());
      void drop.then((fn) => fn());
      window.removeEventListener("keydown", keys, true);
      window.removeEventListener("focus", focus);
    };
  });
</script>

<svelte:head
  ><title>{tabs[active]?.name ?? "Marktub"} — Marktub</title></svelte:head
>
<svelte:window bind:innerWidth={viewportWidth} />
<div
  class="workbench"
  class:has-library={sidebar && !readingFocus}
  class:library-drawer-open={libraryDrawer}
  class:reading-focus={readingFocus}
  class:macos={navigator.platform.startsWith("Mac")}
  style={`--library-width:${sidebarWidth}px`}
>
  <header class="workbench-header" data-tauri-drag-region>
    <nav class="workspace-actions" aria-label="Workspace">
      <button
        class="library-toggle"
        bind:this={libraryToggle}
        aria-expanded={sidebar}
        aria-controls="library-sidebar"
        title="Show or hide library"
        onclick={() => {
          void runCommand("browser");
        }}
        ><Icon name="folder" /><span class="workspace-label">Library</span
        ></button
      >
      <button
        disabled={!canCreateNote}
        title={canCreateNote
          ? `New note${newNoteShortcut.visual ? ` · ${newNoteShortcut.visual}` : ""}`
          : "Close a tab before creating another note."}
        onclick={() => runCommand("new_note")}
        ><Icon name="new-note" /><span class="workspace-label">New note</span
        ></button
      >
      <div class="history-actions">
        <button
          aria-label="Back in file history"
          title="Back"
          disabled={historyAt <= 0}
          onclick={() => navigate(-1)}>‹</button
        >
        <button
          aria-label="Forward in file history"
          title="Forward"
          disabled={historyAt + 1 >= history.length}
          onclick={() => navigate(1)}>›</button
        >
      </div>
      <button
        class="quick-open-button"
        title="Quick open"
        onclick={() => showLibrary(true)}
        ><Icon name="find" /><span>Quick open</span><kbd
          aria-label={shortcutLabel(
            String(currentValues["shortcut.quick_open"]?.value ?? "Mod+P"),
          ).spoken}
          >{shortcutLabel(
            String(currentValues["shortcut.quick_open"]?.value ?? "Mod+P"),
          ).visual}</kbd
        ></button
      >
      <div class="workspace-tools">
        <button title="Git workbench" onclick={showGit}
          ><Icon name="git" /><span class="workspace-label">Git</span></button
        >
        <button title="Command palette" onclick={showPalette}
          ><Icon name="commands" /><span class="workspace-label">Commands</span
          ></button
        >
        <button
          aria-label="Open feature guide"
          title="Built-in feature guide"
          onclick={() => runCommand("example")}
          ><Icon name="book" /><span>Guide</span></button
        >
        <button
          class="settings-button"
          aria-label="Settings"
          title={`Settings · ${shortcutLabel(String(currentValues["shortcut.preferences"]?.value ?? "Mod+Comma")).visual}`}
          onclick={() => showPreferences()}
          ><Icon name="settings" /><span>Settings</span></button
        >
      </div>
    </nav>
    <TabBar
      tabs={tabs.map((tab) => ({
        key: tab.key,
        name: tab.name,
        dirty: tab.dirty,
        busy: openingTabs.has(tab.key) || !!tab.pane?.commandState().busy,
      }))}
      {active}
      locked={closingTabs.length > 0}
      onselect={selectTab}
      onclose={closeTabs}
      onmove={reorderTab}
      onnew={newTab}
    />
  </header>
  <span class="sr-only" aria-live="polite">{tabAnnouncement}</span>
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
    {#if libraryDrawer}
      <button
        class="library-scrim"
        aria-label="Dismiss library"
        tabindex="-1"
        onclick={() => hideLibrary(true)}
      ></button>
    {/if}
    {#if LibraryComponent}<aside
        id="library-sidebar"
        class="library-host"
        aria-label="Library"
        bind:this={libraryHost}
        hidden={!sidebar || readingFocus}
      >
        <LibraryComponent
          bind:this={panel}
          onopen={openTab}
          onrecent={recent}
          onrestore={restoreSession}
          recentFiles={recentInfo.recent}
          canClearRecent={recentInfo.canClearRecent}
          onrecentchange={showRecents}
          onclearrecent={clearRecent}
          onclose={libraryDrawer ? () => hideLibrary(true) : undefined}
        />
      </aside>{/if}
    {#if sidebar && !readingFocus && !libraryDrawer}<ResizeHandle
        value={sidebarWidth}
        min={librarySize.min}
        max={libraryWidth(librarySize.max, viewportWidth)}
        reset={librarySize.default}
        onpreview={(width) => (draftWidth = width)}
        oncommit={resizeLibrary}
      />{/if}
    <div class="document-host" inert={libraryDrawer}>
      {#each tabs as tab, index (tab.key)}<div
          class="document-tab"
          id={`document-panel-${tab.key}`}
          role="tabpanel"
          aria-labelledby={`document-tab-${tab.key}`}
          hidden={index !== active}
        >
          <DocumentPane
            bind:this={tab.pane}
            onlinked={openTab}
            onnewnote={() => {
              void runCommand("new_note");
            }}
            ontemplate={() => {
              void runCommand("note_template");
            }}
            ongit={showGit}
            onarchive={showReviewArchive}
            {recentFiles}
            {folderBusy}
            onrecent={recent}
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
    onrestart={() => closeAll(true)}
    onpreview={(values) => (previewValues = values)}
    oncommit={async () => {
      await refreshPreferences();
      draftWidth = null;
    }}
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

<NewNote
  bind:this={templatePanel}
  oncreate={createNote}
  oncreated={() => {
    void tabs[active]?.pane?.sourceLineAt(1).catch((error) => {
      message = bridge.errorMessage(error);
    });
  }}
/>
