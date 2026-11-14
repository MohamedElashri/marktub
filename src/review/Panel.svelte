<script lang="ts">
  import { onMount, untrack, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import * as api from "./bridge";
  import { errorMessage, type Descriptor } from "../bridge";
  import { blocks, changedBlocks, type Block } from "./blocks";
  import Rendered from "./Rendered.svelte";
  import SourceDiff from "./SourceDiff.svelte";
  let {
    document: doc,
    active = true,
    savedDisplayed = false,
    editing = false,
    stale = false,
    onoff,
    onjump,
    ongit,
    onconflict,
    onarchive,
  }: {
    document: Descriptor;
    active?: boolean;
    savedDisplayed?: boolean;
    editing?: boolean;
    stale?: boolean;
    onoff: () => void;
    onjump: (byte: number) => void | Promise<void>;
    ongit: () => void | Promise<void>;
    onconflict: () => void | Promise<void>;
    onarchive: () => void | Promise<void>;
  } = $props();
  let view = $state<api.View | null>(null),
    failure = $state(""),
    notice = $state(""),
    busy = $state(false),
    baseline = $state("reviewed"),
    tab = $state("changes"),
    source = $state(false),
    selected = $state<Block | null>(null),
    at = $state(0),
    comment = $state(""),
    target = $state<string | null>(null),
    reply = $state<string | null>(null),
    comparison = $state<api.Comparison | null>(null),
    report = $state(""),
    includePath = $state(false),
    snippets = $state(false),
    format = $state("json"),
    blockQuery = $state("");
  let field = $state<HTMLTextAreaElement>();
  let draftDialog: HTMLDialogElement,
    draftAnswer: ((discard: boolean) => void) | null = null;
  let replyDrafts = $state<Record<string, string>>({}),
    recordId = $state<string | null>(null);
  function hasDraft() {
    return (
      !!comment.trim() || Object.values(replyDrafts).some((v) => v?.trim())
    );
  }
  function answerDraft(discard: boolean) {
    draftDialog.close();
    const answer = draftAnswer;
    draftAnswer = null;
    answer?.(discard);
  }

  let queue = Promise.resolve(),
    ended = false;
  const baselineText = $derived(
    baseline === "reviewed" ? view?.reviewedText : view?.viewedText,
  );
  const parsed = $derived.by(() => {
    try {
      return view
        ? {
            items: blocks(view.text),
            changes:
              baselineText === null || baselineText === undefined
                ? []
                : changedBlocks(baselineText, view.text),
            error: "",
          }
        : { items: [], changes: [], error: "" };
    } catch (e) {
      return { items: [], changes: [], error: errorMessage(e) };
    }
  });
  const changes = $derived(parsed.changes),
    page = $derived(changes.slice(at, at + 30));
  const availableBlocks = $derived(
    parsed.items
      .map((block, index) => ({ block, index }))
      .filter(({ block }) =>
        `${block.line} ${block.heading} ${block.quote}`
          .toLocaleLowerCase()
          .includes(blockQuery.toLocaleLowerCase()),
      )
      .slice(0, 200),
  );
  const currentStatus = $derived(
    !view?.decisionRevision
      ? "pending"
      : view.decisionRevision === view.currentHash
        ? view.decision
        : "outdated decision: " + view.decision,
  );
  function enqueue(action: () => Promise<void>) {
    const next = queue.then(action);
    queue = next.catch((e) => {
      failure = errorMessage(e);
    });
    return next;
  }
  async function establish(id: string, isActive: boolean) {
    if (ended) return;
    if (!isActive) {
      if (view) {
        await api.end(view.token);
        view = null;
      }
      return;
    }
    busy = true;
    failure = "";
    try {
      const next = view ? await api.sync(view.token, id) : await api.begin(id);
      if (ended) {
        await api.end(next.token);
        view = null;
        return;
      }
      view = next;
      recordId = next.record;
      selected = null;
      comparison = null;
      report = "";
      await acknowledgeDisplayed(id);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    const id = doc.id,
      isActive = active;
    untrack(() => {
      void enqueue(() => establish(id, isActive)).catch(() => {});
    });
  });
  $effect(() => {
    const shown = savedDisplayed,
      id = doc.id;
    untrack(() => {
      if (shown) void enqueue(() => acknowledgeDisplayed(id)).catch(() => {});
    });
  });
  $effect(() => {
    const changed = stale,
      id = doc.id;
    untrack(() => {
      if (changed) void enqueue(() => captureDisk(id)).catch(() => {});
    });
  });
  export function isCapture(hash: string) {
    return view?.currentHash === hash;
  }
  async function acknowledgeDisplayed(id: string) {
    if (view && savedDisplayed && active) await api.displayed(view.token, id);
  }
  export function markDisplayed(id = doc.id) {
    return enqueue(() => acknowledgeDisplayed(id));
  }
  export async function stop() {
    if (hasDraft()) {
      const discard = await new Promise<boolean>((resolve) => {
        draftAnswer = resolve;
        draftDialog.showModal();
      });
      if (!discard)
        throw Error(
          "Review draft kept. Save it or explicitly discard it before leaving this view.",
        );
      comment = "";
      target = null;
      reply = null;
      replyDrafts = {};
    }
    ended = true;
    try {
      await queue;
      if (view) {
        await api.end(view.token);
        view = null;
      }
    } catch (e) {
      ended = false;
      throw e;
    }
  }
  async function disable() {
    try {
      await stop();
      onoff();
    } catch (e) {
      ended = false;
      failure = errorMessage(e);
    }
  }
  async function captureDisk(id = doc.id) {
    if (!view || ended || !active) return;
    busy = true;
    try {
      view = await api.sync(view.token, id, true);
      selected = null;
      comparison = null;
      report = "";
      notice =
        "Saved disk capture refreshed. Review baselines did not advance.";
    } finally {
      busy = false;
    }
  }
  async function perform(action: Record<string, unknown>) {
    if (!view) return false;
    const expected = view,
      id = doc.id;
    return enqueue(async () => {
      busy = true;
      failure = "";
      try {
        view = await api.update(
          expected.token,
          id,
          expected.version,
          expected.currentHash,
          action,
        );
        report = "";
        if (action.kind === "delete" && typeof action.id === "string") {
          delete replyDrafts[action.id];
          if (target === action.id) {
            target = null;
            reply = null;
            comment = "";
          }
        }
        if (
          action.kind === "delete_reply" &&
          target === action.id &&
          reply === action.reply
        ) {
          target = null;
          reply = null;
          comment = "";
        }
      } finally {
        busy = false;
      }
    })
      .then(() => true)
      .catch(() => false);
  }
  function choose(block: Block) {
    selected = block;
    tab = "comments";
  }
  async function submit() {
    if (!view) return;
    let action: Record<string, unknown>;
    if (reply && target)
      action = { kind: "edit_reply", id: target, reply, text: comment };
    else if (target) action = { kind: "edit", id: target, text: comment };
    else if (selected)
      action = { kind: "comment", ...selection(selected), text: comment };
    else return;
    if (await perform(action)) {
      comment = "";
      target = null;
      reply = null;
    }
  }
  function selection(b: Block) {
    return {
      start: b.start,
      end: b.end,
      quote: b.quote,
      heading_start: b.headingStart,
      heading_end: b.headingEnd,
    };
  }
  async function showSource() {
    if (!view) return;
    busy = true;
    try {
      comparison = await api.compare(view.token, doc.id, baseline);
      source = true;
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function prepare() {
    if (!view) return;
    busy = true;
    try {
      const options = {
        token: view.token,
        id: doc.id,
        path: includePath,
        snippets,
        format,
      };
      const json = await api.report(
        options.token,
        options.id,
        options.path,
        options.snippets,
      );
      if (
        options.token !== view?.token ||
        options.id !== doc.id ||
        options.path !== includePath ||
        options.snippets !== snippets ||
        options.format !== format
      )
        return;
      report = format === "markdown" ? api.markdownReport(json) : json;
      await tick();
      notice = "Feedback prepared. Review it before copying or saving.";
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function saveReport() {
    if (!view) return;
    busy = true;
    try {
      if (await api.save(view.token, doc.id, includePath, snippets, format))
        notice = "Feedback exported to a new native-selected destination.";
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    const listener = listen<{ record: string }>("review-deleted", (event) => {
      if (recordId === event.payload.record) {
        ended = true;
        view = null;
        replyDrafts = {};
        comment = "";
        onoff();
      }
    });
    return () => {
      ended = true;
      void listener.then((stop) => stop());
      void queue
        .then(async () => {
          if (view) await api.end(view.token);
        })
        .catch(() => {});
    };
  });
</script>

<aside class="review-margin" aria-label="Document review" aria-busy={busy}>
  <header>
    <h2>Read, then respond.</h2>
    <button onclick={disable} disabled={busy}>Turn Review off</button>
  </header>
  <p>
    Saved captures only. No author is inferred. Viewing does not mark a revision
    reviewed.
  </p>
  {#if failure}<p role="alert">
      {failure}<button
        onclick={() =>
          void enqueue(() => establish(doc.id, true)).catch(() => {})}
        >Refresh review</button
      >
    </p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if view}
    <div class="review-baseline">
      <span>Current saved {view.currentHash.slice(0, 12)}</span><span
        >Last reviewed {view.reviewedHash?.slice(0, 12) ?? "none"}</span
      ><span>Previous displayed {view.viewedHash?.slice(0, 12) ?? "none"}</span>
    </div>
    {#if view.initial}<p class="review-first">
        This view began without an earlier reviewed baseline. Its first saved
        capture was used.
      </p>{/if}
    {#if !view.viewedHash}<p>
        No prior displayed saved revision. “Viewed” means displayed, not every
        paragraph read.
      </p>{/if}
    {#if editing}<p class="review-warning">
        Source working text is separate from this saved review. Only a native
        saved Read view advances displayed history; use Capture disk after
        saving.<button onclick={onconflict}>Compare editor and disk</button>
      </p>{/if}
    {#if view.renamedTo}<p class="review-warning">
        The original file was renamed to “{view.renamedTo}” in the same folder.
        Open it with native selection; saved snapshots and comments stay here.
        No path has been reauthorized.
      </p>{/if}
    <nav>
      <button
        onclick={() => void enqueue(() => captureDisk()).catch(() => {})}
        disabled={busy}>Capture disk</button
      ><button onclick={() => perform({ kind: "reviewed" })} disabled={busy}
        >Mark saved capture reviewed</button
      ><button onclick={ongit}>Git comparisons</button>
    </nav>
    <p>Decision: <strong>{currentStatus?.replaceAll("_", " ")}</strong></p>
    <nav>
      <button
        onclick={() => perform({ kind: "decision", value: "approved" })}
        disabled={busy}>Approve capture</button
      ><button
        onclick={() =>
          perform({ kind: "decision", value: "changes_requested" })}
        disabled={busy}>Request changes</button
      >
    </nav>
    <nav aria-label="Review sections">
      <button aria-pressed={tab === "changes"} onclick={() => (tab = "changes")}
        >Changes</button
      ><button
        aria-pressed={tab === "comments"}
        onclick={() => (tab = "comments")}
        >Comments {view.comments.length}</button
      ><button
        aria-pressed={tab === "feedback"}
        onclick={() => (tab = "feedback")}>Feedback</button
      >
    </nav>
    {#if tab === "changes"}
      <label
        >Compare saved capture with<select
          aria-label="Review baseline"
          value={baseline}
          onchange={(e) => {
            baseline = e.currentTarget.value;
            source = false;
            comparison = null;
            at = 0;
          }}
          ><option value="reviewed">Last reviewed</option><option value="viewed"
            >Previous displayed</option
          ></select
        ></label
      >
      <nav>
        <button
          onclick={() => {
            source = false;
          }}
          aria-pressed={!source}>Changed Markdown blocks</button
        ><button
          onclick={showSource}
          disabled={busy || baselineText === null || baselineText === undefined}
          aria-pressed={source}>Saved source diff</button
        >
      </nav>
      {#if parsed.error}<p role="status">{parsed.error}</p>{/if}
      {#if source && comparison}<SourceDiff {comparison} />{:else}
        <p>
          {changes.length} changed blocks · exact source spans; repeated text does
          not imply a stable comment identity.
        </p>
        {#each page as change, i (`${change.kind}-${change.block.start}-${i}`)}<section
            class="review-change {change.kind}"
          >
            <header>
              <strong
                >{change.kind === "removed" ? "Removed" : "Added"} · {change
                  .block.kind}</strong
              ><span>Line {change.block.line} · {change.block.heading}</span>
            </header>
            <Rendered text={change.block.quote} />
            <details>
              <summary>Original saved source</summary>
              <pre>{change.block.quote.slice(0, 16384)}</pre>
            </details>
            {#if change.kind !== "removed"}<button
                onclick={() => choose(change.block)}
                disabled={change.block.end - change.block.start > 65536}
                >Comment on this block</button
              ><button onclick={() => onjump(change.block.start)}
                >Show current section</button
              >{/if}
          </section>{/each}
        <nav>
          <button
            disabled={at === 0}
            onclick={() => (at = Math.max(0, at - 30))}
            >Previous changed blocks</button
          ><span
            >{changes.length ? at + 1 : 0}–{Math.min(changes.length, at + 30)} / {changes.length}</span
          ><button
            disabled={at + 30 >= changes.length}
            onclick={() => (at += 30)}>Next changed blocks</button
          >
        </nav>
      {/if}
    {:else if tab === "comments"}
      <label
        >Find saved block<input
          aria-label="Find comment block"
          bind:value={blockQuery}
        /></label
      >
      <p>First 200 matching blocks; search by line, heading or source text.</p>
      <label
        >Current saved block<select
          aria-label="Comment block"
          onchange={(e) => {
            const b =
              e.currentTarget.value === ""
                ? undefined
                : parsed.items[Number(e.currentTarget.value)];
            if (b) choose(b);
          }}
          ><option value="">Choose a source-backed block</option
          >{#each availableBlocks as { block, index } (block.start)}<option
              value={index}
              >Line {block.line}: {block.quote.slice(0, 65)}</option
            >{/each}</select
        ></label
      >
      {#if selected}<p>
          Selected bytes {selected.start}–{selected.end} · {selected.heading}
        </p>{/if}
      <label
        >{target
          ? reply
            ? "Edit reply"
            : "Edit comment"
          : "New anchored comment"}<textarea
          aria-label="Review comment"
          disabled={busy}
          bind:value={comment}
          maxlength="4096"
        ></textarea></label
      ><button
        onclick={submit}
        disabled={busy || !comment.trim() || (!selected && !target)}
        >Save comment</button
      >
      <button
        disabled={busy}
        onclick={() => {
          target = null;
          reply = null;
        }}>New comment on selected block</button
      >
      {#each view.comments as entry (entry.comment.id)}<article
          class="review-thread"
          data-anchor-status={entry.status}
        >
          <header>
            <strong
              >{entry.comment.resolved ? "Resolved" : "Open"} · {entry.status}</strong
            ><span
              >{entry.lineStart === null
                ? "No verified current source position"
                : `Current line ${entry.lineStart}`}</span
            >
          </header>
          <p>{entry.comment.text}</p>
          <small
            >Original {entry.comment.anchor.revision.slice(0, 12)} · bytes {entry
              .comment.anchor.start}–{entry.comment.anchor.end}</small
          >
          <nav>
            <button
              onclick={() => {
                target = entry.comment.id;
                reply = null;
                comment = entry.comment.text;
              }}>Edit comment</button
            ><button
              onclick={() =>
                perform({
                  kind: "resolve",
                  id: entry.comment.id,
                  value: !entry.comment.resolved,
                })}
              >{entry.comment.resolved
                ? "Reopen comment"
                : "Resolve comment"}</button
            ><button
              onclick={() => perform({ kind: "delete", id: entry.comment.id })}
              >Delete thread</button
            >{#if entry.start !== null}<button
                onclick={() => onjump(entry.start!)}>Show anchor</button
              >{/if}<button
              disabled={!selected || busy}
              onclick={() =>
                selected &&
                perform({
                  kind: "reanchor",
                  id: entry.comment.id,
                  ...selection(selected),
                })}>Reanchor to selected block</button
            >
          </nav>
          {#each entry.comment.replies as response (response.id)}<blockquote>
              <p>{response.text}</p>
              <button
                onclick={() => {
                  target = entry.comment.id;
                  reply = response.id;
                  comment = response.text;
                }}>Edit reply</button
              ><button
                onclick={() =>
                  perform({
                    kind: "delete_reply",
                    id: entry.comment.id,
                    reply: response.id,
                  })}>Delete reply</button
              >
            </blockquote>{/each}
          <form
            onsubmit={async (e) => {
              e.preventDefault();
              const text = replyDrafts[entry.comment.id] ?? "";
              if (await perform({ kind: "reply", id: entry.comment.id, text }))
                replyDrafts[entry.comment.id] = "";
            }}
          >
            <label
              >Reply<input
                name="reply"
                aria-label="Reply to comment"
                bind:value={replyDrafts[entry.comment.id]}
                maxlength="4096"
                required
                disabled={busy}
              /></label
            ><button disabled={busy}>Add reply</button>
          </form>
        </article>{/each}
    {:else}
      <p>
        Feedback contains native saved-byte revision hashes, anchor ranges and
        explicit stale/orphaned status. File-path metadata and source snippets
        are omitted by default. Comments and replies are exported verbatim;
        remove sensitive paths you typed yourself. Only saved feedback is
        exported.
      </p>
      <label
        ><input
          type="checkbox"
          disabled={busy}
          bind:checked={includePath}
          onchange={() => (report = "")}
        />Include actual file path (consent)</label
      ><label
        ><input
          type="checkbox"
          disabled={busy}
          bind:checked={snippets}
          onchange={() => (report = "")}
        />Include source quotes and context</label
      ><label
        >Feedback format<select
          aria-label="Feedback format"
          bind:value={format}
          onchange={() => (report = "")}
          ><option value="json">JSON</option><option value="markdown"
            >Portable Markdown</option
          ></select
        ></label
      >
      <nav>
        <button onclick={prepare} disabled={busy}>Prepare feedback</button
        ><button onclick={saveReport} disabled={busy}>Export feedback…</button>
      </nav>
      {#if report}<textarea
          aria-label="Prepared review feedback"
          readonly
          value={report}
          bind:this={field}
        ></textarea><button
          onclick={() => {
            try {
              if (field) api.copyPrepared(report, field);
              notice = "Feedback copied.";
            } catch (e) {
              failure = errorMessage(e);
            }
          }}>Copy prepared feedback</button
        >{/if}
    {/if}
    <footer>
      <p>
        {view.snapshots} / 8 snapshots · 2 MiB per capture · 200 comments · 20 replies
        per comment. Pinned anchors are never automatically deleted. Large source
        blocks use labeled excerpts.
      </p>
      <button onclick={onarchive}>Manage stored reviews</button>
    </footer>
  {:else}<p>
      {active
        ? "Capturing the saved revision…"
        : "Review paused while this tab is hidden."}
    </p>{/if}
</aside>

<dialog bind:this={draftDialog} aria-label="Unsaved review draft">
  <h2>Keep this review draft?</h2>
  <p>
    Save the comment or reply before leaving, or explicitly discard the unsaved
    feedback draft. Saved review records and source text stay intact.
  </p>
  <button onclick={() => answerDraft(false)}>Keep editing review draft</button
  ><button onclick={() => answerDraft(true)}
    >Discard review draft and leave</button
  >
</dialog>
