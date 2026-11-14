<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import * as api from "./bridge";
  import { errorMessage } from "../bridge";
  import { windowStart } from "./spans";
  import Code from "./Code.svelte";
  import type { Descriptor } from "../bridge";
  let {
    editing = false,
    onopen,
  }: {
    editing?: boolean;
    onopen: (descriptor: Descriptor, line: number) => Promise<void>;
  } = $props();
  let dialog: HTMLDialogElement, confirm: HTMLDialogElement;
  let viewport = $state<HTMLElement>();
  let repoState = $state<api.State | null>(null),
    diff = $state<api.Diff | null>(null),
    selected = $state<api.Change | null>(null),
    rows = $state<api.Row[]>([]),
    backups = $state<api.Backup[]>([]);
  let busy = $state(false),
    notice = $state(""),
    failure = $state(""),
    filter = $state(""),
    tab = $state("changes"),
    before = $state("index"),
    after = $state("worktree"),
    split = $state(false),
    expanded = $state(false),
    start = $state(0),
    scroll = $state(0),
    pageOffset = $state(0),
    hunkAt = $state(0);
  let historyScope = $state<string | null>(null);
  let chosen = new SvelteSet<string>(),
    generation = 0,
    pageGeneration = 0;
  const visible = $derived(
    repoState?.files.filter((f) =>
      f.path.toLocaleLowerCase().includes(filter.toLocaleLowerCase()),
    ) ?? [],
  );
  const total = $derived(
    diff
      ? split
        ? expanded
          ? diff.splitFullRows
          : diff.splitRows
        : expanded
          ? diff.unifiedFullRows
          : diff.unifiedRows
      : 0,
  );
  const action = $derived(
    before === "head" && after === "index" ? "unstage" : "stage",
  );
  export async function show() {
    dialog.showModal();
    await tick();
    dialog.querySelector<HTMLElement>("button")?.focus();
  }
  function close() {
    ++generation;
    ++pageGeneration;
    dialog.close();
  }
  async function choose() {
    busy = true;
    failure = "";
    try {
      const result = await api.choose();
      if (result) {
        repoState = result;
        selected = null;
        diff = null;
        rows = [];
        backups = await api.backups(result.id);
        pageOffset = 0;
        tab = "changes";
        filter = "";
        historyScope = null;
        before = "index";
        after = "worktree";
        notice =
          "Repository authorized for this session. Comparisons use saved bytes.";
      }
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function history(commit: string) {
    if (!repoState) return;
    busy = true;
    failure = "";
    try {
      historyScope = commit;
      before = commit;
      after = "worktree";
      selected = null;
      diff = null;
      rows = [];
      const page = await api.historyFiles(repoState.id, commit, 0);
      repoState = { ...repoState, ...page };
      pageOffset = 0;
      tab = "changes";
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function refresh(offset = pageOffset) {
    if (!repoState) return;
    const repo = repoState.id,
      path = selected?.path;
    busy = true;
    failure = "";
    ++generation;
    diff = null;
    rows = [];
    chosen.clear();
    try {
      if (historyScope) {
        const page = await api.historyFiles(repo, historyScope, offset);
        repoState = { ...repoState!, ...page };
      } else repoState = await api.refresh(repo, offset);
      pageOffset = offset;
      backups = await api.backups(repo);
      selected = repoState.files.find((f) => f.path === path) ?? null;
      if (selected) await load(selected);
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function load(file = selected) {
    if (!repoState || !file) return;
    const request = ++generation,
      repo = repoState.id;
    busy = true;
    failure = "";
    selected = file;
    diff = null;
    rows = [];
    chosen.clear();
    try {
      const result = await api.compare(repo, file.id, before, after);
      if (request !== generation || repoState?.id !== repo) return;
      diff = result;
      hunkAt = 0;
      scroll = 0;
      start = 0;
      await tick();
      if (viewport) viewport.scrollTop = 0;
      await loadPage();
    } catch (e) {
      if (request === generation) failure = errorMessage(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  async function loadPage() {
    if (!repoState || !diff) return;
    const request = ++pageGeneration,
      repo = repoState.id,
      token = diff.id,
      offset = windowStart(scroll);
    try {
      const result = await api.page(repo, token, offset, split, expanded);
      if (request === pageGeneration && diff?.id === token) {
        rows = result;
        start = offset;
      }
    } catch (e) {
      if (request === pageGeneration) failure = errorMessage(e);
    }
  }
  function scrolled() {
    if (!viewport) return;
    const next = windowStart(viewport.scrollTop);
    scroll = viewport.scrollTop;
    if (next !== start) void loadPage();
  }
  async function layout() {
    scroll = 0;
    start = 0;
    await tick();
    if (viewport) viewport.scrollTop = 0;
    await loadPage();
  }
  function navigate(delta: number) {
    if (!diff?.hunks.length) return;
    hunkAt = Math.max(0, Math.min(diff.hunks.length - 1, hunkAt + delta));
    const h = diff.hunks[hunkAt];
    const row = split
      ? expanded
        ? h.splitFullRow
        : h.splitRow
      : expanded
        ? h.unifiedFullRow
        : h.unifiedRow;
    if (!viewport) return;
    viewport.scrollTop = row * 24;
    scroll = viewport.scrollTop;
    void loadPage();
  }
  async function mutate(kind: string) {
    if (!repoState || !diff) return;
    const repo = repoState.id,
      token = diff.id;
    busy = true;
    failure = "";
    try {
      const hunks =
        kind === "discard" ? diff.hunks.map((h) => h.id) : [...chosen];
      const result = await api.mutate(repo, token, kind, hunks);
      notice = result.message;
      await refresh();
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function open(line: number) {
    if (!repoState || !selected) return;
    busy = true;
    try {
      const descriptor = await api.open(repoState.id, selected.id);
      close();
      await onopen(descriptor, line);
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function recovery(backup: api.Backup, forget = false) {
    if (!repoState) return;
    busy = true;
    failure = "";
    try {
      if (forget) {
        await api.forgetBackup(repoState.id, backup.id);
        notice = "Git recovery copy deleted.";
      } else if (await api.exportBackup(repoState.id, backup.id))
        notice = "Recovered bytes exported to a new independent file.";
      backups = await api.backups(repoState.id);
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="git-workbench"
  aria-label="Git workbench"
  oncancel={(e) => {
    e.preventDefault();
    close();
  }}
>
  <header class="git-heading">
    <div>
      <h1>Changes, in context.</h1>
      <p>
        {repoState
          ? `${repoState.name} · ${repoState.branch}`
          : "Choose a local repository to inspect saved changes."}
      </p>
    </div>
    <button onclick={close} aria-label="Close Git workbench">Close</button>
  </header>
  <div class="git-actions">
    <button class="primary" onclick={choose} disabled={busy}
      >Choose repository</button
    >{#if repoState}<button
        onclick={() => {
          historyScope = null;
          void refresh(0);
        }}
        disabled={busy}>Refresh</button
      ><button
        onclick={async () => {
          if (repoState) {
            await api.revoke(repoState.id);
            repoState = null;
            diff = null;
            rows = [];
            selected = null;
            backups = [];
            notice = "Repository access revoked for this session.";
            failure = "";
          }
        }}
        disabled={busy}>Revoke repository</button
      ><span>{repoState.head?.slice(0, 8) ?? "Unborn HEAD"}</span>{/if}
    <p>Saved disk and index bytes. Unsaved editor text is excluded.</p>
  </div>
  {#if editing}<p class="git-warning">
      Close Source sessions before stage, unstage or discard. Your open edits
      stay separate.
    </p>{/if}
  {#if failure}<p role="alert" class="git-error">
      {failure}
    </p>{/if}{#if notice}<p role="status" class="git-notice">
      {notice}
    </p>{/if}{#if repoState?.warning}<p class="git-warning">
      {repoState.warning}
    </p>{/if}
  <div class="git-body">
    <aside class="git-rail">
      <nav aria-label="Repository sections">
        <button
          aria-pressed={tab === "changes"}
          onclick={() => (tab = "changes")}
          >Changes {repoState?.total ?? 0}</button
        ><button
          aria-pressed={tab === "history"}
          onclick={() => (tab = "history")}>History</button
        ><button
          aria-pressed={tab === "recovery"}
          onclick={() => (tab = "recovery")}>Recovery {backups.length}</button
        >
      </nav>
      {#if tab === "changes"}<label
          >Filter this page<input
            aria-label="Filter Git files"
            bind:value={filter}
          /></label
        >
        <p class="git-small">Index / Disk</p>
        <div class="git-files">
          {#each visible as file (file.id)}<button
              class:selected={selected?.id === file.id}
              onclick={() => load(file)}
              disabled={busy}
              ><span
                class="git-file-badges"
                aria-label={`Index ${file.staged}, disk ${file.working}`}
                >{file.staged === "clean" ? "·" : file.staged[0].toUpperCase()}
                {file.working === "clean"
                  ? "·"
                  : file.working === "untracked"
                    ? "?"
                    : file.working[0].toUpperCase()}</span
              ><span>{file.path}</span>{#if file.warning}<small
                  >{file.warning}</small
                >{/if}</button
            >{/each}
        </div>
        {#if repoState}<div class="git-pagination">
            <button
              disabled={busy || pageOffset === 0}
              onclick={() => refresh(Math.max(0, pageOffset - 100))}
              >Previous files</button
            ><span
              >{pageOffset + 1}–{Math.min(pageOffset + 100, repoState.total)} / {repoState.total}</span
            ><button
              disabled={busy || pageOffset + 100 >= repoState.total}
              onclick={() => refresh(pageOffset + 100)}>Next files</button
            >
          </div>{/if}
      {:else if tab === "history"}<p class="git-small">
          Up to 50 first-parent commits. Select a baseline, then choose a file
          in Changes.
        </p>
        {#each repoState?.history ?? [] as commit (commit.id)}<button
            class="git-commit"
            onclick={() => {
              void history(commit.id);
            }}><code>{commit.short}</code><span>{commit.summary}</span></button
          >{/each}
      {:else}<p class="git-small">
          Exact bytes captured before discard. Export chooses a new file and
          never reauthorizes an old path.
        </p>
        {#each backups as backup (backup.id)}<div class="git-backup">
            <strong>{backup.path}</strong><small
              >{new Date(backup.created * 1000).toLocaleString()}</small
            ><button onclick={() => recovery(backup)} disabled={busy}
              >Export recovery copy</button
            >
            <details>
              <summary>Delete recovery copy</summary>
              <p>
                This removes the retained bytes permanently. Export first if
                needed.
              </p>
              <button onclick={() => recovery(backup, true)} disabled={busy}
                >Delete this recovery copy</button
              >
            </details>
          </div>{/each}{/if}
    </aside>
    <section class="git-comparison" aria-label="Source Git comparison">
      <div class="git-baselines">
        <label
          >Before<select
            aria-label="Git before"
            value={before}
            onchange={(e) => {
              before = e.currentTarget.value;
              void load();
            }}
            disabled={busy}
            ><option value="index">Index</option><option value="head"
              >HEAD</option
            ><option value="worktree">Disk</option
            >{#each repoState?.history ?? [] as c (c.id)}<option value={c.id}
                >{c.short} {c.summary}</option
              >{/each}</select
          ></label
        ><label
          >After<select
            aria-label="Git after"
            value={after}
            onchange={(e) => {
              after = e.currentTarget.value;
              void load();
            }}
            disabled={busy}
            ><option value="worktree">Disk</option><option value="index"
              >Index</option
            ><option value="head">HEAD</option
            >{#each repoState?.history ?? [] as c (c.id)}<option value={c.id}
                >{c.short} {c.summary}</option
              >{/each}</select
          ></label
        ><button
          onclick={() => {
            split = !split;
            void layout();
          }}
          aria-pressed={split}>Side by side</button
        ><label
          ><input
            type="checkbox"
            bind:checked={expanded}
            onchange={layout}
          />Expand unchanged lines</label
        >
      </div>
      {#if diff}<div class="git-diff-title">
          <strong>{diff.path}</strong><small>{diff.conversion}</small>
        </div>
        <div class="git-hunks">
          <button
            onclick={() => navigate(-1)}
            disabled={!diff.hunks.length || hunkAt === 0}>Previous hunk</button
          ><span
            >{diff.hunks.length ? hunkAt + 1 : 0} / {diff.hunks.length}</span
          ><button
            onclick={() => navigate(1)}
            disabled={!diff.hunks.length || hunkAt + 1 >= diff.hunks.length}
            >Next hunk</button
          ><button
            onclick={() => {
              for (const h of diff!.hunks) chosen.add(h.id);
            }}>Select all hunks</button
          ><button onclick={() => chosen.clear()}>Clear selection</button>
        </div>
        <div class="git-hunk-list" aria-label="Captured hunks">
          {#each diff.hunks as h, i (h.id)}<label
              ><input
                type="checkbox"
                aria-label={`Select hunk ${i + 1}`}
                checked={chosen.has(h.id)}
                onchange={(e) =>
                  e.currentTarget.checked
                    ? chosen.add(h.id)
                    : chosen.delete(h.id)}
              />−{h.oldLines ? h.oldStart + 1 : 0},{h.oldLines} +{h.newStart +
                1},{h.newLines}</label
            >{/each}
        </div>
        {#if diff.mutation}<p class="git-warning">{diff.mutation}</p>{/if}
        <!-- Keyboard focus lets users scroll this named source region. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="git-viewport"
          bind:this={viewport}
          onscroll={scrolled}
          tabindex="0"
          role="region"
          aria-label="Git diff lines"
          onkeydown={(e) => {
            if (e.key === "j" && e.altKey) {
              e.preventDefault();
              navigate(1);
            }
            if (e.key === "k" && e.altKey) {
              e.preventDefault();
              navigate(-1);
            }
          }}
        >
          <div
            class="git-canvas"
            style:height={`${Math.max(1, total) * 24}px`}
            class:split
          >
            {#each rows as row, i (start + i)}<div
                class="git-row {row.kind}"
                style:top={`${(start + i) * 24}px`}
              >
                {#if row.kind === "gap"}<button
                    class="git-gap"
                    onclick={() => {
                      expanded = true;
                      void layout();
                    }}
                    >{row.hidden.toLocaleString()} unchanged lines folded · Expand</button
                  >
                {:else if split}<div class="git-cell old">
                    <button
                      class="git-line-number"
                      title="Open current file in Source (Disk-side hunk position)"
                      onclick={() => open(row.sourceLine ?? 1)}
                      >{row.oldNumber ?? ""}</button
                    >
                    <pre>{#if row.oldText !== null}<Code
                          text={row.oldText}
                          other={row.newText}
                          path={diff.path}
                          side="old"
                          syntaxEnabled={row.kind === "context"}
                        />{/if}</pre>
                  </div>
                  <div class="git-cell new">
                    <button
                      class="git-line-number"
                      title="Open current file in Source (Disk-side hunk position)"
                      onclick={() => open(row.sourceLine ?? 1)}
                      >{row.newNumber ?? ""}</button
                    >
                    <pre>{#if row.newText !== null}<Code
                          text={row.newText}
                          other={row.oldText}
                          path={diff.path}
                          side="new"
                          syntaxEnabled={row.kind === "context"}
                        />{/if}</pre>
                  </div>
                {:else}<button
                    class="git-line-number"
                    title="Open current file in Source (Disk-side hunk position)"
                    onclick={() => open(row.sourceLine ?? 1)}
                    >{row.oldNumber ?? ""}</button
                  ><span class="git-number">{row.newNumber ?? ""}</span><span
                    class="git-sign"
                    >{row.kind === "delete"
                      ? "−"
                      : row.kind === "add"
                        ? "+"
                        : " "}</span
                  >
                  <pre><Code
                      text={row.newText ?? row.oldText ?? ""}
                      path={diff.path}
                      side={row.kind === "delete" ? "old" : "new"}
                      syntaxEnabled={row.kind === "context"}
                    /></pre>{/if}{#if row.clipped}<span
                    title="Long display line clipped to 1 KiB; captured source remains exact"
                    >…</span
                  >{/if}
                {#if [row.oldEnding, row.newEnding].some((e) => e === "CRLF" || e === "no newline")}<small
                    class="git-eol"
                    >{row.oldEnding === row.newEnding
                      ? row.oldEnding
                      : [row.oldEnding, row.newEnding]
                          .filter(Boolean)
                          .join(" / ")}</small
                  >{/if}
              </div>{/each}
          </div>
        </div>
        <footer class="git-mutations">
          <button
            class="primary"
            onclick={() => mutate(action)}
            disabled={busy || editing || !!diff.mutation || !chosen.size}
            >{action === "unstage"
              ? "Unstage selected hunks"
              : "Stage selected hunks"}</button
          ><button
            onclick={() => confirm.showModal()}
            disabled={busy ||
              editing ||
              !!diff.mutation ||
              before !== "index" ||
              after !== "worktree" ||
              !diff.hunks.length}>Discard file changes…</button
          ><span
            >{chosen.size} selected · {total.toLocaleString()} display rows</span
          >
        </footer>
      {:else}<div class="git-empty">
          <h2>
            {busy
              ? "Capturing a comparison…"
              : repoState
                ? "Select a changed file."
                : "Your repository stays local."}
          </h2>
          <p>
            {repoState
              ? "Compare Disk, Index, HEAD or a selected commit. These are source diffs; rendered review belongs to a separate workflow."
              : "Choose the working-tree folder. Git operations use native grants; no hooks, configured tools or network run."}
          </p>
        </div>{/if}
    </section>
  </div>
</dialog>
<dialog
  bind:this={confirm}
  class="git-confirm"
  aria-label="Confirm Git discard"
>
  <h2>Discard saved file changes?</h2>
  <p>{selected?.path}</p>
  <p>
    This replaces the whole saved file with Index bytes. The exact previous
    bytes are retained in Git Recovery. Open Source sessions must be closed
    first.
  </p>
  <button onclick={() => confirm.close()}>Keep changes</button><button
    onclick={() => {
      confirm.close();
      void mutate("discard");
    }}>Discard and keep recovery copy</button
  >
</dialog>
