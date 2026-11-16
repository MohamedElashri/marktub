<script lang="ts">
  import { tick } from "svelte";
  import * as api from "./library";
  import { errorMessage } from "./bridge";
  import { SearchJobs } from "./search";
  import { score } from "./commands";
  let {
    hidden,
    ignored,
    onopen,
    onrecent,
    onpreview,
  }: {
    hidden: boolean;
    ignored: boolean;
    onopen: (item: api.Item) => Promise<void>;
    onrecent: (hint: api.Hint) => Promise<void>;
    onpreview: (item: api.Item) => Promise<void>;
  } = $props();
  let dialog: HTMLDialogElement,
    input: HTMLInputElement,
    resultList: HTMLUListElement;
  let query = $state(""),
    content = $state(false),
    busy = $state(false),
    opening = $state(false),
    error = $state("");
  let progress = $state<api.Progress | null>(null),
    recent = $state<api.Hint[]>([]),
    selected = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined,
    epoch = 0;
  const jobs = new SearchJobs(api);
  const results = $derived(
    [...(progress?.results ?? [])].sort((a, b) =>
      content ? 0 : score(a.item.name, query) - score(b.item.name, query),
    ),
  );
  async function close() {
    const request = ++epoch;
    clearTimeout(timer);
    busy = false;
    opening = false;
    dialog.close();
    try {
      await jobs.stop();
    } catch (e) {
      if (request === epoch) error = errorMessage(e);
    }
  }
  export async function show(searchContent = false) {
    const request = ++epoch;
    clearTimeout(timer);
    opening = false;
    content = searchContent;
    error = "";
    progress = null;
    selected = 0;
    dialog.showModal();
    try {
      const state = await api.state();
      if (request !== epoch || !dialog.open) return;
      recent = state.recent.filter((hint) => hint.authorized).slice(0, 8);
    } catch (e) {
      if (request === epoch) error = errorMessage(e);
    }
    await tick();
    if (request !== epoch || !dialog.open) return;
    input.focus();
    input.select();
    if (query && !content) changed();
  }
  async function search() {
    clearTimeout(timer);
    const request = ++epoch;
    const phrase = content ? query : query.trim();
    progress = null;
    selected = 0;
    error = "";
    busy = true;
    try {
      if (!phrase.trim()) {
        await jobs.stop();
        return;
      }
      const id = await jobs.replace(
        [phrase, content, hidden, ignored],
        () => request === epoch,
      );
      if (!id || request !== epoch) return;
      while (request === epoch) {
        const result = await api.job(id);
        if (request !== epoch) return;
        progress = result;
        selected = Math.min(selected, Math.max(0, result.results.length - 1));
        if (!result.running) break;
        await new Promise((resolve) => setTimeout(resolve, 60));
      }
    } catch (e) {
      if (request === epoch) error = errorMessage(e);
    } finally {
      if (request === epoch) busy = false;
    }
  }
  function changed() {
    const request = ++epoch;
    clearTimeout(timer);
    progress = null;
    selected = 0;
    error = "";
    if (!content && query.trim())
      timer = setTimeout(() => {
        void search();
      }, 200);
    else {
      busy = false;
      void jobs.stop().catch((e) => {
        if (request === epoch) error = errorMessage(e);
      });
    }
  }
  function mode(searchContent: boolean) {
    content = searchContent;
    changed();
    input.focus();
  }
  async function open(item: api.Item) {
    const request = ++epoch;
    busy = true;
    opening = true;
    try {
      await jobs.stop();
      if (request !== epoch) return;
      await onopen(item);
      await close();
    } catch (e) {
      if (request === epoch) error = errorMessage(e);
    } finally {
      if (request === epoch) {
        busy = false;
        opening = false;
      }
    }
  }
  async function openRecent(hint: api.Hint) {
    const request = ++epoch;
    busy = true;
    opening = true;
    try {
      await jobs.stop();
      if (request !== epoch) return;
      await onrecent(hint);
      await close();
    } catch (e) {
      if (request === epoch) error = errorMessage(e);
    } finally {
      if (request === epoch) {
        busy = false;
        opening = false;
      }
    }
  }
  async function key(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (results.length)
        selected =
          (selected + (event.key === "ArrowDown" ? 1 : -1) + results.length) %
          results.length;
      await tick();
      const row = resultList?.children[selected];
      if (row instanceof HTMLElement) {
        const item = row.getBoundingClientRect(),
          viewport = resultList.getBoundingClientRect();
        if (item.top < viewport.top)
          resultList.scrollTop -= viewport.top - item.top;
        else if (item.bottom > viewport.bottom)
          resultList.scrollTop += item.bottom - viewport.bottom;
      }
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (results[selected]) await open(results[selected].item);
      else await search();
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="workspace-search"
  aria-label="Quick open and search"
  oncancel={(event) => {
    event.preventDefault();
    void close();
  }}
>
  <div class="library-title">
    <h2>Find a note or file</h2>
    <button onclick={() => close()}>Close search</button>
  </div>
  <div class="search-modes" role="group" aria-label="Search mode">
    <button aria-pressed={!content} onclick={() => mode(false)}
      >File names</button
    ><button aria-pressed={content} onclick={() => mode(true)}
      >File contents</button
    >
  </div>
  <form
    onsubmit={(event) => {
      event.preventDefault();
      void search();
    }}
  >
    <label
      >{content ? "Find text in files" : "Find a file"}<input
        bind:this={input}
        aria-label="Workspace query"
        bind:value={query}
        oninput={(event) => {
          query = event.currentTarget.value;
          changed();
        }}
        onkeydown={key}
        maxlength="256"
        placeholder={content
          ? "Exact text, matching case…"
          : "Type a file name…"}
      /></label
    >
    <button type="submit" disabled={!query.trim()}>Search</button>
  </form>
  {#if error}<p role="alert">{error}</p>{/if}
  <p role="status">
    {opening
      ? "Opening file…"
      : busy
        ? "Searching…"
        : progress
          ? `${results.length} matches${progress.truncated ? " · partial results" : ""}`
          : "Choose an authorized recent file, or search your open folders."}
    {progress?.error ?? ""}
  </p>
  {#if results[selected]}<span class="sr-only" aria-live="polite"
      >Selected {results[selected].item.name}</span
    >{/if}
  <ul
    bind:this={resultList}
    class="workspace-search-results"
    aria-label="Search results"
  >
    {#each results as hit, index (hit.item.id)}<li
        class:selected={index === selected}
      >
        <button
          class="search-result-open"
          disabled={opening}
          aria-label={`Open ${hit.item.name}`}
          onclick={() => open(hit.item)}
          onfocus={() => {
            selected = index;
          }}
          ><strong>{hit.item.name}</strong><small
            >{hit.item.parent.label ?? hit.item.parent.name}</small
          >{#if hit.context}<span>{hit.context}</span>{/if}</button
        >
        <button
          aria-label={`Preview result ${hit.item.name}`}
          onclick={() => onpreview(hit.item)}>Preview</button
        >
      </li>{/each}
  </ul>
  {#if !query && recent.length}<section aria-label="Recent search files">
      <h3>Recent files</h3>
      {#each recent as hint (hint.id)}<button
          class="note-link-result"
          onclick={() => openRecent(hint)}
          disabled={busy}>{hint.name}</button
        >{/each}
    </section>{/if}
  {#if progress && !busy && !results.length}<p>
      No matches in the scanned folders.
    </p>{/if}
  <details class="library-policy">
    <summary>Search scope</summary>
    <p>
      Saved files in authorized folders only; unsaved edits are not searched.
      Your Hidden and Ignored choices apply. Filename search is fuzzy. Content
      search is literal and case-sensitive, covering UTF-8 text up to 8 MiB per
      file, 256 MiB per search and 200 matches. Use the arrow keys to select a
      result and Enter to open it.
    </p>
  </details>
</dialog>
