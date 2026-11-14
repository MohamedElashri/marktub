<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { Descriptor } from "./bridge";
  import { errorMessage } from "./bridge";
  import * as api from "./library";
  let {
    onopen,
    onrecent,
    onrestore,
    onrecentchange,
  }: {
    onopen: (d: Descriptor) => Promise<void>;
    onrecent: (h: api.Hint) => Promise<void>;
    onrestore: (hints: api.Hint[], active: number) => Promise<void>;
    onrecentchange: (hints: api.Hint[]) => void;
  } = $props();
  let libraryState = $state<api.LibraryState | null>(null);
  let current = $state<api.Folder | null>(null),
    trail = $state<api.Folder[]>([]),
    known = $state<api.Folder[]>([]);
  let gitBadges = $state<Record<string, string>>({});
  async function refreshGitBadges(items: api.Item[]) {
    const ids = items.map((i) => i.id);
    try {
      const result = await api.gitBadges(ids);
      if (page?.items.map((i) => i.id).join(":") === ids.join(":"))
        gitBadges = result;
    } catch {
      gitBadges = {};
    }
  }
  let page = $state<api.Page | null>(null),
    offset = $state(0),
    sort = $state("name"),
    filter = $state("all"),
    hidden = $state(false),
    ignored = $state(false),
    view = $state("list");
  let selected = $state<string[]>([]),
    message = $state(""),
    busy = $state(false);
  let name = $state(""),
    destination = $state("");
  let review = $state<api.Review | null>(null),
    progress = $state<api.Progress | null>(null);
  let query = $state(""),
    content = $state(false);
  let preview = $state(""),
    previewName = $state(""),
    previewHtml = $state("");
  let previewDialog: HTMLDialogElement,
    reviewDialog: HTMLDialogElement,
    quickDialog: HTMLDialogElement;
  let queryInput: HTMLInputElement;
  let timer: ReturnType<typeof setTimeout> | undefined,
    listingTimer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0,
    pollGeneration = 0,
    mounted = true;
  let mutation: {
    id: string;
    folder: string | undefined;
    selection: string;
    kind: string;
  } | null = null;
  let drops = $state<api.DropOffer[]>([]);
  let dropDialog: HTMLDialogElement;
  export function dropped(offers: api.DropOffer[]) {
    drops = offers;
    dropDialog.showModal();
  }
  async function accept(offer: api.DropOffer) {
    await run(async () => {
      const result = await api.acceptDrop(
        offer.id,
        offer.kind === "folder" ? "folder" : "open",
      );
      if (result.document) await onopen(result.document);
      if (result.folder) {
        await refreshState();
        await enter(result.folder, []);
      }
      drops = drops.filter((d) => d.id !== offer.id);
      if (!drops.length) dropDialog.close();
    });
  }
  export function viewName() {
    return view;
  }
  export function quickOpen() {
    quickDialog.showModal();
    queryInput.focus();
  }
  async function refreshState(selectMissing = true) {
    libraryState = await api.state();
    onrecentchange(libraryState.recent);
    if (libraryState.job && progress?.id !== libraryState.job)
      void poll(libraryState.job);
    if (selectMissing && !current && libraryState.roots[0])
      await enter(libraryState.roots[0], []);
    known = [
      ...known,
      ...libraryState.roots.filter((r) => !known.some((k) => k.id === r.id)),
    ].slice(-64);
    if (libraryState.warning) message = libraryState.warning;
  }
  async function run(action: () => Promise<void>) {
    try {
      await action();
    } catch (error) {
      message = errorMessage(error);
    }
  }
  async function addRoot() {
    busy = true;
    await run(async () => {
      const saved = (await api.state()).display;
      const folder = await api.chooseRoot();
      if (folder) {
        await refreshState(false);
        sort = saved.sort;
        hidden = saved.hidden;
        ignored = saved.ignored;
        filter = saved.filter;
        await enter(folder, [], saved.offset);
      }
    });
    busy = false;
  }
  export async function chooseFolder() {
    await addRoot();
  }
  async function load(refresh = false) {
    if (!current) return;
    const request = ++generation;
    clearTimeout(listingTimer);
    try {
      const result = await api.page(
        current.id,
        offset,
        sort,
        hidden,
        filter,
        refresh,
        ignored,
      );
      if (request !== generation || !mounted) return;
      page = result;
      void refreshGitBadges(result.items);
      if (result.complete) {
        if (offset >= result.total && offset > 0) {
          offset = Math.floor(Math.max(0, result.total - 1) / 100) * 100;
          await load();
          return;
        }
        await api.location(current.id, {
          offset,
          sort,
          hidden,
          ignored,
          filter,
        });
      }
      if (!result.complete) listingTimer = setTimeout(() => void load(), 80);
    } catch (error) {
      if (request === generation) message = errorMessage(error);
    }
  }
  async function enter(
    folder: api.Folder,
    ancestors = trail,
    restoreOffset = 0,
  ) {
    current = folder;
    trail = ancestors;
    offset = restoreOffset;
    selected = [];
    destination = folder.id;
    if (!known.some((d) => d.id === folder.id))
      known = [...known, folder].slice(-64);
    await load(true);
  }
  function toggle(item: api.Item) {
    selected = selected.includes(item.id)
      ? selected.filter((id) => id !== item.id)
      : [...selected, item.id];
  }
  async function open(item: api.Item) {
    if (item.directory) {
      await enter(
        { id: item.directory, name: item.name, root: current?.root ?? "" },
        current ? [...trail, current] : [],
      );
      return;
    }
    if (item.kind !== "file") return;
    busy = true;
    await run(async () => {
      await onopen(await api.open(item.id));
      await refreshState();
    });
    busy = false;
  }
  async function showPreview(item: api.Item) {
    if (item.kind !== "file") return;
    previewName = item.name;
    preview = "Loading preview…";
    previewHtml = "";
    previewDialog.showModal();
    await run(async () => {
      preview = await api.preview(item.id);
      const [{ renderSection }, { sanitizePreview }] = await Promise.all([
        import("./reader/render"),
        import("./reader/sanitize"),
      ]);
      previewHtml = sanitizePreview(renderSection(preview).html);
    });
  }
  async function prepare(kind: string) {
    message = "";
    await run(async () => {
      review = await api.prepare({
        kind,
        entries: kind.startsWith("create") ? [] : selected,
        destination: kind === "trash" ? null : destination,
        name: name || null,
      });
      reviewDialog.showModal();
    });
  }
  async function poll(id: string, request = ++pollGeneration) {
    clearTimeout(timer);
    try {
      const next = await api.job(id);
      if (!mounted || request !== pollGeneration) return;
      progress = next;
      if (next.running) timer = setTimeout(() => void poll(id, request), 150);
      else {
        await refreshState();
        if (!mounted || request !== pollGeneration) return;
        const ownsSelection =
          mutation?.id === id &&
          mutation.folder === current?.id &&
          mutation.selection === JSON.stringify(selected);
        if (next.selection.length && ownsSelection) {
          await enter(next.selection[0].parent, []);
          selected = next.selection.map((item) => item.id);
        } else {
          if (ownsSelection && mutation?.kind === "trash") selected = [];
          await load(true);
        }
        if (mutation?.id === id) mutation = null;
      }
    } catch (error) {
      if (!mounted || request !== pollGeneration) return;
      try {
        // A completed job can be replaced before its last UI poll arrives.
        // Reconcile with native state instead of leaving a stale Working state.
        const state = await api.state();
        if (!mounted || request !== pollGeneration) return;
        if (state.job && state.job !== id) await poll(state.job);
        else if (!state.job) {
          progress = null;
          if (mutation?.id === id) mutation = null;
          await refreshState(false);
          await load(true);
        } else message = errorMessage(error);
      } catch (reconcileError) {
        if (mounted && request === pollGeneration)
          message = errorMessage(reconcileError);
      }
    }
  }
  async function commit() {
    if (!review) return;
    message = "";
    await run(async () => {
      const context = {
        folder: current?.id,
        selection: JSON.stringify(selected),
        kind: review!.kind,
      };
      const id = await api.commit(review!.id);
      mutation = { id, ...context };
      reviewDialog.close();
      review = null;
      await poll(id);
    });
  }
  async function scan() {
    await run(async () => {
      const id = await api.scan(query, content, hidden, ignored);
      quickDialog.close();
      await poll(id);
    });
  }
  async function changeView() {
    await run(() => api.configure(true, view));
  }
  onMount(() => {
    mounted = true;
    void run(async () => {
      const s = await api.state();
      view = s.view;
      await refreshState();
    });
    const focus = () => {
      if (!progress?.running) void load(true);
    };
    window.addEventListener("focus", focus);
    const gitListener = listen("git-updated", () => {
      if (page) void refreshGitBadges(page.items);
    });
    return () => {
      mounted = false;
      generation++;
      pollGeneration++;
      clearTimeout(timer);
      clearTimeout(listingTimer);
      window.removeEventListener("focus", focus);
      void gitListener.then((unlisten) => unlisten());
    };
  });
</script>

<div class="library-panel">
  <div class="library-title">
    <h2>Library</h2>
    <button class="folder-button" onclick={addRoot} disabled={busy}
      >Add folder</button
    >
  </div>
  <p class="library-help">Local folders. Reselect after restarting.</p>
  {#if libraryState?.rootHints.length && !libraryState.roots.length}<p
      class="library-help"
    >
      Remembered folders: {libraryState.rootHints
        .map((r) => r.name)
        .join(", ")}. Use Add folder to reselect.
    </p>{/if}
  <div class="root-list" aria-label="Authorized folders">
    {#each libraryState?.roots ?? [] as root (root.id)}<div>
        <button
          aria-pressed={current?.root === root.id}
          title={root.label}
          onclick={() => enter(root, [])}>▱ {root.name}</button
        ><button
          aria-label={`Remove root ${root.name}`}
          onclick={() =>
            run(async () => {
              await api.removeRoot(root.id);
              if (current?.root === root.id) {
                current = null;
                page = null;
                trail = [];
              }
              await refreshState();
            })}>×</button
        >
      </div>{/each}
  </div>
  {#if message}<p role="status" class="library-message">
      {message}<button
        aria-label="Dismiss library message"
        onclick={() => (message = "")}>×</button
      >
    </p>{/if}
  {#if current}
    <nav class="breadcrumbs" aria-label="Folder breadcrumbs">
      {#each trail as folder, index (folder.id)}<button
          onclick={() => enter(folder, trail.slice(0, index))}
          >{folder.name}</button
        ><span>/</span>{/each}<strong>{current.name}</strong>
    </nav>
    <div class="library-controls">
      <label
        >Sort<select
          aria-label="Sort files"
          bind:value={sort}
          onchange={() => {
            offset = 0;
            void load();
          }}
          ><option value="name">Name</option><option value="modified"
            >Modified</option
          ><option value="size">Size</option><option value="type">Type</option
          ></select
        ></label
      >
      <label
        >Show<select
          aria-label="File type"
          bind:value={filter}
          onchange={() => {
            offset = 0;
            void load();
          }}
          ><option value="all">All files</option><option value="text"
            >Markdown / text</option
          ><option value="images">Images</option></select
        ></label
      >
      <label
        >View<select
          aria-label="Library view"
          bind:value={view}
          onchange={changeView}
          ><option value="list">List</option><option value="compact"
            >Compact</option
          ><option value="grid">Grid</option></select
        ></label
      >
      <label
        ><input
          type="checkbox"
          bind:checked={hidden}
          onchange={() => {
            offset = 0;
            void load();
          }}
        />Hidden</label
      >
      <label
        ><input
          type="checkbox"
          bind:checked={ignored}
          onchange={() => {
            offset = 0;
            void load(true);
          }}
        />Ignored</label
      >
      <button onclick={() => load(true)}>Refresh</button><button
        onclick={quickOpen}>Search folders</button
      >
    </div>
    <details class="library-policy">
      <summary>Exclusions &amp; counts</summary>
      <p class="library-help">
        .git and symlink traversal are excluded. “Ignored” uses the root
        .gitignore; nested rules are not applied. Counts include discovered
        entries only.
      </p>
    </details>
    <p class="listing-status" role="status">
      {page?.total ?? 0} visible / {page?.scanned ?? 0} discovered{page?.complete
        ? " · listing complete"
        : " · scanning; order is provisional"}
    </p>
    <div class={`file-list ${view}`} role="list" aria-label="Folder entries">
      {#each page?.items ?? [] as item (item.id)}<div
          class="file-row"
          role="listitem"
          class:selected={selected.includes(item.id)}
        >
          <input
            aria-label={`Select ${item.name}`}
            type="checkbox"
            checked={selected.includes(item.id)}
            onchange={() => toggle(item)}
          />
          <button
            class="file-name"
            title={item.name}
            disabled={item.kind === "link" || item.kind === "special"}
            ondblclick={() => open(item)}
            onclick={() => {
              if (item.directory) void open(item);
              else selected = [item.id];
            }}
            onkeydown={(event) => {
              if (event.key === " ") {
                event.preventDefault();
                void showPreview(item);
              }
              if (event.key === "Enter" && !item.directory) {
                event.preventDefault();
                void open(item);
              }
            }}
            ><span aria-hidden="true"
              >{item.directory ? "▱" : item.kind === "link" ? "↗" : "≡"}</span
            >{item.name}{#if gitBadges[item.id]}<small
                class="library-git-badge"
                title={`Git at last refresh: ${gitBadges[item.id]}`}>Git</small
              >{/if}</button
          >
          <span class="file-meta"
            >{item.kind === "folder"
              ? "Folder"
              : `${item.bytes.toLocaleString()} B`}<br />{new Date(
              item.modified * 1000,
            ).toLocaleDateString()}</span
          >
          {#if item.kind === "file"}<button
              aria-label={`Preview ${item.name}`}
              onclick={() => showPreview(item)}>Space</button
            ><button
              aria-label={`Open ${item.name}`}
              onclick={() => open(item)}
              disabled={busy}>Open</button
            >{/if}
        </div>{/each}
    </div>
    <div class="pagination">
      <button
        disabled={offset === 0}
        onclick={() => {
          offset = Math.max(0, offset - 100);
          void load();
        }}>Previous</button
      ><span>{offset + 1}–{Math.min(offset + 100, page?.total ?? 0)}</span
      ><button
        disabled={offset + 100 >= (page?.total ?? 0)}
        onclick={() => {
          offset += 100;
          void load();
        }}>Next</button
      >
    </div>
    <details class="file-actions">
      <summary>Manage files ({selected.length} selected)</summary>
      <p>
        Close Source tabs before changing files. Completed operations are
        separate from text undo.
      </p>
      <label
        >Name<input
          aria-label="New filename"
          bind:value={name}
          placeholder="notes.txt"
          maxlength="240"
        /></label
      >
      <label
        >Destination<select
          aria-label="Operation destination"
          bind:value={destination}
          >{#each known.filter( (d) => libraryState?.roots.some((r) => r.id === d.root) ) as folder (folder.id)}<option
              value={folder.id}>{folder.name}</option
            >{/each}</select
        ></label
      >
      <div>
        <button
          disabled={busy || progress?.running}
          onclick={() => prepare("createFile")}>New file</button
        ><button
          disabled={busy || progress?.running}
          onclick={() => prepare("createFolder")}>New folder</button
        ><button
          disabled={selected.length !== 1 || busy || progress?.running}
          onclick={() => prepare("rename")}>Rename</button
        ><button
          disabled={!selected.length || busy || progress?.running}
          onclick={() => prepare("copy")}>Copy</button
        ><button
          disabled={!selected.length || busy || progress?.running}
          onclick={() => prepare("move")}>Move</button
        ><button
          disabled={!selected.length || busy || progress?.running}
          onclick={() => prepare("trash")}>Move to Trash</button
        >
      </div>
    </details>
  {/if}
  {#if progress}<section class="library-job" aria-label="Library job">
      <h3>
        {progress.running
          ? "Working…"
          : progress.canceled
            ? "Canceled"
            : "Results"}
      </h3>
      <p>
        {progress.scanned} scanned, {progress.skipped} excluded. {progress.truncated
          ? "Budget reached; results are partial."
          : ""}
      </p>
      {#if progress.error}<p role="alert">
          {progress.error}
        </p>{/if}{#if progress.running}<button
          onclick={() =>
            run(async () => {
              await api.job(progress!.id, true);
            })}>Cancel job</button
        >{/if}
      {#each progress.results as hit (hit.item.id)}<div class="search-result">
          <button onclick={() => open(hit.item)}>{hit.item.name}</button><button
            aria-label={`Preview result ${hit.item.name}`}
            onclick={() => showPreview(hit.item)}>Preview</button
          >{#if hit.context}<p>{hit.context}</p>{/if}
        </div>{/each}
      {#if progress.selection.length}<h3>Moved selection</h3>
        {#each progress.selection as item (item.id)}<button
            onclick={() => open(item)}>Open {item.name}</button
          >{/each}{/if}
      {#each progress.outcomes as result, index (index)}<p>
          <strong>{result.name}: {result.status}</strong><br />{result.message}
        </p>{/each}
    </section>{/if}
  {#if libraryState?.lastOperation.length && !progress}<details
      class="file-actions"
    >
      <summary>Last file operation</summary
      >{#each libraryState.lastOperation as outcome, index (index)}<p>
          <strong>{outcome.name}: {outcome.status}</strong><br
          />{outcome.message}
        </p>{/each}
    </details>{/if}
  {#if libraryState?.session.length}<section class="recents">
      <h3>Previous session</h3>
      <button
        disabled={libraryState.session.some((h) => !h.authorized)}
        onclick={() => onrestore(libraryState!.session, libraryState!.active)}
        >Restore saved tabs</button
      >
      <p class="library-help">
        Reselect containing folders, then reopen saved tabs. Unsaved text is
        available separately in recovery.
      </p>
      {#each libraryState.session as hint (hint.id)}<button
          onclick={() => onrecent(hint)}
          >{hint.name}{hint.authorized ? "" : " (reselect folder)"}</button
        >{/each}
    </section>{/if}
  {#if libraryState?.recent.length}<section class="recents">
      <h3>Recent &amp; bookmarked</h3>
      {#each libraryState.recent as hint (hint.id)}<div>
          <button
            title={hint.authorized
              ? "Open in a new tab"
              : "Reselect the containing folder first"}
            onclick={() => onrecent(hint)}
            ><span class="recent-file-info"
              ><span
                >{hint.name}{hint.authorized ? "" : " (reselect folder)"}</span
              >{#if hint.context}<span class="recent-context"
                  >{hint.context}</span
                >{/if}</span
            ></button
          ><button
            aria-label={`Bookmark ${hint.name}`}
            aria-pressed={hint.pinned}
            onclick={() =>
              run(async () => {
                await api.pin(hint.id);
                await refreshState();
              })}>{hint.pinned ? "★" : "☆"}</button
          >
        </div>{/each}
    </section>{/if}
</div>
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
  {#if previewHtml}<article>
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html previewHtml}
    </article>{:else}<pre>{preview}</pre>{/if}
</dialog>
<dialog bind:this={reviewDialog} aria-label="Review file operation">
  <h2>Review {review?.kind}</h2>
  {#if message}<p role="alert">{message}</p>{/if}
  <p>{review?.sources.join(", ") || name} → {review?.destination}</p>
  <p>{review?.warning}</p>
  <p>
    Trash uses the system Trash when available; otherwise the source stays in
    place. Restore through the system file manager.
  </p>
  <div class="dialog-actions">
    <button onclick={() => reviewDialog.close()}>Cancel</button><button
      class="primary"
      onclick={commit}>Confirm operation</button
    >
  </div>
</dialog>
<dialog bind:this={quickDialog} aria-label="Quick open and search">
  <h2>Find in authorized folders</h2>
  <form
    onsubmit={(event) => {
      event.preventDefault();
      void scan();
    }}
  >
    <label
      >Filename or text<input
        bind:this={queryInput}
        aria-label="Workspace query"
        bind:value={query}
        maxlength="256"
      /></label
    ><label
      ><input type="checkbox" bind:checked={content} />Search file content
      (literal, case-sensitive)</label
    >
    <p>
      Filename search is fuzzy. Content search covers UTF-8 Markdown/text up to
      8 MiB each, 256 MiB per job and 200 results. Hidden and ignore settings
      apply.
    </p>
    <div class="dialog-actions">
      <button type="button" onclick={() => quickDialog.close()}>Cancel</button
      ><button class="primary" type="submit">Search</button>
    </div>
  </form>
</dialog>

<dialog bind:this={dropDialog} aria-label="Dropped items">
  <h2>Dropped items</h2>
  <p>
    Open selected files or authorize folders for browsing. Files stay at their
    original locations.
  </p>
  {#each drops as offer (offer.id)}<div>
      <span>{offer.name}</span><button onclick={() => accept(offer)}
        >{offer.kind === "folder" ? "Add folder" : "Open file"}</button
      >
    </div>{/each}<button onclick={() => dropDialog.close()}>Close</button>
</dialog>
