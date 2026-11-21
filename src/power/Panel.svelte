<script lang="ts">
  import { onDestroy } from "svelte";
  import * as api from "./bridge";
  import { resources, diagramPng, documentMathPngs } from "./assets";
  import { errorMessage } from "../bridge";
  let {
    oncapture,
    onsettle,
    oncopy,
    hasSaved = true,
    hasWorking = false,
  }: {
    oncapture: (saved: boolean) => Promise<api.Capture>;
    onsettle: (capture: api.Capture) => Promise<void>;
    oncopy: () => Promise<api.Capture | null>;
    hasSaved?: boolean;
    hasWorking?: boolean;
  } = $props();
  let dialog: HTMLDialogElement;
  let data = $state<api.State | null>(null),
    capture = $state<api.Capture | null>(null),
    saved = $state(true),
    busy = $state(false),
    failure = $state(""),
    notice = $state(""),
    tab = $state("pdf"),
    toc = $state(true),
    texPdf = $state(false),
    pdfFonts = $state<"system" | "bundled">("system"),
    selected = $state(""),
    write = $state(false),
    job = $state<string | null>(null),
    result = $state<api.Result | null>(null),
    exportId = $state<string | null>(null),
    warning = $state("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let rowAt = $state(0);
  const asset = $derived.by(() => {
    try {
      return capture
        ? resources(capture.text)
        : { images: [], diagrams: [], error: "" };
    } catch (e) {
      return { images: [], diagrams: [], error: errorMessage(e) };
    }
  });
  const block = $derived(capture?.blocks.find((b) => b.id === selected));
  const running = $derived(result?.status === "running");
  async function action(fn: () => Promise<unknown>) {
    busy = true;
    failure = "";
    try {
      await fn();
    } catch (e) {
      failure = errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function refresh() {
    data = await api.state();
  }
  async function recapture(example?: api.Kind) {
    if (running)
      throw Error("Wait for the current job before replacing its capture.");
    clearTimeout(timer);
    job = null;
    result = null;
    exportId = null;
    warning = "";
    if (capture) await api.release(capture.id);
    capture = null;
    capture = await oncapture(saved);
    selected =
      (example
        ? capture.blocks.find((block) => block.kind === example)
        : capture.blocks[0]
      )?.id ?? "";
    notice = "Captured this revision. Later edits do not alter its PDF input.";
  }
  export async function show(example?: api.Kind) {
    if (dialog.open) return;
    if (example) tab = "run";
    dialog.showModal();
    saved = hasSaved;
    await action(async () => {
      await refresh();
      await recapture(example);
    });
  }
  async function close() {
    if (running) {
      failure =
        "Cancel the running job or wait before closing. A shell may already have changed files.";
      return;
    }
    if (capture) {
      await api.release(capture.id);
      capture = null;
    }
    dialog.close();
  }
  async function tool(role: api.Role) {
    await action(async () => {
      await api.chooseTool(role);
      await refresh();
    });
  }
  async function enable(kind: api.Kind) {
    await action(async () => {
      data = await api.enable(kind, !data?.[kind]);
    });
  }
  async function trust(kind: api.Kind) {
    await action(async () => {
      await api.trust(kind);
      await refresh();
    });
  }
  const toolId = (role: api.Role) =>
    data?.tools.filter((t) => t.role === role).at(-1)?.id ?? null;
  const rootId = (kind: api.Kind) =>
    data?.roots.filter((r) => r.kind === kind).at(-1)?.id ?? null;
  const shellSteps = $derived(
    block?.kind === "shell"
      ? [
          ...(!capture?.executable
            ? ["Save your own copy of this example before running it."]
            : []),
          ...(!data?.shell ? ["Enable shell for this session."] : []),
          ...(!rootId("shell")
            ? ["Trust the folder containing your saved document for shell."]
            : []),
          ...(!toolId("shell")
            ? ["Choose an installed shell (sh, bash or zsh)."]
            : []),
        ]
      : [],
  );
  const sqlSteps = $derived(
    block?.kind === "sqlite"
      ? [
          ...(!capture?.executable
            ? ["Save your own copy of this example before running it."]
            : []),
          ...(!data?.sqlite ? ["Enable SQLite for this session."] : []),
          ...(!rootId("sqlite")
            ? ["Trust the folder containing your saved document for SQLite."]
            : []),
          ...(!data?.databases.length
            ? [
                "Choose a local SQLite database. Read-only queries do not need a separate SQL tool.",
              ]
            : []),
        ]
      : [],
  );
  async function saveCopy() {
    await action(async () => {
      const previous = block;
      const next = await oncopy();
      if (!next) {
        notice = "No runnable copy was saved. You can try Save As again.";
        return;
      }
      if (capture) await api.release(capture.id);
      capture = next;
      saved = false;
      selected =
        next.blocks.find(
          (b) => b.kind === previous?.kind && b.line === previous?.line,
        )?.id ??
        next.blocks[0]?.id ??
        "";
      notice =
        "Saved and captured your copy. Complete the setup below, then review the code before running.";
    });
  }
  async function start(mode: api.Mode) {
    if (!capture || busy || running) return;
    await action(async () => {
      clearTimeout(timer);
      job = null;
      result = null;
      warning = "";
      rowAt = 0;
      exportId = null;
      if (mode === "pdf" || mode === "pdf_tex") {
        try {
          await documentMathPngs(capture!.text, (source, display, png) =>
            api.math(capture!.id, source, display, png),
          );
        } catch (e) {
          warning +=
            "Document math remains source where no validated raster is available: " +
            errorMessage(e) +
            "\n";
        }
      }
      if (mode === "pdf" || mode === "pdf_tex")
        for (const source of asset.diagrams) {
          try {
            await api.diagram(capture!.id, source, await diagramPng(source));
          } catch (e) {
            warning += "Diagram remains source: " + errorMessage(e) + "\n";
          }
        }
      const kind: api.Kind | null =
        mode === "shell"
          ? "shell"
          : mode.startsWith("sqlite")
            ? "sqlite"
            : mode === "tex" || mode === "pdf_tex"
              ? "tex"
              : null;
      const request: api.Request = {
        capture: capture!.id,
        block: block?.id ?? null,
        root: kind ? rootId(kind) : null,
        database: data?.databases.at(-1)?.id ?? null,
        tool:
          kind === "sqlite"
            ? null
            : toolId(
                mode === "pdf" ? "typst" : kind === "shell" ? "shell" : "tex",
              ),
        pandoc: toolId("pandoc"),
        mode,
        toc,
        pdfFonts: mode === "pdf" ? pdfFonts : "system",
      };
      await onsettle(capture!);
      const approval = await api.prepare(request);
      notice =
        "Review the native confirmation for the exact captured revision and code.";
      const next = await api.run(approval.token);
      if (!next) {
        notice = "Run canceled. No job started.";
        return;
      }
      job = next;
      result = {
        status: "running",
        message: "Running the confirmed captured revision.",
        output: null,
        table: null,
        pdf: false,
        warnings: [],
      };
      schedule();
    });
  }
  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(() => void update(), 250);
  }
  async function update() {
    if (!job) return;
    try {
      result = await api.poll(job);
      if (result.status === "running") schedule();
    } catch (e) {
      failure = errorMessage(e);
    }
  }
  async function cancel() {
    if (job) {
      await api.cancel(job);
      await update();
    }
  }
  async function save() {
    if (job)
      await action(async () => {
        exportId = await api.save(job!);
        if (exportId)
          notice =
            "PDF exported. Open it in the system viewer to inspect or print.";
      });
  }
  onDestroy(() => {
    clearTimeout(timer);
    if (job && running) void api.cancel(job).catch(() => {});
    if (capture) void api.release(capture.id).catch(() => {});
  });
</script>

<dialog
  class="power-panel"
  aria-label="Export and local jobs"
  bind:this={dialog}
  oncancel={(e) => {
    e.preventDefault();
    void close();
  }}
>
  <header>
    <div>
      <h2>Export & run</h2>
      <p>Choose a revision. Local execution stays off until you enable it.</p>
    </div>
    <button onclick={() => void close()} disabled={busy || running}
      >Close export and jobs</button
    >
  </header>
  {#if failure}<p role="alert">{failure}</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  <section class="power-capture">
    <label
      >Capture revision <select
        aria-label="Capture revision"
        bind:value={saved}
        disabled={busy || running}
        ><option value={true} disabled={!hasSaved}>Saved open snapshot</option
        ><option value={false} disabled={!hasWorking}>Working editor</option
        ></select
      ></label
    ><button onclick={() => void action(recapture)} disabled={busy || running}
      >Capture selected revision</button
    >
    {#if capture}<p><strong>{capture.name}</strong> · {capture.origin}</p>
      <p class="power-hash">SHA-256 {capture.revision}</p>{/if}
  </section>
  <nav>
    <button aria-pressed={tab === "pdf"} onclick={() => (tab = "pdf")}
      >PDF export</button
    ><button aria-pressed={tab === "run"} onclick={() => (tab = "run")}
      >Document fences {capture?.blocks.length ?? 0}</button
    >
  </nav>
  {#if tab === "pdf"}<section>
      <h3>A portable copy</h3>
      <p>
        Convert only this captured revision. Raw markup and external links are
        excluded. Choose image permissions explicitly. PDF export does not
        enable document execution. PDF uses the converter’s print layout and
        your chosen fonts; use HTML export to preserve the reading theme and
        bundled typography.
      </p>
      <div class="power-tools">
        <button disabled={busy || running} onclick={() => tool("pandoc")}
          >Choose Pandoc…</button
        ><span
          >{data?.tools.filter((t) => t.role === "pandoc").at(-1)?.version ??
            "No local Pandoc selected"}</span
        ><button disabled={busy || running} onclick={() => tool("typst")}
          >Choose Typst…</button
        ><span
          >{data?.tools.filter((t) => t.role === "typst").at(-1)?.version ??
            "No local Typst selected"}</span
        >
      </div>
      {#if !texPdf}<label>
          PDF fonts
          <select
            aria-label="PDF fonts"
            bind:value={pdfFonts}
            disabled={busy || running}
          >
            <option value="system">System fonts</option>
            <option value="bundled">Bundled fonts</option>
          </select>
        </label>
        <p>
          Bundled fonts avoid scanning installed fonts and include Arabic, Latin
          and math. Use System fonts for other scripts or installed families.
          Missing glyphs are reported in the result.
        </p>{/if}
      <label
        ><input type="checkbox" bind:checked={toc} disabled={busy || running} /> Include
        table of contents</label
      >
      <label
        ><input
          type="checkbox"
          bind:checked={texPdf}
          disabled={busy || running}
        /> Use separately trusted TeX instead of Typst</label
      >
      {#if texPdf}<p>
          Enable trusted TeX and select its tool/folder in Document fences
          first. Unicode PDF conversion uses xelatex or lualatex; shell escape
          stays disabled.
        </p>{/if}
      {#if asset.error}<p role="alert">{asset.error}</p>{/if}
      {#if asset.images.length}<h4>Image permissions</h4>
        {#each asset.images as reference (reference)}<div class="power-image">
            <code>{reference}</code><button
              disabled={busy || running}
              onclick={() =>
                action(async () => {
                  if (await api.image(capture!.id, reference))
                    notice = "Native-selected image approved for this capture.";
                })}>Choose this image…</button
            >
          </div>{/each}{/if}
      {#if asset.diagrams.length}<p>
          {asset.diagrams.length} local diagrams will be rendered as validated PNGs.
          Unsupported diagrams remain labeled source.
        </p>{/if}
      <button
        class="primary"
        disabled={busy ||
          running ||
          !capture ||
          !toolId("pandoc") ||
          !(texPdf ? toolId("tex") : toolId("typst"))}
        onclick={() => start(texPdf ? "pdf_tex" : "pdf")}>Prepare PDF…</button
      >
    </section>{:else}<section>
      <h3>Explicit local actions</h3>
      <p>
        Only closed top-level sh/bash/shell/zsh, sql/sqlite and tex/latex fences
        are offered. Select a native folder for each runner. Enabling and trust
        expire when the app exits. Nothing runs during open, Read, watch or HTML
        export.
      </p>
      {#each ["shell", "sqlite", "tex"] as kind (kind)}<div
          class="power-runner"
        >
          <strong
            >{kind === "tex"
              ? "Trusted TeX"
              : kind === "sqlite"
                ? "SQLite"
                : "Shell"}</strong
          ><button
            disabled={busy}
            aria-pressed={!!data?.[kind as api.Kind]}
            onclick={() => enable(kind as api.Kind)}
            >{data?.[kind as api.Kind] ? "Disable" : "Enable"} {kind}</button
          ><button
            disabled={busy || !data?.[kind as api.Kind]}
            onclick={() => trust(kind as api.Kind)}
            >Trust folder for {kind}…</button
          ><span
            >{data?.roots.filter((r) => r.kind === kind).at(-1)?.name ??
              "No trusted folder"}</span
          >
        </div>{/each}
      <p class="power-warning">
        Shell and TeX run with your normal OS privileges. Folder trust is not a
        sandbox. Cancellation stops the attached process group; detached
        processes and completed file changes may remain.
      </p>
      <div class="power-tools">
        <button disabled={busy || running} onclick={() => tool("shell")}
          >Choose shell…</button
        ><span
          >{data?.tools.filter((t) => t.role === "shell").at(-1)?.version ??
            "No shell selected"}</span
        ><button disabled={busy || running} onclick={() => tool("tex")}
          >Choose TeX engine…</button
        ><span
          >{data?.tools.filter((t) => t.role === "tex").at(-1)?.version ??
            "No TeX engine selected"}</span
        ><button
          disabled={busy || running}
          onclick={() =>
            action(async () => {
              await api.database();
              await refresh();
            })}>Choose SQLite database…</button
        ><span>{data?.databases.at(-1)?.name ?? "No database selected"}</span>
      </div>
      <label
        >Captured fence <select
          aria-label="Captured fence"
          bind:value={selected}
          disabled={busy || running}
          >{#each capture?.blocks ?? [] as b (b.id)}<option value={b.id}
              >{b.kind} · line {b.line}</option
            >{/each}</select
        ></label
      >
      {#if block?.kind === "shell" && shellSteps.length}
        <div aria-label="Shell setup required" role="status">
          <p>Before running this shell block:</p>
          <ul>
            {#each shellSteps as step (step)}<li>{step}</li>{/each}
          </ul>
          {#if !capture?.executable}<button
              disabled={busy || running}
              onclick={saveCopy}>Save a runnable copy…</button
            >{/if}
        </div>
      {/if}
      {#if block?.kind === "sqlite" && sqlSteps.length}
        <div aria-label="SQLite setup required" role="status">
          <p>Before running this SQL block:</p>
          <ul>
            {#each sqlSteps as step (step)}<li>{step}</li>{/each}
          </ul>
          {#if !capture?.executable}<button
              disabled={busy || running}
              onclick={saveCopy}>Save a runnable copy…</button
            >{/if}
        </div>
      {/if}
      {#if block}<textarea
          aria-label="Captured code"
          readonly
          value={block.code}
          rows={8}
        ></textarea>
        {#if block.kind === "sqlite"}<label
            ><input
              type="checkbox"
              bind:checked={write}
              disabled={busy || running}
            /> Request a DML write transaction</label
          >
          <p>
            Readonly is enforced by the connection and authorizer. ATTACH,
            extensions, pragmas and schema changes are denied. Unfinished writes
            roll back on error or cancel.
          </p>{/if}
        <button
          class="primary"
          disabled={busy ||
            running ||
            !capture?.executable ||
            !data?.[block.kind] ||
            (block.kind === "shell" && shellSteps.length > 0) ||
            (block.kind === "sqlite" && sqlSteps.length > 0)}
          onclick={() =>
            start(
              block!.kind === "sqlite"
                ? write
                  ? "sqlite_write"
                  : "sqlite_read"
                : block!.kind,
            )}>Run captured fence…</button
        >
      {:else}<p>
          No admitted runnable fences in this capture. Add one in Source, then
          capture the working revision.
        </p>{/if}
      {#if capture && !capture.executable}<p>
          Save As before running recovered text or app-owned documents.
        </p>{/if}
    </section>{/if}
  {#if warning}<pre class="power-warning">{warning}</pre>{/if}
  {#if result}<section class="power-result" aria-label="Local job result">
      <h3>{result.status}</h3>
      <p>{result.message}</p>
      {#if running}<button onclick={() => void cancel()}>Cancel job</button
        >{/if}{#if result.output}<p>
          Exit code {result.output.code ?? "signal"}
        </p>
        <pre aria-label="Job stdout">{result.output.stdout}</pre>
        <pre aria-label="Job stderr">{result.output
            .stderr}</pre>{/if}{#if result.table}<p>
          {result.table.rows.length} rows · {result.table.changes} changes{result
            .table.truncated
            ? " · results truncated"
            : ""}
        </p>
        <div>
          <button
            disabled={rowAt === 0}
            onclick={() => (rowAt = Math.max(0, rowAt - 25))}
            >Previous rows</button
          ><span
            >{rowAt + 1}–{Math.min(rowAt + 25, result.table.rows.length)} / {result
              .table.rows.length}</span
          ><button
            disabled={rowAt + 25 >= result.table.rows.length}
            onclick={() => (rowAt += 25)}>Next rows</button
          >
        </div>
        <p>25 rows per page; long cells show 1,024-character excerpts.</p>
        <div class="power-table">
          <table>
            <thead
              ><tr
                >{#each result.table.columns as column, index (index)}<th
                    >{column}</th
                  >{/each}</tr
              ></thead
            ><tbody
              >{#each result.table.rows.slice(rowAt, rowAt + 25) as row, rowIndex (rowIndex)}<tr
                  >{#each row as cell, cellIndex (cellIndex)}<td>{cell}</td
                    >{/each}</tr
                >{/each}</tbody
            >
          </table>
        </div>{/if}{#each result.warnings as text, warningIndex (warningIndex)}<p
          class="power-warning"
        >
          {text}
        </p>{/each}{#if result.pdf}<button disabled={busy} onclick={save}
          >Export PDF…</button
        >{#if exportId}<button
            disabled={busy}
            onclick={() => action(() => api.open(exportId!))}
            >Open PDF to inspect / print</button
          >{/if}{/if}
    </section>{/if}
  <footer>
    <p>
      Captures: 2 MiB · execution confirmation: 8 KiB · output: 256 KiB per
      stream · shell: 30 seconds · SQLite: 5 seconds / 500 rows. Tool jobs use
      private staging; no package installation or network bootstrap.
    </p>
    <button
      disabled={busy || running}
      onclick={() =>
        action(async () => {
          await api.clear();
          job = null;
          result = null;
          exportId = null;
          notice =
            "Completed job results cleared. Document/review/recovery data is kept.";
        })}>Clear completed job results</button
    >
  </footer>
</dialog>
