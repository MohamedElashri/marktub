<script lang="ts">
  import { readEditor } from "./read";
  import { onMount } from "svelte";
  import {
    Annotation,
    Compartment,
    EditorState,
    type ChangeSet,
    type EditorSelection,
    type Transaction,
  } from "@codemirror/state";
  import {
    EditorView,
    keymap,
    lineNumbers,
    drawSelection,
  } from "@codemirror/view";
  import { visualBlocks, rememberImage } from "./blocks";
  import RichTools from "./RichTools.svelte";
  import { format } from "./author";
  import {
    searchKeymap,
    search,
    openSearchPanel,
    selectNextOccurrence,
  } from "@codemirror/search";
  import { indentWithTab } from "@codemirror/commands";
  import {
    foldGutter,
    foldKeymap,
    foldCode,
    unfoldCode,
  } from "@codemirror/language";
  import { rectangularSelection, crosshairCursor } from "@codemirror/view";
  import { defaultKeymap } from "@codemirror/commands";
  import {
    defaultHighlightStyle,
    HighlightStyle,
    syntaxHighlighting,
  } from "@codemirror/language";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { liveFormatting } from "./visual";
  import { applyEdits, flushEdits, errorMessage } from "../bridge";
  import { EditQueue, MAX_QUEUE } from "../document/queue";
  import type { Change, EditDescriptor } from "../document/protocol";
  const syntaxColors: Record<string, string> = {
    "#708": "var(--keyword)",
    "#085": "var(--string)",
    "#164": "var(--number)",
    "#219": "var(--keyword)",
    "#a11": "var(--string)",
    "#05a": "var(--number)",
    "#30a": "var(--keyword)",
    "#940": "var(--number)",
    "#00c": "var(--accent)",
  };
  type Entry = {
    operation: string;
    inverse: ChangeSet;
    selection: EditorSelection;
  };
  const override = Annotation.define<{
    changes: Change[];
    destination: "undo" | "redo";
  }>();
  let {
    session,
    text,
    visual,
    tabWidth = 4,
    wrap = true,
    flushMs = 500,
    presentation = "",
    onchange,
    onready,
    onerror,
    onanchor = () => {},
    onimage,
    spellcheck = false,
    focus = false,
    numbers = true,
  }: {
    session: Omit<EditDescriptor, "text">;
    text: string;
    visual: boolean;
    tabWidth?: number;
    wrap?: boolean;
    flushMs?: number;
    presentation?: string;
    onchange: (queue: EditQueue) => void;
    onready: () => void;
    onerror: (message: string) => void;
    onanchor?: (line: number) => void;
    onimage: (
      copy: boolean,
      offer?: string,
      bytes?: number[],
    ) => Promise<{
      path: string;
      dataUrl: string;
      width?: number;
      height?: number;
    } | null>;
    spellcheck?: boolean;
    focus?: boolean;
    numbers?: boolean;
  } = $props();
  let host: HTMLDivElement;
  let tools: RichTools;
  let lastLine = 0;
  let dropLine = $state<number | null>(null);
  let dragging:
    | { doc: import("@codemirror/state").Text; from: number; to: number }
    | undefined;
  let view: EditorView | undefined;
  let queue: EditQueue;
  let undoStack: Entry[] = [],
    redoStack: Entry[] = [];
  let undoing = false;
  let externalLocked = false;
  let lastEditable = true;
  let logicalBytes = 0;
  const encodedLength = (value: string) =>
    new TextEncoder().encode(value).length;
  const formatting = new Compartment(),
    editable = new Compartment(),
    options = new Compartment();
  function changes(transaction: Transaction): Change[] {
    const replacement = transaction.annotation(override);
    if (replacement) return replacement.changes;
    const result: Change[] = [];
    transaction.changes.iterChanges((from, to, _fromB, _toB, inserted) => {
      result.push({
        from: from + (session.range?.from ?? 0),
        to: to + (session.range?.from ?? 0),
        insert: { kind: "text", text: inserted.toString() },
      });
    });
    return result;
  }
  function valid(changes: Change[]) {
    for (const c of changes)
      if (c.insert.kind === "text")
        for (const scalar of c.insert.text) {
          const point = scalar.codePointAt(0)!;
          if (point === 0 || (point >= 0xd800 && point <= 0xdfff)) return false;
        }
    return true;
  }
  function notify() {
    if (!queue) return;
    onchange(queue);
    const enabled =
      !externalLocked && !undoing && queue.bytes < MAX_QUEUE - 4096;
    if (enabled !== lastEditable) {
      lastEditable = enabled;
      queueMicrotask(() =>
        view?.dispatch({
          effects: editable.reconfigure(EditorView.editable.of(enabled)),
        }),
      );
    }
  }
  export function action(kind: string) {
    if (!view || view.compositionStarted) return;
    try {
      if (kind === "fold") foldCode(view);
      else if (kind === "unfold") unfoldCode(view);
      else if (kind === "find") openSearchPanel(view);
      else if (kind === "next-match") selectNextOccurrence(view);
      else if (["table", "image", "math", "mermaid"].includes(kind))
        void tools.show(kind);
      else {
        view.dispatch({
          ...format(view.state, kind),
          userEvent: "input.format",
        });
        view.focus();
      }
    } catch (e) {
      onerror(errorMessage(e));
    }
  }
  let byteMapDoc: unknown,
    byteMap: number[] = [];
  function mapLines() {
    if (!view) return;
    if (byteMapDoc === view.state.doc) return;
    byteMapDoc = view.state.doc;
    byteMap = [0];
    let bytes = 0;
    const encoder = new TextEncoder();
    for (const line of view.state.doc.iterLines()) {
      bytes += encoder.encode(line).length + 1;
      byteMap.push(bytes);
    }
  }
  export function byteAtLine(line: number) {
    mapLines();
    return byteMap[Math.max(0, line - 1)] ?? 0;
  }
  export function lineAtByte(byte: number) {
    mapLines();
    let low = 0,
      high = byteMap.length - 1;
    while (low < high) {
      const mid = Math.ceil((low + high) / 2);
      if (byteMap[mid] <= byte) low = mid;
      else high = mid - 1;
    }
    return low + 1;
  }
  export function revealLine(number: number, focusEditor = false) {
    if (!view) return;
    const line = view.state.doc.line(
      Math.max(1, Math.min(number, view.state.doc.lines)),
    );
    lastLine = line.number;
    if (focusEditor) view.focus();
    view.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: "start" }),
    });
  }
  export function assetDrop(id: string) {
    void tools.dropped(id);
  }
  export function settle() {
    view?.dispatch({});
  }
  export function lock(value: boolean) {
    externalLocked = value;
    notify();
  }
  export function clearUndo() {
    undoStack = [];
    redoStack = [];
    queue.receipts.clear();
  }
  export function controller() {
    return queue;
  }
  export function reader() {
    if (!view) throw new Error("Editor unavailable.");
    return readEditor(view.state.doc);
  }
  export function snapshot() {
    return view?.state.doc.toString() ?? "";
  }
  export function sizeBytes() {
    return logicalBytes;
  }
  export function length() {
    return view?.state.doc.length ?? null;
  }
  export function caret() {
    return (session.range?.from ?? 0) + (view?.state.selection.main.head ?? 0);
  }
  export function preview() {
    let value = view?.state.doc.sliceString(0, 65536) ?? "";
    while (new TextEncoder().encode(value).length > 65536)
      value = value.slice(0, Math.floor(value.length * 0.9));
    if (
      value.length &&
      value.charCodeAt(value.length - 1) >= 0xd800 &&
      value.charCodeAt(value.length - 1) <= 0xdbff
    )
      value = value.slice(0, -1);
    return value;
  }
  export function undoPending() {
    return undoing;
  }
  export async function undo(redo = false) {
    if (undoing || !view || !(redo ? redoStack : undoStack).length) return;
    undoing = true;
    notify();
    try {
      await queue.flush();
      const stack = redo ? redoStack : undoStack;
      const entry = stack[stack.length - 1];
      const receipt = queue.receipts.get(entry.operation);
      if (!receipt?.inverse.length)
        throw new Error(
          "Undo provenance is unavailable. Text was kept; save an independent copy before reopening.",
        );
      stack.pop();
      view.dispatch({
        changes: entry.inverse,
        selection: entry.selection,
        annotations: override.of({
          changes: receipt.inverse,
          destination: redo ? "undo" : "redo",
        }),
        scrollIntoView: true,
      });
    } catch (error) {
      onerror(errorMessage(error));
    } finally {
      undoing = false;
      notify();
      view?.focus();
    }
  }
  onMount(() => {
    const block = (event: Event) => {
      const detail = (
        event as CustomEvent<{
          kind: string;
          from: number;
          row?: number;
          column?: number;
        }>
      ).detail;
      void tools.show(detail.kind, detail.from, detail.row, detail.column);
    };
    host.addEventListener("author-block", block);
    const drag = (event: Event) => {
      if (!view) return;
      const from = (event as CustomEvent<{ from: number }>).detail.from;
      const line = view.state.doc.lineAt(from);
      if (line.from !== from) return;
      dragging = {
        doc: view.state.doc,
        from: line.from,
        to: Math.min(view.state.doc.length, line.to + 1),
      };
    };
    host.addEventListener("author-dragstart", drag);

    logicalBytes = encodedLength(text);
    queue = new EditQueue(
      session,
      {
        apply: (batches, retained) =>
          applyEdits(session.id, session.generation, batches, retained),
        flush: () => flushEdits(session.id, session.generation),
      },
      notify,
    );
    queue.retained = () => [...undoStack, ...redoStack].map((e) => e.operation);
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: text,
        selection: { anchor: Math.min(session.caret, text.length) },
        extensions: [
          markdown({ base: markdownLanguage, completeHTMLTags: false }),
          EditorState.allowMultipleSelections.of(true),
          rectangularSelection(),
          crosshairCursor(),
          foldGutter(),
          search({ top: true }),
          syntaxHighlighting(
            HighlightStyle.define(
              defaultHighlightStyle.specs.map((spec) => ({
                ...spec,
                color: spec.color
                  ? (syntaxColors[String(spec.color)] ?? "var(--quiet)")
                  : undefined,
              })),
            ),
          ),

          drawSelection(),
          options.of([
            EditorState.tabSize.of(tabWidth),
            ...(numbers ? [lineNumbers()] : []),
            ...(wrap ? [EditorView.lineWrapping] : []),
          ]),
          editable.of(EditorView.editable.of(true)),
          keymap.of([
            {
              key: "Mod-z",
              run: () => {
                void undo();
                return true;
              },
            },
            {
              key: "Mod-Shift-z",
              run: () => {
                void undo(true);
                return true;
              },
            },
            {
              key: "Ctrl-y",
              run: () => {
                void undo(true);
                return true;
              },
            },
            {
              key: "Mod-b",
              run: () => {
                action("bold");
                return true;
              },
            },
            {
              key: "Mod-i",
              run: () => {
                action("italic");
                return true;
              },
            },
            {
              key: "Mod-k",
              run: () => {
                action("link");
                return true;
              },
            },
            ...searchKeymap,
            ...foldKeymap,
            indentWithTab,
            ...defaultKeymap,
          ]),
          EditorView.domEventHandlers({
            dragover: (e, v) => {
              if (
                !dragging ||
                !e.dataTransfer?.types.includes("application/x-marktub-block")
              )
                return false;
              e.preventDefault();
              const at = v.posAtCoords({ x: e.clientX, y: e.clientY });
              dropLine = at === null ? null : v.state.doc.lineAt(at).number;
              return true;
            },
            dragleave: () => {
              dropLine = null;
              return false;
            },
            drop: (e, v) => {
              if (
                !dragging ||
                !e.dataTransfer?.types.includes("application/x-marktub-block")
              )
                return false;
              e.preventDefault();
              const d = dragging;
              dragging = undefined;
              const line = dropLine;
              dropLine = null;
              if (d.doc !== v.state.doc) {
                onerror("Source changed during drag; block was kept.");
                return true;
              }
              if (!line) return true;
              const at = v.state.doc.line(line).from;
              if (at >= d.from && at <= d.to) return true;
              const text =
                v.state.doc.sliceString(d.from, d.to).replace(/\n$/, "") + "\n";
              const changes = [
                { from: d.from, to: d.to, insert: "" },
                { from: at, to: at, insert: text },
              ].sort((a, b) => a.from - b.from);
              v.dispatch({ changes, userEvent: "input.move-block" });
              return true;
            },
            paste: (event) => {
              const file = Array.from(event.clipboardData?.files ?? []).find(
                (f) => /^image\/(png|jpeg)$/.test(f.type),
              );
              if (!file) return false;
              event.preventDefault();
              if (file.size > 2 * 1024 * 1024) {
                onerror(
                  "Clipboard image exceeds 2 MiB; choose the file instead.",
                );
                return true;
              }
              void file
                .arrayBuffer()
                .then((b) => tools.pasted(Array.from(new Uint8Array(b))))
                .catch((e) => onerror(String(e)));
              return true;
            },
            scroll: () => {
              if (view && !focus) {
                const at = view.viewport.from;
                const line = view.state.doc.lineAt(at).number;
                if (line !== lastLine) {
                  lastLine = line;
                  onanchor(line);
                }
              }
              return false;
            },
            beforeinput: (event) => {
              if (
                event.inputType === "historyUndo" ||
                event.inputType === "historyRedo"
              ) {
                event.preventDefault();
                void undo(event.inputType === "historyRedo");
                return true;
              }
              return false;
            },
          }),
          EditorState.transactionFilter.of((transaction) => {
            if (!transaction.docChanged) return transaction;
            const wire = changes(transaction);
            let size = logicalBytes;
            transaction.changes.iterChanges((from, to, _a, _b, inserted) => {
              size +=
                encodedLength(inserted.toString()) -
                encodedLength(transaction.startState.doc.sliceString(from, to));
            });
            if (
              size > (session.range ? 1024 * 1024 : 32 * 1024 * 1024) ||
              !valid(wire) ||
              !queue.canAccept(wire)
            ) {
              onerror(
                "Input was refused because it exceeds the recovery budget or contains invalid Unicode. Existing text is unchanged; retry recovery or use a smaller paste.",
              );
              return [];
            }
            return transaction;
          }),
          EditorView.contentAttributes.of(() => ({
            "aria-label": session.range
              ? "Range Markdown editor"
              : "Source Markdown editor",
            spellcheck: String(spellcheck),
          })),
          EditorView.updateListener.of((update) => {
            if (update.selectionSet && !update.docChanged) notify();
            if (update.selectionSet && !update.view.composing) {
              const line = update.state.doc.lineAt(
                update.state.selection.main.head,
              ).number;
              if (line !== lastLine) {
                lastLine = line;
                onanchor(line);
              }
            }
            for (const transaction of update.transactions)
              if (transaction.docChanged) {
                transaction.changes.iterChanges(
                  (from, to, _a, _b, inserted) => {
                    logicalBytes +=
                      encodedLength(inserted.toString()) -
                      encodedLength(
                        transaction.startState.doc.sliceString(from, to),
                      );
                  },
                );
                const operation = queue.enqueue(changes(transaction));
                const entry = {
                  operation,
                  inverse: transaction.changes.invert(
                    transaction.startState.doc,
                  ),
                  selection: transaction.startState.selection,
                };
                const replacement = transaction.annotation(override);
                if (replacement)
                  (replacement.destination === "undo"
                    ? undoStack
                    : redoStack
                  ).push(entry);
                else {
                  undoStack.push(entry);
                  redoStack = [];
                }
                if (undoStack.length > 1000) undoStack.shift();
                notify();
              }
          }),
          formatting.of(visual ? [liveFormatting, visualBlocks] : []),
        ],
      }),
    });
    notify();
    onready();
    return () => {
      host.removeEventListener("author-block", block);
      host.removeEventListener("author-dragstart", drag);
      queue.dispose();
      view?.destroy();
      view = undefined;
    };
  });
  $effect(() => {
    const enabled = visual,
      width = tabWidth,
      wrapping = wrap;
    void presentation;
    void spellcheck;
    void focus;
    if (queue) queue.batchDelay = flushMs;
    view?.requestMeasure();
    view?.dispatch({
      effects: [
        formatting.reconfigure(enabled ? [liveFormatting, visualBlocks] : []),
        options.reconfigure([
          EditorState.tabSize.of(width),
          ...(numbers ? [lineNumbers()] : []),
          ...(wrapping ? [EditorView.lineWrapping] : []),
        ]),
      ],
    });
  });
</script>

{#if dropLine !== null}<p class="author-drop-hint" role="status">
    Drop image block before line {dropLine}
  </p>{/if}
<div class="editor-host" class:writing-focus={focus} bind:this={host}></div>
<RichTools
  bind:this={tools}
  getView={() => view}
  onimage={async (copy, offer, bytes) => {
    const result = await onimage(copy, offer, bytes);
    if (result && view) rememberImage(view, result.path, result.dataUrl);
    return result;
  }}
  {onerror}
/>
