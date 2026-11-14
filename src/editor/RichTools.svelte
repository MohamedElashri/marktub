<script lang="ts">
  import type { EditorView } from "@codemirror/view";
  import type { Text } from "@codemirror/state";
  import {
    tableAt,
    tableCell,
    tableShape,
    imageText,
    imageSpan,
    richSpan,
    type Patch,
  } from "./author";
  import { renderRich } from "../renderers/rich";
  import { sanitizeRich } from "../reader/safe";
  import { tick } from "svelte";
  import { onDestroy } from "svelte";
  import { AuthorMath } from "./math";
  import { errorMessage } from "../bridge";
  let {
    getView,
    onimage,
    onerror,
    wholeDocument = true,
  }: {
    getView: () => EditorView | undefined;
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
    onerror: (text: string) => void;
    wholeDocument?: boolean;
  } = $props();
  let dialog: HTMLDialogElement;
  let kind = $state(""),
    body = $state(""),
    path = $state(""),
    alt = $state(""),
    title = $state(""),
    sizing = $state(false),
    width = $state(320),
    height = $state(240),
    writeWidth = false,
    writeHeight = false,
    locked = $state(true),
    ratio = 1;
  let rows = $state<string[][]>([]),
    row = $state(2),
    column = $state(0),
    preview = $state(""),
    error = $state(""),
    zoom = $state(1),
    pending = $state(false);
  let from = 0,
    to = 0,
    original: Text | undefined,
    request = 0,
    before = "",
    after = "";
  let mathPreview: AuthorMath | undefined;
  onDestroy(() => mathPreview?.destroy());
  function capture(at?: number) {
    const v = getView();
    if (!v) throw Error("Editor unavailable.");
    if (v.compositionStarted)
      throw Error("Finish composition before opening block controls.");
    original = v.state.doc;
    from = at ?? v.state.selection.main.from;
    to = at ?? v.state.selection.main.to;
    return v;
  }
  function apply(patch: Patch | Patch[]) {
    const v = getView();
    if (!v || v.state.doc !== original)
      throw Error(
        "The source changed while these controls were open. Reopen the block.",
      );
    const before = v.state.doc;
    v.dispatch({
      changes: patch,
      userEvent: "input.author",
      scrollIntoView: true,
    });
    if (
      v.state.doc === before &&
      (Array.isArray(patch) ? patch : [patch]).some(
        (p) => before.sliceString(p.from, p.to) !== p.insert,
      )
    )
      throw Error(
        "The change was refused. Your draft is kept; check the recovery/input notice.",
      );
    original = v.state.doc;
    v.focus();
  }
  export async function show(
    name: string,
    at?: number,
    initialRow?: number,
    initialColumn?: number,
  ) {
    try {
      const v = capture(at);
      kind = name;
      error = "";
      preview = "";
      zoom = 1;
      if (name === "table") {
        const t = tableAt(v.state, from);
        from = t.from;
        to = t.to;
        rows = t.rows.map((r) => r.map((c) => c.text));
        row = initialRow ?? (rows.length > 2 ? 2 : 0);
        column = initialColumn ?? 0;
        body = rows[row][column];
      } else if (name === "image") {
        const line = v.state.doc.lineAt(from);
        const match = imageSpan(line.text, from - line.from);
        if (match) {
          from = line.from + match.from;
          to = line.from + match.to;
          path = match.path;
          alt = match.alt;
          title = match.title;
          sizing = !!(match.width || match.height);
          width = match.width ?? 320;
          height = match.height ?? 240;
          writeWidth = match.width !== undefined;
          writeHeight = match.height !== undefined;
          locked = !!(match.width && match.height);
        } else {
          if (line.text.includes("!["))
            throw Error("This image spelling remains editable in Source.");
          path = "";
          alt = "";
          title = "";
          sizing = false;
          locked = false;
          writeWidth = writeHeight = true;
        }
        ratio = width / height;
      } else {
        const span = richSpan(v.state, name, from, to);
        if (span) {
          ({ from, to, body } = span);
          before = after = "";
        } else {
          before = name === "mermaid" ? "```mermaid\n" : "$";
          after = name === "mermaid" ? "\n```" : "$";
          body =
            v.state.doc.sliceString(from, to) ||
            (name === "mermaid" ? "graph TD\nA[Start] --> B[Finish]" : "a^2");
        }
      }
      dialog.showModal();
      await tick();
      dialog.querySelector<HTMLElement>("input,textarea,button")?.focus();
    } catch (e) {
      onerror(errorMessage(e));
    }
  }
  function close() {
    ++request;
    mathPreview?.destroy();
    mathPreview = undefined;
    original = undefined;
    preview = "";
    dialog.close();
    getView()?.focus();
  }
  async function image(copy: boolean, offer?: string, bytes?: number[]) {
    const epoch = ++request;
    pending = true;
    try {
      const result = await onimage(copy, offer, bytes);
      if (epoch !== request || !result) return;
      if (getView()?.state.doc !== original)
        throw Error(
          "Image prepared, but source changed. Reopen Image and insert its path. Copied files are kept.",
        );
      path = result.path;
      if (result.width && result.height) {
        width = result.width;
        height = result.height;
        ratio = width / height;
        locked = true;
        writeWidth = writeHeight = true;
      }
      preview = result.dataUrl;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      pending = false;
    }
  }
  export async function dropped(offer: string) {
    await show("image");
    if (dialog.open) await image(true, offer);
  }
  export async function pasted(bytes: number[]) {
    await show("image");
    if (dialog.open) await image(true, undefined, bytes);
  }
  async function render() {
    const epoch = ++request;
    try {
      mathPreview?.destroy();
      mathPreview = undefined;
      const current = () => epoch === request && dialog.open;
      let result: string;
      if (kind === "math" && wholeDocument && original) {
        const inserted = before + body + after;
        mathPreview = new AuthorMath(
          original.sliceString(0, from) + inserted + original.sliceString(to),
        );
        result = await mathPreview.render(
          from + before.length,
          from + before.length + body.length,
          body,
          current,
        );
      } else
        result = await renderRich(
          {
            kind: kind === "mermaid" ? "mermaid" : "math",
            source: body,
            display: true,
          },
          "auto",
          current,
        );
      if (epoch === request) preview = result;
    } catch (e) {
      if (epoch === request && dialog.open) error = errorMessage(e);
    }
  }
  function commit() {
    try {
      if (kind === "table") {
        const v = getView()!;
        apply(tableCell(v.state, from, row, column, body));
        const t = tableAt(v.state, from);
        rows = t.rows.map((r) => r.map((c) => c.text));
        to = t.to;
        body = rows[row]?.[column] ?? "";
      } else {
        apply({
          from,
          to,
          insert:
            kind === "image"
              ? imageText(
                  path,
                  alt,
                  title,
                  sizing && writeWidth ? width : null,
                  sizing && writeHeight ? height : null,
                )
              : before + body + after,
        });
        close();
      }
    } catch (e) {
      error = errorMessage(e);
    }
  }
  function shape(action: string) {
    try {
      const v = getView()!;
      apply(
        tableShape(
          v.state,
          from,
          action,
          ["row-add", "row-delete"].includes(action) ? row : column,
        ),
      );
      const t = tableAt(v.state, from);
      rows = t.rows.map((r) => r.map((c) => c.text));
      to = t.to;
      row = Math.min(row, rows.length - 1);
      column = Math.min(column, t.cols - 1);
      body = rows[row]?.[column] ?? "";
    } catch (e) {
      error = errorMessage(e);
    }
  }
  function resize(event: PointerEvent) {
    if (!sizing) return;
    const start = event.clientX,
      initial = width;
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      width = Math.max(
        1,
        Math.min(4096, Math.round(initial + e.clientX - start)),
      );
      writeWidth = true;
      if (locked) {
        writeHeight = true;
        height = Math.max(1, Math.min(4096, Math.round(width / ratio)));
      }
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
  function moveImage(direction: number) {
    try {
      const v = getView()!;
      const line = v.state.doc.lineAt(from);
      if (line.from !== from || line.to !== to)
        throw Error("Move requires an image on its own line.");
      const next = line.number + direction;
      if (next < 1 || next > v.state.doc.lines) return;
      const other = v.state.doc.line(next);
      const low = direction < 0 ? other : line,
        high = direction < 0 ? line : other;
      apply([
        { from: low.from, to: low.to, insert: high.text },
        { from: high.from, to: high.to, insert: low.text },
      ]);
      close();
    } catch (e) {
      error = errorMessage(e);
    }
  }
  function moveCell(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    event.preventDefault();
    if (body !== rows[row]?.[column]) {
      commit();
      if (error) return;
    }
    let n = row * rows[0].length + column + (event.shiftKey ? -1 : 1);
    n = Math.max(0, Math.min(rows.length * rows[0].length - 1, n));
    row = Math.floor(n / rows[0].length);
    column = n % rows[0].length;
    if (row === 1) row = event.shiftKey ? 0 : 2;
    body = rows[row]?.[column] ?? "";
  }
</script>

<dialog
  bind:this={dialog}
  class="author-dialog"
  aria-label="Edit block"
  oncancel={(e) => {
    e.preventDefault();
    close();
  }}
>
  <header>
    <h2>
      {kind === "table"
        ? "Edit table"
        : kind === "image"
          ? "Image"
          : kind === "mermaid"
            ? "Diagram source"
            : "Math source"}
    </h2>
    <button onclick={close} aria-label="Close block controls">×</button>
  </header>
  {#if kind === "table"}
    <div class="table-editor">
      <table>
        <tbody
          >{#each rows as cells, r (r)}{#if r !== 1}<tr
                >{#each cells as cell, c (c)}<td
                    ><button
                      class:selected={r === row && c === column}
                      onclick={() => {
                        row = r;
                        column = c;
                        body = cell;
                      }}>{cell || "Empty cell"}</button
                    ></td
                  >{/each}</tr
              >{/if}{/each}</tbody
        >
      </table>
    </div>
    <label
      >Cell text<input
        aria-label="Table cell text"
        bind:value={body}
        onkeydown={moveCell}
      /></label
    >
    <p>
      Choose a cell, then Apply cell. Tab / Shift Tab moves between cells. Each
      action is one undo step.
    </p>
    <div class="block-actions">
      {#each [["row-add", "Add row"], ["row-delete", "Delete row"], ["column-add", "Add column"], ["column-delete", "Delete column"], ["left", "Align left"], ["center", "Align center"], ["right", "Align right"]] as [action, label] (action)}<button
          onclick={() => shape(action)}>{label}</button
        >{/each}
    </div>
  {:else if kind === "image"}
    <div class="block-actions">
      <button onclick={() => image(true)} disabled={pending}
        >Choose and copy image</button
      ><button onclick={() => image(false)} disabled={pending}
        >Link selected image</button
      >
    </div>
    <label>Local path<input aria-label="Image path" bind:value={path} /></label
    ><label
      >Alt text<input aria-label="Image alt text" bind:value={alt} /></label
    ><label>Title<input aria-label="Image title" bind:value={title} /></label>
    <label
      ><input
        type="checkbox"
        bind:checked={sizing}
        onchange={() => {
          if (sizing) writeWidth = writeHeight = true;
        }}
      />Write optional size attributes</label
    >{#if sizing}<div class="block-actions">
        <label
          >Width<input
            type="number"
            min="1"
            max="4096"
            bind:value={width}
            oninput={() => {
              writeWidth = true;
              if (locked) {
                writeHeight = true;
                height = Math.round(width / ratio);
              }
            }}
          /></label
        ><label
          >Height<input
            type="number"
            min="1"
            max="4096"
            bind:value={height}
            oninput={() => {
              writeHeight = true;
              if (locked) {
                writeWidth = true;
                width = Math.round(height * ratio);
              }
            }}
          /></label
        ><label><input type="checkbox" bind:checked={locked} />Lock ratio</label
        >
      </div>{/if}
    {#if preview.startsWith("data:image/")}<img
        class="author-image"
        src={preview}
        {alt}
      />{/if}
    <div class="block-actions">
      <button onpointerdown={resize} disabled={!sizing}>Drag to resize</button
      ><button onclick={() => moveImage(-1)}>Move block up</button><button
        onclick={() => moveImage(1)}>Move block down</button
      >
    </div>
    <p>
      Choose a new copy destination; an existing assets folder is proposed. Undo
      removes the reference, never the copied file. Images outside authorized
      roots need selection again in Read.
    </p>
  {:else}<label
      >Source<textarea aria-label="Block source" rows="8" bind:value={body}
      ></textarea></label
    >
    <div class="block-actions">
      <button onclick={render}>Preview locally</button><button
        onclick={() => (zoom = Math.max(0.5, zoom - 0.25))}>Zoom out</button
      ><button onclick={() => (zoom = Math.min(3, zoom + 0.25))}>Zoom in</button
      >
    </div>
    {#if preview}<div class="author-render" style={`zoom:${zoom}`}>
        <!-- eslint-disable svelte/no-at-html-tags -->{@html sanitizeRich(
          preview,
          "author-preview",
        )}<!-- eslint-enable svelte/no-at-html-tags -->
      </div>{/if}{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  <footer>
    <button onclick={close}>Close</button><button
      class="primary"
      onclick={commit}
      disabled={pending}
      >{kind === "table" ? "Apply cell" : "Apply block"}</button
    >
  </footer>
</dialog>
