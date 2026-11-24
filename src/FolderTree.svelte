<script lang="ts">
  import { untrack } from "svelte";
  import type { SvelteMap } from "svelte/reactivity";
  import FolderTree from "./FolderTree.svelte";
  import * as api from "./library";
  import Icon from "./ui/Icon.svelte";
  import { errorMessage } from "./bridge";
  let {
    folder,
    ancestors = [],
    expanded,
    revision,
    hidden,
    ignored,
    foldersOnly = false,
    currentFolder = null,
    depth = 0,
    onopen,
    onfolder,
  }: {
    folder: api.Folder;
    ancestors?: api.Folder[];
    expanded: SvelteMap<string, string>;
    revision: number;
    hidden: boolean;
    ignored: boolean;
    foldersOnly?: boolean;
    currentFolder?: string | null;
    depth?: number;
    onopen: (item: api.Item) => Promise<void>;
    onfolder: (folder: api.Folder, ancestors: api.Folder[]) => Promise<void>;
  } = $props();
  let page = $state<api.Page | null>(null),
    offset = $state(0),
    busy = $state(false),
    error = $state("");
  let generation = 0,
    timer: ReturnType<typeof setTimeout> | undefined;
  async function load(
    refresh = false,
    showHidden = hidden,
    showIgnored = ignored,
  ) {
    const request = ++generation;
    clearTimeout(timer);
    busy = true;
    error = "";
    try {
      const result = await api.page(
        folder.id,
        offset,
        "name",
        showHidden,
        foldersOnly ? "folders" : "markdown",
        refresh,
        showIgnored,
      );
      if (request !== generation) return;
      page = result;
      if (result.complete && offset >= result.total && offset) {
        offset = 0;
        await load();
        return;
      }
      if (!result.complete)
        timer = setTimeout(() => {
          void load();
        }, 80);
    } catch (e) {
      if (request === generation) error = errorMessage(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  $effect(() => {
    const open = expanded.has(folder.id);
    const display = { revision, hidden, ignored, foldersOnly };
    if (open)
      untrack(() => {
        void load(true, display.hidden, display.ignored);
      });
    return () => {
      generation++;
      clearTimeout(timer);
    };
  });
  async function toggle(event: MouseEvent | KeyboardEvent) {
    if (event.currentTarget instanceof HTMLButtonElement)
      event.currentTarget.focus({ preventScroll: true });
    if (expanded.has(folder.id)) {
      expanded.delete(folder.id);
      return;
    }
    if (depth >= 16 || expanded.size >= 16) {
      error =
        "Collapse another folder, or open this folder as a Library root, to browse further.";
      return;
    }
    expanded.set(folder.id, folder.root);
    await onfolder(folder, ancestors);
  }
</script>

<div class="folder-tree-node">
  <button
    class="tree-folder"
    aria-current={folder.id === currentFolder ? "location" : undefined}
    aria-expanded={expanded.has(folder.id)}
    aria-label={`Folder ${folder.name}`}
    onclick={(event) =>
      foldersOnly && expanded.has(folder.id)
        ? onfolder(folder, ancestors)
        : toggle(event)}
    title={foldersOnly
      ? (folder.label ?? folder.name)
      : "Use Left and Right arrows to collapse or expand"}
    onkeydown={(event) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (
        (event.key === "ArrowLeft" && expanded.has(folder.id)) ||
        (event.key === "ArrowRight" && !expanded.has(folder.id))
      ) {
        event.preventDefault();
        void toggle(event);
      }
    }}
  >
    {#if foldersOnly}<Icon name="folder" />{:else}<span aria-hidden="true"
        >{expanded.has(folder.id) ? "⌄" : "›"}</span
      >{/if}
    <span dir="auto">{folder.name}</span>
  </button>
  {#if foldersOnly}
    <button
      class="folder-disclosure"
      aria-label={`${expanded.has(folder.id) ? "Collapse" : "Expand"} ${folder.name}`}
      aria-expanded={expanded.has(folder.id)}
      onclick={toggle}>{expanded.has(folder.id) ? "⌄" : "›"}</button
    >
  {/if}
  {#if error}<p role="status" class="library-help">{error}</p>{/if}
  {#if expanded.has(folder.id)}
    <ul
      class="folder-tree-entries"
      aria-label={`${foldersOnly ? "Folders" : "Files"} in ${folder.name}`}
    >
      {#each (page?.items ?? []).filter((item) => !foldersOnly || item.directory) as item (item.id)}<li
        >
          {#if item.directory}<FolderTree
              folder={{
                id: item.directory,
                root: folder.root,
                name: item.name,
              }}
              ancestors={[...ancestors, folder]}
              {expanded}
              {revision}
              {hidden}
              {ignored}
              {foldersOnly}
              {currentFolder}
              depth={depth + 1}
              {onopen}
              {onfolder}
            />
          {:else}<button
              class="tree-file"
              aria-label={`Open ${item.name}`}
              disabled={item.kind !== "file" || busy}
              title={item.name}
              onclick={() => onopen(item)}
              ><span aria-hidden="true">≡</span><span>{item.name}</span></button
            >{/if}
        </li>{/each}
    </ul>
    {#if busy || !page?.complete}<p class="library-help" role="status">
        Loading folder…
      </p>{/if}
    {#if page?.complete && !page.items.length && !foldersOnly}<p
        class="library-help"
      >
        No Markdown files or folders.
      </p>{/if}
    {#if offset > 0 || (page?.total ?? 0) > 100}<div class="pagination">
        <button
          disabled={offset === 0 || busy}
          onclick={() => {
            offset = Math.max(0, offset - 100);
            void load();
          }}>Previous files</button
        >
        <span>{offset + 1}–{Math.min(offset + 100, page?.total ?? 0)}</span>
        <button
          disabled={busy || offset + 100 >= (page?.total ?? 0)}
          onclick={() => {
            offset += 100;
            void load();
          }}>Next files</button
        >
      </div>{/if}
  {/if}
</div>
