<script lang="ts">
  import { onMount, tick } from "svelte";
  import Icon from "./ui/Icon.svelte";
  import { dock } from "./ui/dock";
  import BusyStatus from "./ui/BusyStatus.svelte";
  import { shortcutLabel } from "./commands";
  import { appearance, osDark, type Values } from "./preferences";
  import { arabicCompatibility } from "./fonts/arabic-compatibility";
  import { listen } from "@tauri-apps/api/event";
  import * as bridge from "./bridge";
  import type { Descriptor, SourceRange, SettingsResult } from "./bridge";
  import type {
    EditDescriptor,
    Recovery,
    Comparison,
  } from "./document/protocol";
  import type { EditQueue } from "./document/queue";
  import type { Database } from "./power/bridge";
  import { SvelteMap } from "svelte/reactivity";
  import type { Fence, InlineRun, InlineTarget } from "./power/fence";
  import type Reader from "./reader/Reader.svelte";
  import type { Read } from "./reader/model";

  import type ReviewPanel from "./review/Panel.svelte";
  import type SourceEditor from "./editor/SourceEditor.svelte";

  const instanceId = $props.id();
  let {
    active = true,
    onstate,
    onpreferences,
    previewSettings = null,
    ongit,
    onarchive,
    recentFiles = [],
    folderBusy = false,
    onrecent,
    onfolder,
    onnewnote,
    ontemplate,
    onlinked,
  }: {
    active?: boolean;
    ongit: () => void | Promise<void>;
    onarchive: () => void | Promise<void>;
    onpreferences: () => void;
    recentFiles?: import("./library").Hint[];
    folderBusy?: boolean;
    onrecent: (hint: import("./library").Hint) => Promise<void>;
    onfolder: () => Promise<void>;
    onnewnote: () => void;
    ontemplate: () => void;
    onlinked: (document: Descriptor, link?: string) => Promise<void>;
    previewSettings?: Values | null;
    onstate: (
      name: string,
      dirty: boolean,
      id: string | null,
      location: string | null,
    ) => void;
  } = $props();
  let readerToolbarTarget = $state<HTMLDivElement>();
  let authorToolbarTarget = $state<HTMLDivElement>();
  let documentMenuTarget = $state<HTMLDivElement>();
  let formattingOpen = $state(false);
  let formattingElement = $state<HTMLDetailsElement>();
  let actionsOpen = $state(false);
  let actionsElement: HTMLDetailsElement;
  async function documentAction(
    action: () => void | Promise<void>,
    modal = false,
  ) {
    actionsOpen = false;
    // Reload must reserve synchronously, before another mode handler can run.
    // Dialogs instead wait for the hidden action to relinquish focus.
    if (modal) await tick();
    actionsElement?.querySelector("summary")?.focus();
    try {
      await action();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  $effect(() => {
    if (!active) {
      actionsOpen = false;
    }
    if (!active || mode === "read" || writingFocus) formattingOpen = false;
  });
  $effect(() => {
    onstate(
      document?.name ?? "New tab",
      dirty,
      document?.id ?? null,
      document?.location ?? null,
    );
  });
  let reviewEnabled = $state(false),
    reviewDisplayed = $state(false);
  let PowerComponent = $state<
    typeof import("./power/Panel.svelte").default | null
  >(null);
  let powerPanel = $state<import("./power/Panel.svelte").default>();
  async function capturePower(saved: boolean) {
    if (busy) throw Error("Wait for the current document operation.");
    operationBusy = true;
    operation = "Capturing revision…";
    editor?.lock(true);
    try {
      await editor?.settle();
      const api = await import("./power/bridge");
      if (!saved) {
        if (!session || !editor)
          throw Error("Start Source before capturing a working revision.");
        await editor.controller().flush();
      }
      return await api.capture({
        document: realDocument(),
        editor: session?.id ?? null,
        generation: session?.generation ?? null,
        revision: editor?.controller().revision ?? null,
        saved,
      });
    } finally {
      editor?.lock(false);
      operationBusy = false;
    }
  }
  async function settlePower(capture: import("./power/bridge").Capture) {
    if (!capture.origin.startsWith("Working native revision")) return;
    if (busy || !session || !editor)
      throw Error("Working editor changed; capture the revision again.");
    operationBusy = true;
    operation = "Updating recovery…";
    editor.lock(true);
    try {
      await editor.settle();
      await editor.controller().flush();
    } finally {
      editor.lock(false);
      operationBusy = false;
    }
  }
  async function saveRunnerDocument() {
    const previous = realDocument();
    if (!session) await switchMode("source");
    if (!session || !editor || busy)
      throw Error("The editor is not ready to save a copy.");
    await save(true);
    return (
      !!realDocument() && realDocument() !== previous && !session.needsSaveAs
    );
  }
  async function saveRunnerCopy() {
    return (await saveRunnerDocument()) ? capturePower(false) : null;
  }
  async function openPower() {
    if (busy || inlinePending || powerPanel?.inlineRunning() || !document)
      return;
    PowerComponent ??= (await import("./power/Panel.svelte")).default;
    await tick();
    await powerPanel?.show();
  }
  const inlineResults = new SvelteMap<
    string,
    { target: InlineTarget; run: InlineRun }
  >();
  let inlinePending = $state(false);
  let choosingInlineDatabase = $state(false);
  let inlineDatabase = $state<{ document: string; value: Database } | null>(
    null,
  );
  const getInlineDatabase = () =>
    inlineDatabase?.document === realDocument()
      ? inlineDatabase?.value
      : undefined;
  async function chooseInlineDatabase(temporary = false) {
    const id = realDocument();
    if (!id || choosingInlineDatabase) return;
    choosingInlineDatabase = true;
    try {
      const { database } = await import("./power/bridge");
      const value = await database(id, temporary);
      if (value && realDocument() === id)
        inlineDatabase = { document: id, value };
      else if (temporary && realDocument() === id) inlineDatabase = null;
    } catch (error) {
      message = bridge.errorMessage(error);
    } finally {
      choosingInlineDatabase = false;
    }
  }
  const inlineKey = (target: InlineTarget) =>
    `${target.document}:${target.line}:${target.kind}`;
  function storeInline(target: InlineTarget, run: InlineRun) {
    if (target.document !== realDocument()) return;
    const key = inlineKey(target);
    inlineResults.delete(key);
    inlineResults.set(key, { target, run });
    if (inlineResults.size > 8)
      inlineResults.delete(inlineResults.keys().next().value!);
  }
  function getInline(fence: Fence) {
    const entry = inlineResults.get(
      inlineKey({ ...fence, document: realDocument() ?? "" }),
    );
    return entry?.target.code === fence.code ? entry.run : undefined;
  }
  async function runFence(fence: Fence) {
    if (
      busy ||
      inlinePending ||
      choosingInlineDatabase ||
      powerPanel?.inlineRunning()
    )
      return;
    const origin = window.document.activeElement;
    inlinePending = true;
    let target: InlineTarget | undefined;
    try {
      if (document?.bundledGuide || !realDocument()) {
        if (!(await saveRunnerDocument())) return;
        await switchMode("read");
        await reader?.revealSource(fence.line - 1);
      }
      const id = realDocument();
      if (!id) throw Error("Save the document to run its blocks.");
      target = { ...fence, document: id };
      storeInline(target, { pending: true });
      PowerComponent ??= (await import("./power/Panel.svelte")).default;
      await tick();
      if (id !== realDocument() || !powerPanel)
        throw Error("The document changed.");
      await powerPanel.showFence(
        target,
        !session,
        fence.kind === "sqlite" ? (getInlineDatabase()?.id ?? null) : null,
      );
      if (getInline(fence)?.pending) inlineResults.delete(inlineKey(target));
    } catch (error) {
      if (target) storeInline(target, { error: bridge.errorMessage(error) });
      else message = bridge.errorMessage(error);
    } finally {
      inlinePending = false;
      await tick();
      if (active && origin instanceof HTMLElement && origin.isConnected)
        origin.focus({ preventScroll: true });
    }
  }
  async function revealGuideExample(heading: string) {
    if (mode !== "read") await switchMode("read");
    await reader?.followAnchor(heading);
  }
  async function runFenceAtCaret() {
    if (busy || inlinePending || powerPanel?.inlineRunning()) return;
    const fence = editor?.fenceAtCaret();
    if (!fence) {
      message = "Place the caret inside a closed top-level Shell or SQL block.";
      return;
    }
    await switchMode("split");
    await reader?.revealSource(fence.line - 1);
    await runFence(fence);
  }
  function clearInline(fence: Fence) {
    inlineResults.delete(
      inlineKey({ ...fence, document: realDocument() ?? "" }),
    );
  }
  async function cancelInline() {
    try {
      await powerPanel?.cancelInline();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  let LinksComponent = $state<
    typeof import("./NoteLinks.svelte").default | null
  >(null);
  let linksPanel = $state<import("./NoteLinks.svelte").default>();
  async function showLinks(target?: string) {
    if (!realDocument() || busy) return;
    LinksComponent ??= (await import("./NoteLinks.svelte")).default;
    await tick();
    if (target) await linksPanel?.follow(target);
    else await linksPanel?.show();
  }
  export async function revealLink(link: string) {
    if (!link.includes("#")) return;
    await switchMode("read");
    await tick();
    await reader?.followAnchor(link.slice(link.indexOf("#") + 1));
  }
  let ReviewComponent = $state<typeof ReviewPanel | null>(null),
    reviewPanel = $state<ReviewPanel>();
  async function enableReview() {
    if (!document || !realDocument()) return;
    ReviewComponent ??= (await import("./review/Panel.svelte")).default;
    reviewEnabled = true;
  }
  async function toggleReview() {
    try {
      if (reviewEnabled) await endReview();
      else await enableReview();
    } catch (e) {
      message = bridge.errorMessage(e);
    }
  }
  async function endReview() {
    await reviewPanel?.stop();
    reviewEnabled = false;
    reviewDisplayed = false;
  }
  async function reviewJump(byte: number) {
    if (session || !document || !reviewPanel?.isCapture(document.revision)) {
      message =
        "Review positions belong to a saved capture. Reload the saved Read view before navigating; your editor text is kept.";
      return;
    }
    await reader?.revealByte(byte);
  }
  let relocation = $state<{ previous: string; document: Descriptor } | null>(
    null,
  );
  let loadingDocument = $state<string | null>(null);
  $effect(() => {
    if (!relocation || busy) return;
    const next = relocation;
    relocation = null;
    if (next.previous === document?.id && !session)
      void load(next.document, undefined, true).catch((error) => {
        if (document?.id !== next.document.id)
          void bridge.releaseDocument(next.document.id).catch(() => {});
        message = bridge.errorMessage(error);
      });
    else void bridge.releaseDocument(next.document.id).catch(() => {});
  });
  export async function load(
    selected: Descriptor,
    link?: string,
    preservePosition = false,
  ) {
    operationBusy = true;
    operation = "Opening document…";
    loadingDocument = selected.id;
    try {
      const position = preservePosition
        ? {
            byte: reader?.position() ?? readerStart,
            caret: 0,
            anchor: reader?.anchor() ?? readerAnchor,
          }
        : await bridge
            .documentPosition(selected.id)
            .catch(() => ({ byte: 0, caret: 0, anchor: null }));
      readerAnchor = position.anchor;
      if (link?.includes("#")) {
        try {
          const fragment = decodeURIComponent(
            link.slice(link.indexOf("#") + 1),
          );
          readerAnchor = fragment.startsWith("h-") ? fragment : "h-" + fragment;
        } catch {
          readerAnchor = null;
        }
      }
      await showDocument(
        selected,
        await bridge.readRange(
          selected.id,
          Math.min(position.byte, selected.byteLength),
        ),
      );
    } finally {
      loadingDocument = null;
      operationBusy = false;
    }
  }
  export function revealNavigation() {
    actionsOpen = false;
    void setReadingFocus(false, false);
  }
  export async function requestClose() {
    if (busy || inlinePending || powerPanel?.inlineRunning()) return false;
    // Reserve before the first await: watcher reloads and mode changes must not
    // enter while a leave decision or durable Review end is pending.
    operationBusy = true;
    operation = "Closing document…";
    try {
      const choice = await askLeave();
      if (!choice) return false;
      editor?.lock(true);
      await endReview();
      await finishLeave(choice);
      if (realDocument()) await bridge.releaseDocument(realDocument()!);
      document = null;
      return true;
    } catch (error) {
      editor?.lock(false);
      message = bridge.errorMessage(error);
      return false;
    } finally {
      operationBusy = false;
    }
  }
  export async function openNative() {
    await open();
  }
  let document = $state<Descriptor | null>(null);
  let page = $state<SourceRange | null>(null);
  let session = $state<Omit<EditDescriptor, "text"> | null>(null);
  let sourceText = $state("");
  let editor = $state<SourceEditor | undefined>();
  let SourceComponent = $state<typeof SourceEditor | null>(null);
  let mode = $state<"read" | "source" | "split" | "visual">("read");
  let readingFocus = $state(false);
  const focusedReading = $derived(
    !!document && mode === "read" && readingFocus,
  );
  let splitPercent = $state(50),
    writingFocus = $state(false),
    windowStart = $state(0);
  let renderRequest = 0;
  let syncedSplitEpoch = -1;
  let visual = $state(false),
    dirty = $state(false),
    durable = $state(true),
    operationBusy = $state(false),
    editorUndoing = $state(false),
    stale = $state(false);
  const busy = $derived(operationBusy || editorUndoing);
  let operation = $state("Working…");
  let diskState = $state("unchanged");
  let recoveryCapacity = $state(false);
  let message = $state(""),
    recoveryError = $state("");

  let recoveries = $state<Recovery[]>([]);
  let showSavedBackups = $state(false);
  let comparison = $state<Comparison | null>(null),
    compareLocal = $state("");
  let settings = $state<SettingsResult | null>(null);
  let settingsDocument = $state<string | null>(null);
  let settingsRequest = 0;
  let readerEpoch = $state(0);
  let readerPending = $state<"" | "loading" | "failed">("");
  let readerStart = $state(0);
  let readerAnchor = $state<string | null>(null);
  let reader = $state<Reader | undefined>();
  let ReaderComponent = $state<typeof Reader | null>(null);
  async function setReadingFocus(next: boolean, moveFocus = true) {
    if (!document || mode !== "read" || readingFocus === next) return;
    const restore = reader?.captureLayout();
    actionsOpen = false;
    readingFocus = next;
    await tick();
    restore?.();
    if (active && moveFocus) reader?.focusReading(next);
  }
  let currentRead = $state<Read>((start) =>
    bridge.readRange(document!.id, start),
  );
  let openGeneration = 0;
  let previewTimer: ReturnType<typeof setTimeout> | undefined,
    watchTimer: ReturnType<typeof setTimeout> | undefined,
    positionTimer: ReturnType<typeof setTimeout> | undefined;
  let discardDialog: HTMLDialogElement, compareDialog: HTMLDialogElement;
  let discardAnswer = $state<
    ((answer: "keep" | "discard" | null) => void) | null
  >(null);
  const openShortcut = $derived(
    shortcutLabel(
      String(settings?.resolved["shortcut.open"]?.value ?? "Mod+O"),
    ),
  );

  const value = (key: string, fallback: string | number | boolean) =>
    (previewSettings ?? settings?.resolved)?.[key]?.value ?? fallback;
  const realDocument = () =>
    document && document.id !== "recovery" ? document.id : null;

  const displayed = $derived(previewSettings ?? settings?.resolved ?? {});
  const presentation = $derived(
    appearance(displayed, $osDark, $arabicCompatibility),
  );
  const settingsCurrent = $derived(
    !!document && document.id === settingsDocument,
  );
  const renderHtml = $derived(
    settingsCurrent && value("reader.render_html", false) === true,
  );
  const remoteImages = $derived(
    settingsCurrent && value("reader.remote_images", true) === true,
  );
  $effect.pre(() => {
    const css = presentation;
    const restore = reader?.captureLayout();
    if (active) {
      window.document.documentElement.setAttribute("style", css);
      window.document.documentElement.dataset.theme = String(
        value("theme", "system"),
      );
    }
    let canceled = false;
    void tick().then(async () => {
      if (canceled) return;
      restore?.();
      await window.document.fonts.ready;
      if (!canceled) restore?.();
    });
    return () => {
      canceled = true;
    };
  });
  export async function sourceLineAt(line: number) {
    await switchMode("source");
    await tick();
    editor?.revealLine(line, true);
  }
  const configuredNumbers = $derived(
    Boolean(value("editor.line_numbers", true)),
  );
  const configuredSpellcheck = $derived(
    Boolean(value("editor.spellcheck", false)),
  );
  let numbers = $derived(configuredNumbers);
  let spellcheck = $derived(configuredSpellcheck);
  export function preferences() {
    return settings?.resolved ?? {};
  }
  export function commandState() {
    return {
      document: !!document,
      editing: !!session,
      busy: busy || inlinePending || !!powerPanel?.inlineRunning(),
      mode,
      readingFocus: focusedReading,
      authorBudget:
        !session?.range &&
        (editor?.sizeBytes() ?? document?.byteLength ?? 0) <= 2 * 1024 * 1024,
    };
  }
  export async function command(id: string) {
    switch (id) {
      case "open":
        return open();
      case "save":
        return save();
      case "save_as":
        return save(true);
      case "read":
        return switchMode("read");
      case "source":
        return switchMode("source");
      case "split":
        return switchMode("split");
      case "visual":
        return switchMode("visual");
      case "reload":
        return reload();
      case "power":
        await openPower();
        break;
      case "review":
        return toggleReview();
      case "compare":
        return compare();
      case "focus":
        if (mode === "read") await setReadingFocus(!readingFocus);
        else writingFocus = !writingFocus;
        break;
      case "outline":
        await setReadingFocus(false, false);
        reader?.action("outline");
        break;
      case "find":
        if (mode === "read") {
          await setReadingFocus(false, false);
          reader?.action("find");
        } else editor?.action("find");
        break;
    }
  }
  export async function refreshPreferences() {
    await loadSettings();
  }

  $effect(() => {
    if (stale && diskState === "changed" && !busy && !session && !dirty) {
      diskState = "reloading";
      void reload().finally(() => {
        if (diskState === "reloading") diskState = "failed";
      });
    }
  });
  async function render() {
    if (mode !== "read" && mode !== "split") return;
    const request = ++renderRequest;
    try {
      ReaderComponent ??= (await import("./reader/Reader.svelte")).default;
      if (request !== renderRequest) return;
      if (session?.range && editor) {
        readerPending = "loading";
        const editing = session,
          queue = editor.controller();
        await queue.flush();
        const token = await bridge.captureEditRead(
          editing.id,
          editing.generation,
          queue.revision,
        );
        if (
          request !== renderRequest ||
          editing.id !== session?.id ||
          editing.generation !== session.generation
        )
          return;
        currentRead = (start) =>
          bridge.readEditSnapshot(editing.id, editing.generation, token, start);
      } else
        currentRead =
          editor?.reader() ??
          ((start) => bridge.readRange(document!.id, start));
      // Heading IDs (including duplicate counters) belong to the old snapshot.
      // Retain its byte position, never apply an unverified ID to working text.
      if (session) readerAnchor = null;
      readerEpoch++;
      readerPending = "";
    } catch (e) {
      if (request === renderRequest) {
        message = bridge.errorMessage(e);
        readerPending = "failed";
      }
    }
  }
  export function droppedImage(
    offers: { id: string; name?: string; kind?: string }[],
  ) {
    if (!session || mode === "read") return false;
    const image = offers.find(
      (o) => o.kind === "file" && /\.(png|jpe?g)$/i.test(o.name ?? ""),
    );
    if (!image) return false;
    if (offers.length > 1)
      message =
        "Image import handles one file at a time. Drop remaining files individually.";
    editor?.assetDrop(image.id);
    return true;
  }
  function sourceAnchor(line: number, fraction = 0) {
    if (mode !== "split" || !editor) return;
    void reader?.revealSource(line, fraction);
  }
  function previewAnchor(line: number, clicked: boolean, fraction = 0) {
    if (mode !== "split") return;
    editor?.revealLine(line, clicked, fraction);
  }
  function resizeSplit(event: PointerEvent) {
    const target = event.currentTarget as HTMLElement;
    const parent = target.parentElement!;
    target.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      const r = parent.getBoundingClientRect();
      splitPercent = Math.max(
        25,
        Math.min(75, (100 * (e.clientX - r.left)) / r.width),
      );
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", end);
      target.removeEventListener("pointercancel", end);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end);
    target.addEventListener("pointercancel", end);
  }
  async function changeWindow(start: number, fromByte = false) {
    if (!session?.range || !editor || busy) return;
    const previous = editor;
    operationBusy = true;
    operation = "Changing editing range…";
    previous.lock(true);
    try {
      await previous.settle();
      const q = previous.controller();
      await q.flush();
      const result = await bridge.changeEditWindow(
        session.id,
        session.generation,
        q.revision,
        Math.max(0, start),
        fromByte,
      );
      const { text, ...metadata } = result;
      sourceText = text;
      session = metadata;
      dirty = result.state.dirty;
      windowStart = result.range?.from ?? 0;
      message =
        "Range changed. Undo, selection and find are local to this range; earlier edits remain in the full-document recovery.";
    } catch (e) {
      message = bridge.errorMessage(e);
    } finally {
      operationBusy = false;
      previous.lock(false);
    }
  }
  let previewKey = "";
  function changed(queue: EditQueue) {
    if (
      queue.descriptor.id !== session?.id ||
      queue.descriptor.generation !== session.generation
    )
      return;
    editorUndoing = editor?.undoPending() ?? false;
    const length = editor?.length();
    if (session?.range && length !== null && length !== undefined) {
      const to = session.range.from + length;
      session.range.total += to - session.range.to;
      session.range.to = to;
    }
    dirty = queue.dirty;
    durable = queue.durable;
    recoveryError = queue.error ? bridge.errorMessage(queue.error) : "";
    recoveryCapacity =
      !!queue.error &&
      typeof queue.error === "object" &&
      "code" in queue.error &&
      queue.error.code === "capacity";
    const nextPreview = `${queue.descriptor.id}:${queue.descriptor.generation}:${queue.revision}`;
    if (nextPreview !== previewKey) {
      previewKey = nextPreview;
      clearTimeout(previewTimer);
      if (mode === "read" || mode === "split")
        previewTimer = setTimeout(() => {
          if (editor) render();
        }, 180);
    }
    clearTimeout(positionTimer);
    positionTimer = setTimeout(() => {
      const id = realDocument();
      if (id)
        void bridge
          .rememberPosition(
            id,
            reader?.position() ?? page?.start ?? 0,
            editor?.caret() ?? 0,
            reader?.anchor() ?? readerAnchor,
          )
          .catch((error) => {
            message = bridge.errorMessage(error);
          });
    }, 1000);
  }
  async function askLeave(): Promise<"keep" | "discard" | null> {
    await editor?.settle();
    if (!dirty) return Promise.resolve("keep");
    if (discardAnswer) return Promise.resolve(null);
    return new Promise((resolve) => {
      discardAnswer = resolve;
    });
  }
  function answerLeave(answer: "keep" | "discard" | null) {
    const resolve = discardAnswer;
    discardAnswer = null;
    discardDialog.close();
    resolve?.(answer);
  }
  $effect(() => {
    if (discardAnswer) discardDialog.showModal();
  });
  async function finishLeave(choice: "keep" | "discard") {
    if (session) {
      editor?.lock(true);
      try {
        const id = realDocument();
        if (id)
          await bridge
            .rememberPosition(
              id,
              reader?.position() ?? page?.start ?? 0,
              editor?.caret() ?? 0,
              reader?.anchor() ?? readerAnchor,
            )
            .catch((error) => {
              message = bridge.errorMessage(error);
            });
        if (choice === "keep") await editor?.controller().flush();
        await bridge.releaseEditor(
          session.id,
          session.generation,
          choice === "discard",
        );
      } catch (error) {
        editor?.lock(false);
        throw error;
      }
    }
    clearTimeout(previewTimer);
    clearTimeout(positionTimer);
    session = null;
    sourceText = "";
    dirty = false;
    durable = true;
    recoveryError = "";
    await tick();
  }
  let preferenceWrite = Promise.resolve();
  function preference(key: string, value: string) {
    preferenceWrite = preferenceWrite
      .then(async () => {
        const old = await bridge.getSettings(realDocument(), "global");
        const line = `$${key}=${value}`;
        const lines = old.text.split("\n");
        const found = lines.findIndex((l) => l.trim().startsWith(`$${key}=`));
        if (found >= 0) lines[found] = line;
        else lines.push(line);
        settings = await bridge.updateSettings(
          realDocument(),
          "global",
          lines.join("\n"),
        );
      })
      .catch((e) => {
        message = bridge.errorMessage(e);
      });
  }
  async function loadSettings() {
    const request = ++settingsRequest;
    const owner = document?.id ?? null;
    const next = await bridge.getSettings(realDocument(), "global");
    if (request === settingsRequest && owner === (document?.id ?? null)) {
      settings = next;
      settingsDocument = owner;
      if (next.diagnostics.length)
        message =
          "Preferences use safe defaults: " +
          next.diagnostics.map((d) => d.message).join(" ");
    }
  }
  async function refreshRecoveries() {
    try {
      recoveries = await bridge.listRecoveries(showSavedBackups);
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  async function showDocument(selected: Descriptor, loaded: SourceRange) {
    ReaderComponent ??= (await import("./reader/Reader.svelte")).default;
    if (reviewEnabled && document?.location !== selected.location)
      await endReview();
    reviewDisplayed = false;
    if (
      !document ||
      (document.id !== selected.id &&
        (!document.location || document.location !== selected.location))
    )
      readingFocus = false;
    if (document && realDocument() && selected.id !== document.id)
      await bridge.releaseDocument(document.id);
    document = selected;
    page = loaded;
    stale = false;
    diskState = "unchanged";
    mode = "read";
    readerStart = loaded.start;
    readerPending = "";
    currentRead = (start) => bridge.readRange(selected.id, start);
    readerEpoch++;
    await loadSettings().catch(
      (error) => (message = bridge.errorMessage(error)),
    );
    void refreshRecoveries();
    void bridge
      .rememberDocument(selected.id)
      .catch((error) => (message = bridge.errorMessage(error)));
  }
  async function open(example = false, link?: string) {
    if (busy || inlinePending || powerPanel?.inlineRunning()) return;
    operationBusy = true;
    operation = "Opening file…";
    const request = ++openGeneration;
    message = "";
    let selected: Descriptor | null = null;
    try {
      const choice = await askLeave();
      if (!choice) return;
      selected = example
        ? await bridge.openExample()
        : link && realDocument()
          ? await bridge.openDocumentLink(realDocument()!, link)
          : await bridge.chooseDocument();
      if (!selected) return;
      const position = await bridge
        .documentPosition(selected.id)
        .catch(() => ({ byte: 0, caret: 0, anchor: null }));
      readerAnchor = position.anchor;
      if (link?.includes("#")) {
        try {
          const fragment = decodeURIComponent(
            link.split("#").slice(1).join("#"),
          );
          readerAnchor = fragment.startsWith("h-") ? fragment : "h-" + fragment;
        } catch {
          readerAnchor = null;
        }
      }
      const loaded = await bridge.readRange(
        selected.id,
        Math.min(position.byte, selected.byteLength),
      );
      if (request !== openGeneration) {
        await bridge.releaseDocument(selected.id);
        return;
      }
      if (reviewEnabled && document?.location !== selected.location)
        await endReview();
      await finishLeave(choice);
      await showDocument(selected, loaded);
    } catch (error) {
      if (selected && selected.id !== document?.id)
        await bridge.releaseDocument(selected.id).catch(() => {});
      message = bridge.errorMessage(error);
    } finally {
      if (request === openGeneration) operationBusy = false;
    }
  }
  async function switchMode(next: "read" | "source" | "split" | "visual") {
    if (busy) return;
    if (next !== "read" && !session) {
      const id = realDocument();
      if (!id) return;
      operationBusy = true;
      operation = "Preparing editor…";
      try {
        const descriptor = await bridge.beginEdit(
          id,
          reader?.position() ?? readerStart,
        );
        SourceComponent ??= (await import("./editor/SourceEditor.svelte"))
          .default;
        const { text, ...metadata } = descriptor;
        sourceText = text;
        session = metadata;
        windowStart = metadata.range?.from ?? 0;
        dirty = metadata.state.dirty;
        durable = true;
      } catch (error) {
        message = bridge.errorMessage(error);
        return;
      } finally {
        operationBusy = false;
      }
    }
    if (
      (next === "split" || next === "visual") &&
      (session?.range ||
        (editor?.sizeBytes() ?? document!.byteLength) > 2 * 1024 * 1024)
    ) {
      message =
        "Split and Visual currently admit ordinary documents up to 2 MiB; use Source/range editing for this document.";
      next = "source";
    }
    if (next !== "read") readingFocus = false;
    const nativeEdits = editor?.flushNative();
    if (nativeEdits) await nativeEdits;
    mode = next;
    if (next === "visual") visual = true;
    else if (next === "source") visual = false;
    if (next === "read" || next === "split") void render();
    await tick();
  }
  async function save(asNew = false) {
    if (!session || !editor || busy) return;
    if (asNew && reviewEnabled) {
      try {
        await endReview();
      } catch (e) {
        message = bridge.errorMessage(e);
        return;
      }
    }
    operationBusy = true;
    operation = "Saving document…";
    const queue = editor.controller();
    try {
      const nativeEdits = editor.settle();
      if (nativeEdits) await nativeEdits;
      let revision = queue.pause();
      try {
        revision = await queue.saveBarrier();
      } catch (error) {
        if (!asNew) throw error;
        revision = queue.pause();
      }
      if (asNew || session.needsSaveAs) {
        const pending = queue.pending.filter(
          (batch) => batch.newRevision <= revision,
        );
        const result = await bridge.saveAsDocument(
          session.id,
          session.generation,
          revision,
          pending,
        );
        if (!result) return;
        if (result.saved.rescue) {
          message = `An independent copy (${result.saved.name}) was saved. Recovery is still unavailable; your open text is kept. Reopen the copy before starting a new editing session.`;
          if (result.document) await bridge.releaseDocument(result.document.id);
          return;
        }
        queue.saved(result.saved.state);
        session.needsSaveAs = false;
        session.name = result.saved.name;
        if (realDocument()) await bridge.releaseDocument(document!.id);
        document = result.document ?? {
          id: "recovery",
          name: result.saved.name,
          byteLength: 0,
          revision: "saved-copy",
          watching: false,
        };
        await loadSettings();
        message = result.refreshWarning ?? "Saved an independent copy.";
      } else {
        const result = await bridge.saveDocument(
          session.id,
          session.generation,
          revision,
        );
        queue.saved(result.state);
        message = result.noop
          ? "Already saved · original bytes and file identity unchanged."
          : "Saved.";
      }
      stale = false;
    } catch (error) {
      message = bridge.errorMessage(error);
      if (
        typeof error === "object" &&
        error &&
        "code" in error &&
        error.code === "conflict"
      )
        stale = true;
    } finally {
      queue.resume();
      operationBusy = false;
    }
  }
  async function clearUndoAndRetry() {
    editor?.clearUndo();
    await retry();
  }
  async function retry() {
    try {
      await editor?.controller().flush();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  async function checkDisk() {
    const id = document?.id,
      token = session?.generation;
    try {
      const status = session
        ? await bridge.editingStatus(session.id, session.generation)
        : realDocument()
          ? await bridge.documentStatus(realDocument()!)
          : "unchanged";
      if (id === document?.id && token === session?.generation) {
        diskState = status;
        stale = status !== "unchanged";
      }
    } catch (error) {
      if (id === document?.id) {
        stale = true;
        message = bridge.errorMessage(error);
      }
    }
  }
  async function compare() {
    if (!session) await switchMode("source");
    if (!session) return;
    try {
      comparison = await bridge.compareDocument(session.id, session.generation);
      compareLocal = editor?.preview() ?? "";
      compareDialog.showModal();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  async function reload() {
    const id = realDocument();
    if (!id || busy) return;
    operationBusy = true;
    operation = "Reloading from disk…";
    let selected: Descriptor | null = null;
    try {
      const choice = await askLeave();
      if (!choice) return;
      readerAnchor = reader?.anchor() ?? readerAnchor;
      selected = await bridge.reopenDocument(id);
      const loaded = await bridge.readRange(selected.id);
      await finishLeave(choice);
      await showDocument(selected, loaded);
    } catch (error) {
      if (selected && selected.id !== document?.id)
        await bridge.releaseDocument(selected.id).catch(() => {});
      message = bridge.errorMessage(error);
    } finally {
      operationBusy = false;
    }
  }
  export async function createNote(title: string, template: string) {
    if (busy || document)
      throw Error("Choose an empty tab before creating a note.");
    operationBusy = true;
    operation = "Creating note…";
    let created: EditDescriptor | null = null;
    try {
      created = await bridge.createNote(title, template);
      SourceComponent ??= (await import("./editor/SourceEditor.svelte"))
        .default;
      installDraft(created);
      message = "Draft kept in recovery. Save to choose its file and folder.";
      await tick();
      editor?.revealLine(1, true);
    } catch (error) {
      if (created && created.id !== session?.id)
        await bridge
          .releaseEditor(created.id, created.generation, false)
          .catch(() => {});
      throw error;
    } finally {
      operationBusy = false;
    }
  }
  function installDraft(draft: EditDescriptor) {
    const { text, ...metadata } = draft;
    session = metadata;
    sourceText = text;
    document = {
      id: "recovery",
      name: metadata.name,
      byteLength: new TextEncoder().encode(text).length,
      revision: "recovery",
      watching: false,
    };
    page = { start: 0, end: 0, total: 0, text: "" };
    mode = "source";
    stale = false;
    dirty = true;
    durable = true;
  }
  async function restore(id: string) {
    const choice = await askLeave();
    if (!choice) return;
    operationBusy = true;
    operation = "Restoring recovery…";
    let restored: EditDescriptor | null = null;
    try {
      restored = await bridge.restoreRecovery(id);
      SourceComponent ??= (await import("./editor/SourceEditor.svelte"))
        .default;
      await finishLeave(choice);
      if (realDocument()) await bridge.releaseDocument(document!.id);
      installDraft(restored);
      message =
        "Recovered text. Save As requires a new native destination; previous paths are not automatically reauthorized.";
      await refreshRecoveries();
    } catch (error) {
      if (restored && restored.id !== session?.id)
        await bridge
          .releaseEditor(restored.id, restored.generation, false)
          .catch(() => {});
      message = bridge.errorMessage(error);
    } finally {
      operationBusy = false;
    }
  }
  async function discard(id: string) {
    const answer = await new Promise<"keep" | "discard" | null>((resolve) => {
      discardAnswer = resolve;
    });
    if (answer !== "discard") return;
    try {
      await bridge.discardRecovery(id);
      await refreshRecoveries();
    } catch (error) {
      message = bridge.errorMessage(error);
    }
  }
  onMount(() => {
    const stops = [
      listen("preferences-changed", () => {
        void loadSettings().catch(
          (error) => (message = bridge.errorMessage(error)),
        );
      }),
      listen<{ id: string }>("document-invalidated", (event) => {
        if (event.payload.id === document?.id && !watchTimer)
          watchTimer = setTimeout(() => {
            watchTimer = undefined;
            void checkDisk();
          }, 150);
      }),
      listen<{ previous: string; document: Descriptor }>(
        "document-relocated",
        (event) => {
          const next = event.payload;
          if (
            next.previous === document?.id ||
            next.previous === loadingDocument ||
            next.previous === relocation?.document.id
          ) {
            const previous =
              next.previous === relocation?.document.id
                ? relocation.previous
                : next.previous;
            if (relocation)
              void bridge
                .releaseDocument(relocation.document.id)
                .catch(() => {});
            relocation = { previous, document: next.document };
          }
        },
      ),
    ];
    const focus = () => {
      if (active) {
        void checkDisk();
        void loadSettings().catch(
          (error) => (message = bridge.errorMessage(error)),
        );
      }
    };
    window.addEventListener("focus", focus);
    const dismissActions = (event: Event) => {
      if (
        actionsOpen &&
        event.target instanceof Node &&
        !actionsElement?.contains(event.target)
      )
        actionsOpen = false;
    };
    // Observe both pointer input and keyboard/programmatic click activation
    // before editor handlers, without consuming or changing their input.
    window.addEventListener("pointerdown", dismissActions, true);
    window.addEventListener("click", dismissActions, true);
    window.addEventListener("focusin", dismissActions, true);

    void loadSettings().catch((error) => {
      message = bridge.errorMessage(error);
    });
    void refreshRecoveries();
    void bridge
      .storageWarning()
      .then((warning) => {
        if (warning) message = warning;
      })
      .catch((error) => {
        message = bridge.errorMessage(error);
      });
    return () => {
      clearTimeout(previewTimer);
      clearTimeout(watchTimer);
      clearTimeout(positionTimer);
      if (relocation)
        void bridge.releaseDocument(relocation.document.id).catch(() => {});
      for (const stop of stops) void stop.then((unlisten) => unlisten());
      window.removeEventListener("focus", focus);
      window.removeEventListener("pointerdown", dismissActions, true);
      window.removeEventListener("click", dismissActions, true);
      window.removeEventListener("focusin", dismissActions, true);
    };
  });
</script>

<div
  class="window-shell"
  data-dirty={dirty}
  data-editing={!!session}
  data-busy={busy}
  data-mode={mode}
  class:with-review={reviewEnabled && !focusedReading}
  class:reading-focus={focusedReading}
  class:writing-focus={writingFocus && mode !== "read"}
  data-theme={String(value("theme", "system"))}
  style={presentation}
  data-code-style={String(value("code.style", "classic"))}
>
  <header class="toolbar" class:document-toolbar={!!document}>
    {#if !document}
      <span class="wordmark"
        >Marktub<span class="wordmark-dot" aria-hidden="true"></span></span
      >
      <span class="document-name">Markdown, at your own pace</span>
    {/if}
    {#if document}<div class="mode-switch" aria-label="Document mode">
        <button
          aria-pressed={mode === "read"}
          onclick={() => switchMode("read")}
          disabled={busy}>Read</button
        ><button
          aria-pressed={mode === "source"}
          onclick={() => switchMode("source")}
          disabled={busy}>Source</button
        ><button
          aria-pressed={mode === "split"}
          onclick={() => switchMode("split")}
          disabled={busy}>Split</button
        ><button
          aria-pressed={mode === "visual"}
          onclick={() => switchMode("visual")}
          disabled={busy}>Visual</button
        >
      </div>{/if}
    {#if session && mode !== "read"}<div
        class="author-toolbar-slot"
        bind:this={authorToolbarTarget}
      ></div>{/if}
    <div class="reader-toolbar-slot" bind:this={readerToolbarTarget}></div>
    {#if session && (mode !== "read" || dirty || session.needsSaveAs)}<button
        class="save-button"
        class:primary={dirty}
        onclick={() => save()}
        disabled={busy}>Save</button
      >{/if}
    <button
      class="open-button"
      onclick={() => open()}
      disabled={busy}
      title={openShortcut.visual
        ? `Open file (${openShortcut.visual})`
        : "Open file"}
    >
      <Icon name="folder" /><span class:sr-only={!!document}>Open file</span
      >{#if !document}
        <kbd aria-label={openShortcut.spoken}>{openShortcut.visual}</kbd
        >{/if}</button
    >
    <details
      class="document-actions"
      bind:this={actionsElement}
      bind:open={actionsOpen}
    >
      <summary aria-label="More document actions">More</summary>
      <div class="document-actions-list" bind:this={documentMenuTarget}>
        {#if document?.bundledGuide}
          <button
            disabled={busy}
            onclick={() => documentAction(() => revealGuideExample("shell"))}
            >Shell example</button
          >
          <button
            disabled={busy}
            onclick={() => documentAction(() => revealGuideExample("sqlite"))}
            >SQLite example</button
          >
        {/if}

        {#if session && !session.range && (mode === "source" || mode === "visual")}
          <button
            onclick={() => documentAction(runFenceAtCaret, true)}
            disabled={busy || inlinePending || !!powerPanel?.inlineRunning()}
            >Run block</button
          >
        {/if}

        {#if session}<button
            onclick={() => documentAction(() => save(true), true)}
            disabled={busy}>Save As</button
          >{/if}
        {#if document && realDocument()}<button
            onclick={() => documentAction(reload)}
            disabled={busy}>Reload</button
          >{/if}
        {#if realDocument()}<button
            onclick={() => documentAction(() => showLinks(), true)}
            disabled={busy}>Links</button
          >
          <button
            aria-pressed={reviewEnabled}
            onclick={() => documentAction(toggleReview)}
            disabled={busy}>{reviewEnabled ? "Review on" : "Review"}</button
          >
        {/if}
        {#if document}<button
            onclick={() => documentAction(openPower, true)}
            disabled={busy}>Export &amp; run</button
          >{/if}
        <button
          onclick={() => documentAction(onpreferences, true)}
          disabled={busy}>Settings</button
        >
        <button
          onclick={() => {
            showSavedBackups = !showSavedBackups;
            void refreshRecoveries();
          }}
          disabled={busy}
          >{showSavedBackups
            ? "Hide saved backups"
            : "Show saved backups"}</button
        >
      </div>
    </details>
  </header>
  <BusyStatus
    active={busy}
    label={editorUndoing ? "Applying undo / redo…" : operation}
  />
  {#if LinksComponent && realDocument()}<LinksComponent
      bind:this={linksPanel}
      documentId={realDocument()!}
      onopen={onlinked}
    />{/if}
  {#if PowerComponent}<PowerComponent
      bind:this={powerPanel}
      oncapture={capturePower}
      onsettle={settlePower}
      oncopy={saveRunnerCopy}
      oninline={storeInline}
      hasSaved={!!realDocument()}
      hasWorking={!!session}
    />{/if}
  {#if reviewEnabled && ReviewComponent && document && realDocument()}<ReviewComponent
      bind:this={reviewPanel}
      {document}
      {active}
      savedDisplayed={reviewDisplayed && mode === "read" && !session}
      editing={!!session}
      {stale}
      onoff={() => (reviewEnabled = false)}
      onjump={reviewJump}
      {ongit}
      onconflict={compare}
      {onarchive}
    />{/if}
  {#if message}<div class="notice" role="status">
      {message}<button
        onclick={() => (message = "")}
        aria-label="Dismiss message">×</button
      >
    </div>{/if}
  {#if recoveryError}<div class="notice" role="alert">
      Recovery failed: {recoveryError} Your text stays open. {#if recoveryCapacity && !session?.range}<button
          onclick={clearUndoAndRetry}
          disabled={busy}>Clear undo history and retry</button
        >{/if}
      <button onclick={retry} disabled={busy}>Retry recovery</button><button
        onclick={() => save(true)}
        disabled={busy}>Save As</button
      >
    </div>{/if}
  {#if stale}<div class="notice" role="status">
      The file changed or was removed on disk. Your open text is kept.<button
        onclick={compare}
        disabled={busy}>Compare</button
      ><button onclick={reload} disabled={busy || !realDocument()}
        >Reload disk</button
      ><button
        onclick={() => {
          stale = false;
          message =
            "Local edits kept. Saving over a changed disk revision remains blocked; use Compare, Reload or Save As.";
        }}>Keep local</button
      >{#if session}<button onclick={() => save(true)} disabled={busy}
          >Save As</button
        >{/if}
    </div>{/if}
  {#if recoveries.length || showSavedBackups}<details
      class="recovery-list"
      use:dock={document ? documentMenuTarget : undefined}
      aria-label="Unsaved recovery records"
      open={!document || showSavedBackups}
    >
      <summary
        >Recover documents and saved backups ({recoveries.length})</summary
      >{#each recoveries as recovery (recovery.id)}<div>
          <span
            >{recovery.name} · {recovery.clean ? "saved backup" : "unsaved"} · revision
            {recovery.revision}</span
          ><button
            onclick={() => documentAction(() => restore(recovery.id), true)}
            disabled={busy || recovery.damaged}>Restore</button
          ><button onclick={() => discard(recovery.id)} disabled={busy}
            >Discard recovery</button
          >
        </div>{/each}
    </details>{/if}
  <main aria-busy={busy} class:has-document={!!document}>
    {#if !document}<section class="empty-state">
        <h1>A little room to read.</h1>
        <p>Start a note, or open your Markdown files. Your words stay local.</p>
        <div class="empty-actions">
          <button class="primary" onclick={onnewnote} disabled={busy}
            >New note</button
          >
          <button onclick={ontemplate} disabled={busy}>From template</button>
          <button onclick={() => open()} disabled={busy}>Open file</button
          ><button
            class="folder-button"
            onclick={onfolder}
            disabled={busy || folderBusy}>Open folder</button
          >
          <button onclick={() => open(true)} disabled={busy}
            >Read the example</button
          >
        </div>
        {#if recentFiles.length}<section
            class="recent-files"
            aria-label="Recent files"
          >
            <h2>Recent files</h2>
            <ul>
              {#each recentFiles.slice(0, 6) as hint (hint.id)}<li>
                  <button
                    onclick={() => onrecent(hint)}
                    disabled={busy || folderBusy}
                    title={[hint.name, hint.context]
                      .filter(Boolean)
                      .join(" — ")}
                  >
                    <span class="recent-file-info"
                      ><span>{hint.name}</span>{#if hint.context}<span
                          class="recent-context">{hint.context}</span
                        >{/if}</span
                    ><small>Open</small>
                  </button>
                </li>{/each}
            </ul>
          </section>{/if}
      </section>
    {:else}
      <div
        class="author-layout"
        class:split={mode === "split"}
        class:visual={mode === "visual"}
        style={mode === "split"
          ? `grid-template-columns:minmax(0,${splitPercent}fr) 8px minmax(0,${100 - splitPercent}fr)`
          : ""}
      >
        {#if session && SourceComponent}<div
            class:hidden={mode === "read"}
            class="source-surface"
          >
            {#if writingFocus}<div class="writing-focus-controls">
                <button onclick={() => (writingFocus = false)}
                  ><Icon name="focus" />Exit writing focus</button
                >
              </div>{/if}
            <div
              class="author-tools"
              use:dock={mode === "read" || writingFocus
                ? undefined
                : authorToolbarTarget}
            >
              <details
                class="formatting-tools"
                bind:this={formattingElement}
                bind:open={formattingOpen}
                onfocusout={(event) => {
                  if (
                    event.relatedTarget instanceof Node &&
                    !formattingElement?.contains(event.relatedTarget)
                  )
                    formattingOpen = false;
                }}
              >
                <summary
                  title="Formatting and editor options"
                  aria-label="Formatting and editor options"
                  ><Icon name="type" /><span>Format</span></summary
                >
                <div class="formatting-menu">
                  <div class="source-toolbar">
                    <span
                      >{session.needsSaveAs
                        ? "Choose a new destination with Save As."
                        : "Changes stay local until you save."}</span
                    ><button
                      class="icon-button"
                      aria-label="Undo"
                      title="Undo"
                      onclick={() => editor?.undo()}
                      disabled={busy}
                      ><Icon name="undo" /><span class="sr-only">Undo</span
                      ></button
                    ><button
                      class="icon-button"
                      aria-label="Redo"
                      title="Redo"
                      onclick={() => editor?.undo(true)}
                      disabled={busy}
                      ><Icon name="redo" /><span class="sr-only">Redo</span
                      ></button
                    ><label
                      ><input type="checkbox" bind:checked={visual} /> Live heading/list
                      formatting</label
                    >
                  </div>
                  <div class="author-controls">
                    <button
                      class="icon-button"
                      aria-label="Find / replace"
                      title="Find / replace"
                      onclick={() => editor?.action("find")}
                      ><Icon name="find" /><span class="sr-only"
                        >Find / replace</span
                      ></button
                    ><button
                      class="focus-toggle icon-button"
                      aria-label={writingFocus
                        ? "Exit writing focus"
                        : "Writing focus"}
                      title={writingFocus
                        ? "Exit writing focus"
                        : "Writing focus"}
                      aria-pressed={writingFocus}
                      onclick={() => (writingFocus = !writingFocus)}
                      ><Icon name="focus" /><span class="sr-only"
                        >{writingFocus
                          ? "Exit writing focus"
                          : "Writing focus"}</span
                      ></button
                    ><label
                      ><input
                        type="checkbox"
                        bind:checked={spellcheck}
                      />Spellcheck</label
                    ><label
                      ><input type="checkbox" bind:checked={numbers} />Line
                      numbers</label
                    >{#each [["fold", "Fold"], ["unfold", "Unfold"], ["bold", "Bold"], ["italic", "Italic"], ["code", "Code"], ["link", "Link"], ["heading1", "Heading"], ["list", "List"], ["ordered", "Numbered"], ["task", "Task"], ["quote", "Quote"], ["table", "Table"], ["image", "Image"], ["math", "Math"], ["mermaid", "Diagram"]] as [id, label] (id)}<button
                        class="icon-button"
                        class:tool-group-start={[
                          "bold",
                          "heading1",
                          "table",
                        ].includes(id)}
                        aria-label={label}
                        title={label}
                        onclick={() => editor?.action(id)}
                        disabled={busy}
                        ><Icon name={id} /><span class="sr-only">{label}</span
                        ></button
                      >{/each}
                  </div>
                </div>
              </details>
            </div>
            {#if session.range}<div
                class="range-controls"
                role="region"
                aria-label="Range editing"
              >
                <strong
                  >Range {session.range.from.toLocaleString()}–{session.range.to.toLocaleString()}
                  of {session.range.total.toLocaleString()} UTF-16 units</strong
                >
                <p>
                  Undo / find / selection apply only here. Save writes the full
                  document. Switching flushes recovery and starts a new local
                  undo history; boundaries may split long lines.
                </p>
                <input
                  aria-label="Range start"
                  type="number"
                  min="0"
                  max={session.range.total}
                  bind:value={windowStart}
                /><button
                  onclick={() => changeWindow(windowStart)}
                  disabled={busy}>Go to range</button
                ><button
                  onclick={() =>
                    changeWindow(Math.max(0, session!.range!.from - 131072))}
                  disabled={busy || session.range.from === 0}
                  >Previous range</button
                ><button
                  onclick={() => changeWindow(session!.range!.to)}
                  disabled={busy || session.range.to >= session.range.total}
                  >Next range</button
                >
              </div>{/if}
            {#key session.id + ":" + session.generation}<SourceComponent
                bind:this={editor}
                {session}
                text={sourceText}
                {visual}
                {spellcheck}
                {numbers}
                active={active && (mode === "source" || mode === "split")}
                nativeAllowed={$arabicCompatibility}
                focus={writingFocus}
                onanchor={sourceAnchor}
                onimage={async (copy, offer, bytes) => {
                  const id = realDocument();
                  if (!id)
                    throw Error(
                      "Save recovered text to a document before importing images.",
                    );
                  return bridge.importAuthorImage(id, copy, offer, bytes);
                }}
                tabWidth={Number(value("editor.tab_width", 4))}
                wrap={Boolean(value("editor.wrap", true))}
                flushMs={Number(value("recovery.flush_ms", 500))}
                {presentation}
                onchange={changed}
                onready={() => {
                  sourceText = "";
                  if (editor) render();
                }}
                onerror={(text) => (message = text)}
              />{/key}
          </div>{/if}
        {#if mode === "split"}<button
            class="split-divider"
            aria-label="Resize source and preview"
            onpointerdown={resizeSplit}
            onkeydown={(e) => {
              if (e.key === "ArrowLeft")
                splitPercent = Math.max(25, splitPercent - 5);
              if (e.key === "ArrowRight")
                splitPercent = Math.min(75, splitPercent + 5);
            }}>⋮</button
          >{/if}
        {#if (mode === "read" || mode === "split") && active && readerPending}
          <p role="status">
            {readerPending === "loading"
              ? "Preparing the full working snapshot…"
              : "Working preview unavailable. Your edits are kept; choose Read again to retry."}
          </p>
        {:else if (mode === "read" || mode === "split") && active}
          {#key `${readerEpoch}:${renderHtml}:${remoteImages}`}{#if ReaderComponent}<ReaderComponent
                bind:this={reader}
                read={currentRead}
                toolbarTarget={readerToolbarTarget}
                embedded={mode === "split"}
                focused={focusedReading}
                ontogglefocus={() => setReadingFocus(!readingFocus)}
                onedit={mode === "read" &&
                (session?.range || document.byteLength > 16 * 1024 * 1024)
                  ? async (byte) => {
                      if (session?.range) await changeWindow(byte, true);
                      await switchMode("source");
                    }
                  : undefined}
                onsource={previewAnchor}
                operationBusy={busy}
                presentation={displayed}
                allowHtml={renderHtml}
                {remoteImages}
                dark={$osDark}
                start={readerStart}
                anchorId={readerAnchor}
                onpreference={preference}
                preferences={{
                  theme: String(value("theme", "system")),
                  width: Number(value("reader.width", 760)),
                  size: Number(value("reader.font_size", 18)),
                }}
                title={document.name}
                revision={session
                  ? `Working revision ${editor?.controller().revision ?? 0}`
                  : `Saved snapshot ${document.revision}`}
                ondisplay={() => {
                  if (mode === "split" && syncedSplitEpoch !== readerEpoch) {
                    syncedSplitEpoch = readerEpoch;
                    const position = editor?.viewportAnchor();
                    if (position)
                      sourceAnchor(position.line, position.fraction);
                  }
                  reviewDisplayed = true;
                  void reviewPanel
                    ?.markDisplayed()
                    .catch((e) => (message = bridge.errorMessage(e)));
                }}
                onposition={(byte, anchor) => {
                  readerAnchor = anchor;
                  readerStart = byte;
                  const id = realDocument();
                  if (id)
                    void bridge
                      .rememberPosition(id, byte, editor?.caret() ?? 0, anchor)
                      .catch((e) => (message = bridge.errorMessage(e)));
                }}
                onlink={(link) => void open(false, link)}
                onnote={(target) => void showLinks(target)}
                onimage={async (link, automatic) => {
                  const id = realDocument();
                  if (!id) {
                    if (automatic) return null;
                    throw Error(
                      "Choose a saved document before authorizing image files.",
                    );
                  }
                  return bridge.chooseImage(id, link, automatic);
                }}
                onerror={(error) => (message = error)}
                onrun={runFence}
                oncancelrun={cancelInline}
                onclearrun={clearInline}
                getRun={getInline}
                runBusy={() =>
                  inlinePending ||
                  choosingInlineDatabase ||
                  !!powerPanel?.inlineRunning()}
                saveBeforeRun={() =>
                  !!document?.bundledGuide || !realDocument()}
                getDatabase={getInlineDatabase}
                onselectdatabase={chooseInlineDatabase}
                onresetdatabase={() => chooseInlineDatabase(true)}
              />{/if}{/key}
        {/if}
      </div>
    {/if}
  </main>
  {#if session || (document && !document.bundledGuide && !document.watching && realDocument())}
    <footer class="statusbar">
      {#if session}<span
          >{(dirty
            ? "Unsaved changes"
            : stale
              ? "Disk revision changed"
              : "Saved") +
            " · " +
            (durable ? "Recovery up to date" : "Recovery pending")}</span
        >{/if}
      {#if document && !document.bundledGuide && !document.watching && realDocument()}<span
          >File watching unavailable; rechecked on focus</span
        >{/if}
    </footer>
  {/if}
</div>

<svelte:window
  onkeydown={(event) => {
    if (
      !active ||
      event.defaultPrevented ||
      event.isComposing ||
      event.keyCode === 229 ||
      window.document.querySelector("dialog[open]")
    )
      return;
    if (formattingOpen && event.key === "Escape") {
      event.preventDefault();
      formattingOpen = false;
      formattingElement?.querySelector("summary")?.focus();
    } else if (actionsOpen && event.key === "Escape") {
      event.preventDefault();
      actionsOpen = false;
      actionsElement?.querySelector("summary")?.focus();
    } else if (focusedReading && event.key === "Escape") {
      event.preventDefault();
      void setReadingFocus(false);
    } else if (writingFocus && event.key === "Escape") {
      event.preventDefault();
      writingFocus = false;
    }
  }}
  onpointerdown={(event) => {
    if (formattingOpen && !formattingElement?.contains(event.target as Node))
      formattingOpen = false;
  }}
/>
<dialog
  bind:this={discardDialog}
  aria-labelledby={`${instanceId}-discard-title`}
  oncancel={(event) => {
    event.preventDefault();
    answerLeave(null);
  }}
>
  <h2 id={`${instanceId}-discard-title`}>Keep unsaved recovery?</h2>
  <p>
    Keep recovery flushes acknowledged edits before leaving. Discard permanently
    removes this recovery record. Cross-restart recovery restores text, not undo
    history.
  </p>
  <div class="dialog-actions">
    <button onclick={() => answerLeave(null)}>Keep editing</button><button
      onclick={() => answerLeave("keep")}>Keep recovery</button
    ><button class="primary" onclick={() => answerLeave("discard")}
      >Discard edits</button
    >
  </div>
</dialog>
<dialog
  bind:this={compareDialog}
  class="compare-dialog"
  aria-labelledby={`${instanceId}-compare-title`}
>
  <h2 id={`${instanceId}-compare-title`}>Compare revisions</h2>
  {#if comparison}<p>
      {comparison.missing
        ? "The selected file is missing. Local edits and the base remain available."
        : "Disk changes never replace your open edits automatically."} First 64 KiB
      of each revision shown.
    </p>
    <div class="comparison">
      <section>
        <h3>Saved base</h3>
        <pre>{comparison.base}</pre>
      </section>
      <section>
        <h3>Local edits</h3>
        <pre>{compareLocal}</pre>
      </section>
      <section>
        <h3>Current disk</h3>
        <pre>{comparison.disk}</pre>
      </section>
    </div>{/if}
  <div class="dialog-actions">
    <button onclick={() => compareDialog.close()}>Keep local</button><button
      onclick={() => {
        compareDialog.close();
        void reload();
      }}
      disabled={!realDocument()}>Reload disk</button
    ><button
      onclick={() => {
        compareDialog.close();
        void save(true);
      }}>Save As</button
    >
  </div>
</dialog>
