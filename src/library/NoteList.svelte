<script lang="ts">
  import { errorMessage } from "../bridge";
  import * as api from "../library";
  import Icon from "../ui/Icon.svelte";

  let {
    folder,
    active,
    activeLocation,
    revision,
    gitRevision,
    hidden,
    ignored,
    busy,
    canCreate,
    onopen,
    oncreate,
  }: {
    folder: api.Folder | null;
    active: boolean;
    activeLocation: string | null;
    revision: number;
    gitRevision: number;
    hidden: boolean;
    ignored: boolean;
    busy: boolean;
    canCreate: boolean;
    onopen: (item: api.Item) => Promise<void>;
    oncreate: () => void;
  } = $props();

  const pageSize = 100;
  const dateFormat = new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
  let page = $state<api.Page | null>(null);
  let offset = $state(0);
  let query = $state("");
  let error = $state("");
  let loading = $state(false);
  let gitBadges = $state<Record<string, string>>({});
  let lastRevision: number | undefined;
  let lastPage = "";
  let input = $state<HTMLInputElement>();
  let list: HTMLUListElement;

  $effect(() => {
    if (!folder || !active) return;
    const id = folder.id;
    const from = offset;
    const search = query.trim();
    const showHidden = hidden;
    const showIgnored = ignored;
    let refresh = lastRevision !== undefined && revision !== lastRevision;
    lastRevision = revision;
    let canceled = false;
    let timer: ReturnType<typeof setTimeout>;
    loading = true;
    error = "";
    const key = JSON.stringify([from, search, showHidden, showIgnored]);
    if (lastPage !== key) {
      page = null;
      lastPage = key;
    }
    const load = async () => {
      try {
        const result = await api.page(
          id,
          from,
          "name",
          showHidden,
          "notes",
          refresh,
          showIgnored,
          search,
        );
        if (canceled) return;
        refresh = false;
        if (result.complete && from && from >= result.total) {
          offset =
            Math.floor(Math.max(0, result.total - 1) / pageSize) * pageSize;
          return;
        }
        page = result;
        loading = !result.complete;
        if (!result.complete) timer = setTimeout(() => void load(), 80);
      } catch (e) {
        if (canceled) return;
        error = errorMessage(e);
        loading = false;
      }
    };
    if (search) timer = setTimeout(() => void load(), 160);
    else void load();
    return () => {
      canceled = true;
      clearTimeout(timer);
    };
  });

  $effect(() => {
    void gitRevision;
    if (!active || !page?.items.length) {
      gitBadges = {};
      return;
    }
    const ids = page.items.map((item) => item.id);
    let canceled = false;
    void api.gitBadges(ids).then(
      (badges) => {
        if (!canceled) gitBadges = badges;
      },
      () => {
        if (!canceled) gitBadges = {};
      },
    );
    return () => {
      canceled = true;
    };
  });

  function formatDate(timestamp: number) {
    const date = new Date(timestamp * 1000);
    return Number.isFinite(date.getTime()) ? dateFormat.format(date) : "";
  }

  function focusRow(index: number) {
    list?.querySelectorAll("button")[index]?.focus();
  }

  function fromSearch(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229 || !page?.items.length)
      return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      focusRow(event.key === "ArrowDown" ? 0 : page.items.length - 1);
    }
    if (event.key === "Enter" && !busy) {
      event.preventDefault();
      void onopen(page.items[0]);
    }
  }

  function moveFocus(event: KeyboardEvent, index: number) {
    if (event.isComposing || event.keyCode === 229 || !page) return;
    switch (event.key) {
      case "Enter":
        event.preventDefault();
        if (!busy) void onopen(page.items[index]);
        break;
      case "ArrowDown":
        event.preventDefault();
        focusRow(index + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        if (index === 0) input?.focus();
        else focusRow(index - 1);
        break;
      case "Home":
        event.preventDefault();
        focusRow(0);
        break;
      case "End":
        event.preventDefault();
        focusRow(page.items.length - 1);
        break;
    }
  }
</script>

<section
  class="library-note-browser"
  aria-label="Folder notes"
  hidden={!active}
>
  <header class="note-browser-header">
    <div>
      <h2 dir="auto" title={folder?.label ?? folder?.name}>
        {folder?.name ?? "Notes"}
      </h2>
      {#if page || loading}
        <p>
          {loading
            ? "Loading notes…"
            : `${page?.total ?? 0} ${page?.total === 1 ? "note" : "notes"}`}
        </p>
      {/if}
    </div>
    <button
      class="icon-button"
      aria-label="New note from library"
      title="New note"
      onclick={oncreate}
      disabled={!canCreate}
    >
      <Icon name="new-note" />
    </button>
  </header>
  {#if folder}
    <label class="note-browser-search">
      <Icon name="find" />
      <span class="sr-only">Filter notes in this folder</span>
      <input
        type="search"
        placeholder="Filter by name"
        maxlength="128"
        bind:this={input}
        value={query}
        oninput={(event) => {
          offset = 0;
          query = event.currentTarget.value;
        }}
        onkeydown={fromSearch}
      />
    </label>
  {/if}
  <div class="note-browser-content" aria-busy={loading}>
    {#if error}<p class="note-browser-message" role="alert">{error}</p>{/if}
    {#if !folder}
      <p class="note-browser-message">Add a folder to browse its notes.</p>
    {:else if page?.complete && page.total === 0}
      <p class="note-browser-message">
        {query.trim()
          ? "No matching notes."
          : "No Markdown or text notes in this folder."}
      </p>
    {/if}
    <ul
      class="note-list"
      aria-label="Notes in selected folder"
      bind:this={list}
    >
      {#each page?.items ?? [] as item, index (item.id)}
        <li>
          <button
            class="note-list-row"
            aria-label={`Open ${item.name}`}
            aria-current={item.location && item.location === activeLocation
              ? "page"
              : undefined}
            title={item.name}
            disabled={busy}
            onclick={() => onopen(item)}
            onkeydown={(event) => moveFocus(event, index)}
          >
            <Icon name="file" />
            <span>
              <bdi>{item.name}</bdi>
              <small>{formatDate(item.modified)}</small>
            </span>
            {#if gitBadges[item.id]}
              <span
                class="library-git-badge"
                title={`Git at last refresh: ${gitBadges[item.id]}`}>Git</span
              >
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  </div>
  {#if offset || (page?.total ?? 0) > pageSize}
    <nav class="pagination note-browser-pagination" aria-label="Notes pages">
      <button
        aria-label="Previous notes"
        disabled={loading || offset === 0}
        onclick={() => (offset -= pageSize)}>‹</button
      >
      <span
        >{page
          ? `${offset + 1}–${Math.min(offset + pageSize, page.total)} of ${page.total}`
          : "Loading…"}</span
      >
      <button
        aria-label="Next notes"
        disabled={loading || !page || offset + pageSize >= page.total}
        onclick={() => (offset += pageSize)}>›</button
      >
    </nav>
  {/if}
</section>
