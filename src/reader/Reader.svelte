<script lang="ts">
  import { onMount, tick } from "svelte";
  import { ReaderClient } from "./client";
  import type { Read, Section, Heading, Page } from "./model";
  import { sanitizeReader, safeLink } from "./safe";
  import { hydrate, clearRichCache } from "../renderers/rich";
  import { exportStandalone } from "./standalone";
  let {
    read,
    embedded = false,
    onedit,
    ondisplay = () => {},
    onsource = () => {},
    start = 0,
    anchorId = null,
    onposition,
    onlink,
    onnote = () => {},
    onimage,
    onerror,
    title = "Document",
    revision = "Saved snapshot",
    operationBusy = false,
    onpreference,
    preferences = { theme: "system", width: 760, size: 18 },
  }: {
    read: Read;
    embedded?: boolean;
    ondisplay?: () => void;
    onedit?: (byte: number) => void | Promise<void>;
    onsource?: (line: number, clicked: boolean, fraction?: number) => void;
    start?: number;
    anchorId?: string | null;
    onposition: (byte: number, anchor: string | null) => void;
    onlink: (link: string) => void;
    onnote?: (target: string) => void;
    onimage: (
      link: string,
    ) => Promise<{ dataUrl: string; width: number; height: number } | null>;
    onerror: (message: string) => void;
    title?: string;
    revision?: string;
    operationBusy?: boolean;
    onpreference: (key: string, value: string) => void;
    preferences?: { theme: string; width: number; size: number };
  } = $props();
  let client: ReaderClient | undefined, host: HTMLElement;
  let sections = $state<Section[]>([]),
    headings = $state<Heading[]>([]),
    complete = $state(false),
    scanned = $state(0),
    total = $state(0),
    at = $state(0),
    pages = $state<{ index: number; page: Page }[]>([]);
  let focus = $state(true),
    outline = $state(false),
    query = $state(""),
    headingQuery = $state(""),
    matches = $state<{ section: number; excerpt: string }[]>([]),
    finding = $state(false),
    notice = $state(""),
    exporting = $state(false);
  let request = 0,
    restored = false;
  let activeAnchor: string | null = null;
  const filtered = $derived(
    headings
      .filter((h) =>
        h.title.toLocaleLowerCase().includes(headingQuery.toLocaleLowerCase()),
      )
      .slice(0, 200),
  );
  $effect(() => {
    if (embedded) return;
    document.documentElement.dataset.readingFocus = String(focus);
    return () => {
      delete document.documentElement.dataset.readingFocus;
    };
  });
  let loadingSection: number | null = null;
  async function show(section: number) {
    if (!client || !sections.length) return;
    const epoch = ++request;
    at = Math.max(0, Math.min(section, sections.length - 1));
    loadingSection = at;
    const current = client;
    const values: typeof pages = [];
    try {
      for (let i = at; i < Math.min(at + 3, sections.length); i++)
        values.push({ index: i, page: await current.page(i) });
      if (epoch !== request || current !== client) return;
      pages = values;
      activeAnchor =
        headings.find((h) => h.section === at)?.id ??
        headings.filter((h) => h.section < at).at(-1)?.id ??
        null;
      onposition(sections[at].start, anchor());
      await tick();
      if (epoch !== request || current !== client) return;
      ondisplay();
      syncUntil = performance.now() + 200;
      host.scrollTop = 0;
      await alignSource(false);
      for (const value of values) {
        const block = host.querySelector<HTMLElement>(
          `[data-section="${value.index}"]`,
        );
        if (block)
          void hydrate(
            block,
            value.page.rich,
            () => request === epoch && current === client,
            current.math,
          )
            .then(() => alignSource(false))
            .catch((e) => onerror(String(e)));
      }
    } catch (e) {
      if (current === client) onerror(String(e));
    } finally {
      if (epoch === request) loadingSection = null;
    }
  }
  function scrollWithin(node: Element | null, center = false) {
    if (!node) return;
    host.scrollTop +=
      node.getBoundingClientRect().top -
      host.getBoundingClientRect().top -
      (center
        ? host.clientHeight / 2 - node.getBoundingClientRect().height / 2
        : 0);
  }
  async function goHeading(heading: Heading) {
    intent();
    await show(heading.section);
    activeAnchor = heading.id;
    scrollWithin(host.querySelector(`#${CSS.escape(heading.id)}`));
    onposition(sections[at].start, heading.id);
  }
  async function jumpMatch(section: number) {
    intent();
    await show(section);
    const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode as Text;
      const at = node.data
        .toLocaleLowerCase()
        .indexOf(query.toLocaleLowerCase());
      if (at < 0) continue;
      const mark = document.createElement("mark");
      const rest = node.splitText(at);
      rest.splitText(query.length);
      rest.replaceWith(mark);
      mark.textContent = rest.data;
      scrollWithin(mark, true);
      break;
    }
  }
  let syncUntil = 0;
  let manualScroll = false;
  let sourceTarget: { line: number; fraction: number } | null = null;
  function intent() {
    syncUntil = 0;
    sourceTarget = null;
    manualScroll = true;
  }
  export async function revealByte(byte: number) {
    const index = sections.findIndex((s) => byte >= s.start && byte < s.end);
    if (index >= 0) await show(index);
    else onerror("The saved section is not yet indexed.");
  }
  export async function revealSource(line: number, fraction = 0) {
    sourceTarget = { line, fraction };
    manualScroll = false;
    syncUntil = performance.now() + 200;
    await alignSource();
  }
  async function alignSource(load = true) {
    const target = sourceTarget;
    if (!target || !client) return;
    const { line, fraction } = target;
    let section = -1;
    for (let i = sections.length - 1; i >= 0; i--) {
      const s = sections[i];
      if (
        s.line !== undefined &&
        line >= s.line &&
        line <= Math.max(s.line, s.endLine ?? s.line)
      ) {
        section = i;
        break;
      }
    }
    if (section < 0) return;
    if (
      load &&
      loadingSection !== null &&
      section >= loadingSection &&
      section < loadingSection + 3
    )
      return;
    if (
      !pages.some((page) => page.index === section) ||
      (load &&
        loadingSection !== null &&
        (section < loadingSection || section >= loadingSection + 3))
    ) {
      if (load) await show(section);
      return;
    }
    await tick();
    if (sourceTarget !== target) return;
    const local = line - sections[section].line!;
    const block = host.querySelector(`[data-section="${section}"]`);
    const node = [
      ...(block?.querySelectorAll<HTMLElement>("[data-source-line]") ?? []),
    ]
      .filter((n) => Number(n.dataset.sourceLine) <= local)
      .at(-1);
    if (!node) {
      scrollWithin(block);
      return;
    }
    const start = Number(node.dataset.sourceLine);
    const end = Number(node.dataset.sourceEndLine ?? start + 1);
    const ratio = Math.max(
      0,
      Math.min(1, (local - start + fraction) / Math.max(1, end - start)),
    );
    host.scrollTop +=
      node.getBoundingClientRect().top -
      host.getBoundingClientRect().top +
      ratio * node.getBoundingClientRect().height;
    syncUntil = performance.now() + 200;
  }
  function sourceAnchor(node: HTMLElement, clicked = false, ratio = 0) {
    const section = node.closest<HTMLElement>("[data-section]");
    if (!section) return;
    const index = Number(section.dataset.section);
    if (sections[index]?.line === undefined) return;
    const start = Number(node.dataset.sourceLine ?? 0);
    const end = Number(node.dataset.sourceEndLine ?? start + 1);
    const position =
      start + Math.max(0, Math.min(0.999999, ratio)) * Math.max(1, end - start);
    const line = sections[index].line! + Math.floor(position);
    if (embedded) sourceTarget = { line, fraction: position % 1 };
    onsource(line, clicked, position % 1);
  }
  function authorScroll() {
    if (!embedded || performance.now() < syncUntil || !manualScroll) return;
    const top = host.getBoundingClientRect().top;
    const nodes = [...host.querySelectorAll<HTMLElement>("[data-source-line]")];
    // Prefer the innermost mapped block at the viewport edge (for lists/tables),
    // rather than an ancestor spanning several unrelated source lines.
    const crossing = nodes.filter((node) => {
      const r = node.getBoundingClientRect();
      return r.top <= top && r.bottom > top;
    });
    const node =
      crossing.at(-1) ??
      nodes.find((node) => node.getBoundingClientRect().bottom > top);
    if (node) {
      const r = node.getBoundingClientRect();
      sourceAnchor(node, false, (top - r.top) / Math.max(1, r.height));
    }
  }
  async function copyShown() {
    try {
      await navigator.clipboard.writeText(host.innerText);
      notice = "Shown text copied.";
    } catch {
      notice =
        "Clipboard unavailable. Select text and use the system Copy command.";
    }
  }
  async function find() {
    if (!client || !query.trim()) return;
    finding = true;
    try {
      const result = await client.find(query);
      matches = result.matches;
      notice = result.complete
        ? `${matches.length}${matches.length === 200 ? "+" : ""} matching sections`
        : "Results cover the scanned portion; search again when indexing finishes.";
    } catch (e) {
      onerror(String(e));
    } finally {
      finding = false;
    }
  }
  async function click(event: MouseEvent) {
    const target = (event.target as Element).closest<HTMLElement>(
      "a,button[data-image]",
    );
    if (!target) {
      if (embedded) {
        const node = (event.target as Element).closest<HTMLElement>(
          "[data-source-line]",
        );
        if (node) sourceAnchor(node, true);
      }
      return;
    }
    event.preventDefault();
    if (operationBusy) {
      notice = "Wait for the current file operation.";
      return;
    }
    if (target.dataset.image !== undefined) {
      try {
        const image = await onimage(target.dataset.image);
        const src = image?.dataUrl;
        if (src && target.isConnected) {
          if (image) client?.approveImage(target.dataset.image!, image);
          const img = document.createElement("img");
          img.src = src;
          img.alt = target.textContent ?? "Image";
          img.decoding = "async";
          img.onerror = () => {
            const error = document.createElement("p");
            error.textContent =
              "Image decoding failed. The Markdown reference is preserved; choose another local image.";
            img.replaceWith(error);
          };
          img.loading = "lazy";
          img.style.maxWidth = "100%";
          if (target.dataset.width) img.width = Number(target.dataset.width);
          if (target.dataset.height) img.height = Number(target.dataset.height);
          target.replaceWith(img);
        }
      } catch (e) {
        onerror(String(e));
      }
      return;
    }
    if (target.dataset.noteLink) {
      onnote(target.dataset.noteLink);
      return;
    }
    const link = target.getAttribute("href");
    if (!link || !safeLink(link)) return;
    if (link.startsWith("#")) {
      await followAnchor(link.slice(1));
    } else onlink(link);
  }
  let requestedAnchor = $state<string | null>(null);
  $effect(() => {
    const anchor = requestedAnchor;
    if (anchor === null || !complete) return;
    requestedAnchor = null;
    void jumpAnchor(anchor);
  });
  export async function followAnchor(anchor: string) {
    requestedAnchor = anchor;
  }
  async function jumpAnchor(anchor: string) {
    let id: string;
    try {
      id = decodeURIComponent(anchor);
    } catch {
      notice = "This anchor has invalid URL encoding.";
      return;
    }
    const heading = headings.find(
      (h) => h.id === id || h.id === `h-${id}` || h.title === id,
    );
    if (heading) {
      await goHeading(heading);
    } else notice = "This anchor is not indexed.";
  }
  export async function exportHtml() {
    if (!client || !complete) {
      notice = "Wait for the document context scan before exporting.";
      return;
    }
    exporting = true;
    try {
      await exportStandalone(client, title, revision);
      notice = "HTML export completed.";
    } catch (e) {
      onerror(String(e));
    } finally {
      exporting = false;
    }
  }
  export function action(command: string) {
    if (command === "focus") focus = !focus;
    if (command === "outline") outline = !outline;
    if (command === "find") {
      outline = true;
      focus = false;
      void tick().then(() =>
        host
          .closest(".reader")
          ?.querySelector<HTMLInputElement>('[aria-label="Find in document"]')
          ?.focus(),
      );
    }
  }
  export function captureLayout() {
    if (!host) return () => {};
    const top = host.getBoundingClientRect().top;
    const node = [
      ...host.querySelectorAll("h1,h2,h3,p,pre,table,blockquote"),
    ].find((node) => node.getBoundingClientRect().bottom > top);
    const offset = node ? node.getBoundingClientRect().top - top : 0;
    return () => {
      if (node?.isConnected)
        host.scrollTop +=
          node.getBoundingClientRect().top -
          host.getBoundingClientRect().top -
          offset;
    };
  }
  export function navigation() {
    focus = false;
  }
  export function anchor() {
    return (
      activeAnchor ?? headings.filter((h) => h.section <= at).at(-1)?.id ?? null
    );
  }
  export function position() {
    return sections[at]?.start ?? start;
  }
  onMount(() => {
    if (embedded) focus = false;
    const restoreAnchor = anchorId;
    const current = new ReaderClient(
      read,
      (update) => {
        const wasComplete = complete;
        sections = update.sections;
        headings = update.headings;
        complete = update.complete;
        scanned = update.scanned;
        total = update.total;
        if (sourceTarget) {
          restored = true;
          if (complete && !wasComplete) void show(at);
          void alignSource();
          return;
        }
        if (!restored && sections.length) {
          const found = sections.findIndex((s) => s.end > start);
          if (found >= 0 || complete) {
            restored = true;
            const heading = restoreAnchor
              ? headings.find(
                  (h) =>
                    h.id === restoreAnchor ||
                    h.title === restoreAnchor.replace(/^h-/, ""),
                )
              : undefined;
            if (heading) void goHeading(heading);
            else void show(Math.max(0, found));
          }
        } else if (complete && !wasComplete) {
          const restoredHeading = restoreAnchor
            ? headings.find(
                (h) =>
                  h.id === restoreAnchor ||
                  h.title === restoreAnchor.replace(/^h-/, ""),
              )
            : undefined;
          if (restoredHeading) void goHeading(restoredHeading);
          else void show(at);
        }
      },
      onerror,
    );
    client = current;
    return () => {
      request++;
      client = undefined;
      current.destroy();
      clearRichCache();
    };
  });
</script>

<div class="reader" class:reading-focus={focus} data-revision={revision}>
  <div class="reader-controls" aria-label="Reading controls">
    <button aria-pressed={!focus} onclick={() => (focus = !focus)}
      >{focus ? "Exit focus" : "Focus reading"}</button
    >
    <button aria-expanded={outline} onclick={() => (outline = !outline)}
      >Outline & find</button
    >
    <button
      onclick={exportHtml}
      disabled={!complete || exporting || operationBusy}
      >{exporting ? "Exporting…" : "Export HTML"}</button
    >
    {#if !focus}
      <button onclick={copyShown}>Copy shown text</button>
      <label
        >Theme <select
          aria-label="Reading theme"
          value={preferences.theme}
          onchange={(e) => onpreference("theme", e.currentTarget.value)}
          ><option value="system">Follow system</option><option value="light"
            >Light</option
          ><option value="paper">Parchment</option><option value="dark"
            >Dark</option
          ><option value="midnight">Ink</option><option value="custom"
            >Custom</option
          ></select
        ></label
      >
      <label
        >Width <select
          aria-label="Reading width"
          value={String(preferences.width)}
          onchange={(e) => onpreference("reader.width", e.currentTarget.value)}
          >{#if ![600, 760, 1000].includes(preferences.width)}<option
              value={String(preferences.width)}>{preferences.width}px</option
            >{/if}<option value="600">Narrow</option><option value="760"
            >Comfortable</option
          ><option value="1000">Wide</option></select
        ></label
      >
      <label
        >Text <select
          aria-label="Reading text size"
          value={String(preferences.size)}
          onchange={(e) =>
            onpreference("reader.font_size", e.currentTarget.value)}
          >{#if ![16, 18, 22, 28].includes(preferences.size)}<option
              value={String(preferences.size)}>{preferences.size}</option
            >{/if}<option value="16">16</option><option value="18">18</option
          ><option value="22">22</option><option value="28">28</option></select
        ></label
      >
    {/if}
    {#if onedit}<button
        onclick={() => onedit?.(sections[at]?.start ?? 0)}
        disabled={operationBusy}>Edit this section</button
      >{/if}
    <span role="status"
      >{complete
        ? `${sections.length.toLocaleString()} sections`
        : `Resolving document context · ${total ? Math.round((scanned / total) * 100) : 0}%`}</span
    >
  </div>
  {#if notice}<p class="reader-notice" role="status">{notice}</p>{/if}
  <div class="reading-layout">
    {#if outline}<aside class="outline" aria-label="Document navigation">
        <label
          >Find in document<input
            aria-label="Find in document"
            type="search"
            bind:value={query}
            onkeydown={(e) => {
              if (e.key === "Enter") void find();
            }}
          /></label
        ><button onclick={find} disabled={finding}
          >{finding ? "Searching…" : "Find"}</button
        >
        {#each matches as match (match.section)}<button
            class="find-result"
            onclick={() => jumpMatch(match.section)}>{match.excerpt}</button
          >{/each}
        <label
          >Filter headings<input
            type="search"
            bind:value={headingQuery}
          /></label
        >
        <nav aria-label="Headings">
          {#each filtered as h (h.id)}<button
              style:padding-left={`${(h.level - 1) * 10 + 8}px`}
              onclick={() => goHeading(h)}>{h.title}</button
            >{/each}
        </nav>
        {#if headings.length > 200}<p>
            Filter to reach additional headings.
          </p>{/if}
      </aside>{/if}
    <!-- Navigation is delegated; document links never navigate the WebView. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <article
      class="reader-scroll prose"
      bind:this={host}
      aria-label="Markdown reader"
      onclick={click}
      onscroll={authorScroll}
      onwheel={intent}
      ontouchmove={intent}
      onpointerdown={intent}
      onkeydown={(e) => {
        if (
          [
            "ArrowDown",
            "ArrowUp",
            "PageDown",
            "PageUp",
            "Home",
            "End",
          ].includes(e.key)
        )
          intent();
      }}
    >
      {#each pages as item (item.index)}<section data-section={item.index}>
          {#if item.page.provisional}<p class="context-note">
              Provisional: forward definitions are still being indexed.
            </p>{/if}
          {#if item.page.fallback}<p class="context-note">
              {item.page.fallback}
            </p>{/if}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          {@html sanitizeReader(item.page.html)}
        </section>{/each}
      {#if !pages.length}<p role="status">
          {complete ? "This document is empty." : "Opening the first section…"}
        </p>{/if}
    </article>
  </div>
  <nav class="section-navigation" aria-label="Reader sections">
    <button
      onclick={() => {
        intent();
        void show(at - 3);
      }}
      disabled={at === 0}>Previous sections</button
    >
    <label
      >Section <input
        aria-label="Go to section"
        type="number"
        min="1"
        max={sections.length}
        value={at + 1}
        onchange={(e) => {
          intent();
          void show(Number(e.currentTarget.value) - 1);
        }}
      />
      / {sections.length.toLocaleString()}</label
    >
    <button
      onclick={() => {
        intent();
        void show(at + 3);
      }}
      disabled={at + 3 >= sections.length}>Next sections</button
    >
  </nav>
</div>
