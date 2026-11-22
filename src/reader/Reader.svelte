<script lang="ts">
  import { mount, onMount, tick, unmount } from "svelte";
  import InlineFence from "../power/InlineFence.svelte";
  import Icon from "../ui/Icon.svelte";
  import { dock } from "../ui/dock";
  import type { Database } from "../power/bridge";
  import type { Fence, InlineRun } from "../power/fence";
  import { ReaderClient } from "./client";
  import { headingBoundary, matchingHeadings } from "./headings";
  import type { Read, Section, Heading, Page } from "./model";
  import { sanitizeReader, safeLink } from "./safe";
  import { hydrate, clearRichCache } from "../renderers/rich";
  import { themes, type Values } from "../preferences";
  import { exportStandalone } from "./standalone";
  import {
    FlowLayout,
    readerWindow,
    WHOLE_SOURCE_LIMIT,
    WHOLE_HTML_LIMIT,
    WINDOW_HTML_LIMIT,
  } from "./flow";
  let {
    read,
    embedded = false,
    toolbarTarget,
    focused = false,
    ontogglefocus = () => {},
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
    onrun,
    oncancelrun = () => {},
    onclearrun = () => {},
    getRun = () => undefined,
    runBusy = () => false,
    saveBeforeRun = () => false,
    getDatabase = () => undefined,
    onselectdatabase = () => {},
    onresetdatabase = () => {},
    title = "Document",
    revision = "Saved snapshot",
    operationBusy = false,
    presentation = {},
    dark = false,
    onpreference,
    preferences = { theme: "system", width: 760, size: 18 },
  }: {
    read: Read;
    embedded?: boolean;
    toolbarTarget?: HTMLElement;
    focused?: boolean;
    ontogglefocus?: () => void;
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
    onrun?: (fence: Fence) => void;
    oncancelrun?: (fence: Fence) => void;
    onclearrun?: (fence: Fence) => void;
    getRun?: (fence: Fence) => InlineRun | undefined;
    runBusy?: () => boolean;
    saveBeforeRun?: () => boolean;
    getDatabase?: () => Database | undefined;
    onselectdatabase?: () => void;
    onresetdatabase?: () => void;
    title?: string;
    revision?: string;
    operationBusy?: boolean;
    presentation?: Values;
    dark?: boolean;
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
  const contextReady = $derived(
    complete &&
      (pages.length > 0 || sections.length === 0) &&
      pages.every((item) => !item.page.provisional),
  );
  let outline = $state(false),
    query = $state(""),
    headingQuery = $state(""),
    matches = $state<{ section: number; excerpt: string }[]>([]),
    finding = $state(false),
    notice = $state(""),
    exporting = $state(false);
  let request = 0,
    restored = false;
  let optionsOpen = $state(false);
  let optionsElement = $state<HTMLDetailsElement>();
  let focusToggle = $state<HTMLButtonElement>();
  let activeAnchor: string | null = null;
  const filtered = $derived(matchingHeadings(headings, headingQuery));
  const currentHeading = $derived(
    headings[headingBoundary(headings, at + 1) - 1]?.id,
  );
  $effect(() => {
    if (focused) optionsOpen = false;
  });
  const helpId = `reader-help-${crypto.randomUUID()}`;
  const layout = new FlowLayout();
  let from = $state(0),
    to = $state(0);
  let before = $state(0),
    after = $state(0);
  let whole = $state(false),
    copying = $state(false),
    allSelected = $state(false);
  let forceWindow = false;
  let deferredRefresh = false;
  let loadingSection: number | null = null;
  let loadingJump = false,
    loadingIntent = 0,
    intentSerial = 0;
  let observer: ResizeObserver | undefined;
  let richObserver: IntersectionObserver | undefined;
  let frame = 0,
    refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let measuring = false;
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- Cache mutations do not drive the view.
  const cache = new Map<number, Page>();
  let cacheBytes = 0;
  const pageSize = (page: Page) =>
    2 * (page.html.length + page.rich.reduce((n, r) => n + r.source.length, 0));
  async function page(index: number) {
    const saved = cache.get(index);
    if (saved) return saved;
    const current = client!;
    const epoch = contextEpoch;
    const section = sections[index];
    const value = await current.page(index);
    if (
      current !== client ||
      epoch !== contextEpoch ||
      sections[index]?.end !== section.end
    )
      throw Error("Reader context changed");
    cache.set(index, value);
    cacheBytes += pageSize(value);
    while (cacheBytes > WHOLE_HTML_LIMIT && cache.size > 1) {
      const key = cache.keys().next().value!;
      cacheBytes -= pageSize(cache.get(key)!);
      cache.delete(key);
    }
    return value;
  }
  let contextEpoch = 0;
  function spacers() {
    const space = whole ? { before: 0, after: 0 } : layout.spacers(from, to);
    before = space.before;
    after = space.after;
  }
  function visibleAnchor() {
    if (!host) return null;
    const { top, bottom } = host.getBoundingClientRect();
    const intersects = (node: HTMLElement) => {
      const rect = node.getBoundingClientRect();
      return rect.bottom > top && rect.top < bottom;
    };
    const node =
      [...host.querySelectorAll<HTMLElement>("[data-source-line]")].find(
        intersects,
      ) ??
      [...host.querySelectorAll<HTMLElement>(".reader-chunk")].find(intersects);
    return node
      ? {
          node,
          section: node.closest<HTMLElement>("[data-section]")?.dataset.section,
          line: node.dataset.sourceLine,
          offset: node.getBoundingClientRect().top - top,
        }
      : null;
  }
  let expectedScroll = 0;
  function moveTo(top: number) {
    host.scrollTop = top;
    expectedScroll = host.scrollTop;
  }
  function viewportIndex() {
    const anchor = visibleAnchor();
    if (anchor?.section !== undefined) return Number(anchor.section);
    const padding = Number.parseFloat(getComputedStyle(host).paddingTop) || 0;
    return layout.atScroll(Math.max(0, host.scrollTop - padding), from, to);
  }
  function scrolled() {
    if (!host || !client) return;
    // Native scrollbar drags and external reveals have no wheel/key event.
    // Distinguish them from our own Source/layout synchronization by position.
    if (Math.abs(host.scrollTop - expectedScroll) > 1) intent();
    expectedScroll = host.scrollTop;
    scheduleScroll();
  }
  function restoreVisible(saved: ReturnType<typeof visibleAnchor>) {
    if (!saved) return;
    const block = host.querySelector(`[data-section="${saved.section}"]`);
    const node = saved.node.isConnected
      ? saved.node
      : saved.line !== undefined
        ? block?.querySelector(`[data-source-line="${saved.line}"]`)
        : block;
    if (node)
      moveTo(
        host.scrollTop +
          node.getBoundingClientRect().top -
          host.getBoundingClientRect().top -
          saved.offset,
      );
  }
  function selected() {
    const selection = document.getSelection();
    return (
      !!selection &&
      !selection.isCollapsed &&
      selection.rangeCount > 0 &&
      selection.getRangeAt(0).intersectsNode(host)
    );
  }
  async function show(section: number, jump = true, refresh = false) {
    if (!client || !sections.length) return false;
    const target = Math.max(
      0,
      Math.min(
        Number.isFinite(section) ? Math.trunc(section) : 0,
        sections.length - 1,
      ),
    );
    const full =
      !forceWindow && total <= WHOLE_SOURCE_LIMIT && sections.length <= 128;
    const range = full
      ? { from: 0, to: sections.length }
      : readerWindow(layout, target, host.clientHeight || 600);
    // Keep the live browser selection intact while the user extends it. Full
    // document copy is independent of this window and never copies a fragment.
    if (!jump && selected()) {
      range.from = Math.min(from, range.from);
      range.to = Math.max(to, range.to);
      if (range.to - range.from > 48) {
        notice =
          "Selection preserved. Copy or clear it to continue scrolling; Copy document Markdown includes the entire document.";
        return;
      }
    }
    if (
      !refresh &&
      full === whole &&
      range.from === from &&
      range.to === to &&
      pages.length &&
      pages.every((item) => item.page.provisional === !complete)
    ) {
      if (jump) scrollWithin(host.querySelector(`[data-section="${target}"]`));
      return true;
    }
    const epoch = ++request;
    const current = client;
    loadingSection = target;
    loadingJump = jump;
    const navigation = (loadingIntent = intentSerial);
    let saved = jump ? null : visibleAnchor();
    const scrollAtStart = host.scrollTop;
    const atEnd = () =>
      host.scrollTop > 0 &&
      host.scrollHeight - host.clientHeight - host.scrollTop < 2;
    let keepEnd = atEnd();
    const values: typeof pages = [];
    let bytes = 0;
    try {
      for (let i = range.from; i < range.to; i++) {
        const value = await page(i);
        if (epoch !== request || current !== client) return;
        bytes += pageSize(value);
        if (bytes > (full ? WHOLE_HTML_LIMIT : WINDOW_HTML_LIMIT)) {
          if (full) {
            forceWindow = true;
            return await show(target, jump, true);
          }
          // A single source block remains intact. Restrict its neighbours if
          // generated markup is unusually large rather than splitting words.
          if (i <= target) {
            values.length = 0;
            bytes = pageSize(value);
          } else break;
        }
        values.push({ index: i, page: value });
      }
      if (epoch !== request || current !== client) return;
      if (jump && navigation !== intentSerial) {
        if (sourceTarget) return false;
        return await show(viewportIndex(), false, refresh);
      }
      if (!jump && Math.abs(host.scrollTop - scrollAtStart) > 1) {
        saved = visibleAnchor();
        keepEnd = atEnd();
        const destination = viewportIndex();
        if (!values.some((value) => value.index === destination))
          return await show(destination, false, refresh);
      }
      // Selection may begin while native reads are in flight. Revalidate at
      // commit, not only when choosing the requested window.
      if (!jump && selected()) {
        if (range.from > from || range.to < to) {
          return await show(target, false, refresh);
        }
        if (
          pages.some(
            (old) =>
              !values.some(
                (value) =>
                  value.index === old.index &&
                  value.page.html === old.page.html &&
                  value.page.provisional === old.page.provisional,
              ),
          )
        ) {
          deferredRefresh = refresh;
          notice = "Selection preserved. Clear it to refresh the reading view.";
          return;
        }
      }
      const focused = document.activeElement as HTMLElement | null;
      const focusedBlock = focused?.closest<HTMLElement>("[data-section]");
      if (
        !jump &&
        focusedBlock &&
        host.contains(focusedBlock) &&
        !values.some(
          (value) => value.index === Number(focusedBlock.dataset.section),
        )
      )
        host.focus({ preventScroll: true });
      whole = full;
      from = values[0]?.index ?? range.from;
      to = (values.at(-1)?.index ?? from - 1) + 1;
      pages = values;
      spacers();
      await tick();
      if (epoch !== request || current !== client) return;
      expectedScroll = host.scrollTop;
      observeBlocks();
      await measureBlocks(null);
      if (epoch !== request || current !== client) return;
      if (jump) {
        at = target;
        activeAnchor = null;
        syncUntil = performance.now() + 200;
        scrollWithin(host.querySelector(`[data-section="${target}"]`));
        onposition(sections[target].start, anchor());
      } else if (keepEnd) moveTo(host.scrollHeight);
      else if (saved) restoreVisible(saved);
      else scrollWithin(host.querySelector(`[data-section="${target}"]`));
      rememberAnchor();
      ondisplay();
      await alignSource(false);
      scheduleScroll();
      return epoch === request && current === client;
    } catch (e) {
      if (
        current === client &&
        epoch === request &&
        !String(e).includes("context changed")
      )
        onerror(String(e));
    } finally {
      if (epoch === request) {
        loadingSection = null;
        if (sourceTarget) void alignSource();
      }
    }
  }
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- Tracks the lifetime of separately mounted DOM.
  const fences = new Map<HTMLElement, ReturnType<typeof mount>>();
  function removeFences(root?: HTMLElement) {
    for (const [anchor, component] of fences) {
      if (root && anchor.isConnected && !root.contains(anchor)) continue;
      void unmount(component);
      anchor.remove();
      fences.delete(anchor);
    }
  }
  function showFences(chunk: HTMLElement) {
    if (!onrun || total > 2 * 1024 * 1024) return;
    const section = sections[Number(chunk.dataset.section)];
    if (section?.line === undefined) return;
    for (const node of chunk.querySelectorAll<HTMLElement>(
      ":scope > div[data-run-kind]",
    )) {
      if (node.nextElementSibling?.classList.contains("reader-fence-tools"))
        continue;
      const kind = node.dataset.runKind;
      if (kind !== "shell" && kind !== "sqlite") continue;
      const source = node.dataset.runSource;
      if (!source) continue;
      const fence: Fence = {
        kind,
        // The native block line is the one-based first line of code.
        line: section.line + Number(node.dataset.sourceLine) + 1,
        code: JSON.parse(source) as string,
      };
      const anchor = document.createElement("div");
      anchor.className = "reader-fence-tools";
      node.after(anchor);
      const component = mount(InlineFence, {
        target: anchor,
        props: {
          fence,
          getRun: (value: Fence) => getRun(value),
          onrun: (value: Fence) => onrun?.(value),
          oncancel: (value: Fence) => oncancelrun(value),
          onclear: (value: Fence) => onclearrun(value),
          isBusy: () => operationBusy || runBusy(),
          needsSave: () => saveBeforeRun(),
          getDatabase: () => getDatabase(),
          onselectdatabase: () => onselectdatabase(),
          onresetdatabase: () => onresetdatabase(),
        },
      });
      fences.set(anchor, component);
    }
  }
  function observeBlocks() {
    for (const [anchor, component] of fences) {
      if (anchor.isConnected) continue;
      void unmount(component);
      fences.delete(anchor);
    }
    observer?.disconnect();
    observer?.observe(host);
    richObserver?.disconnect();
    for (const block of host.querySelectorAll<HTMLElement>(".reader-chunk")) {
      observer?.observe(block);
      richObserver?.observe(block);
      const page = pages.find(
        (item) => item.index === Number(block.dataset.section),
      )?.page;
      const html = page?.html ?? "";
      if (richSources.get(block) !== html) richBytes.delete(block);
      richSources.set(block, html);
      restoreImages(block);
      if (page && !page.provisional) showFences(block);
    }
  }
  let lastAnchor: ReturnType<typeof visibleAnchor> = null;
  let lastScroll = 0;
  function rememberAnchor() {
    lastAnchor = visibleAnchor();
    lastScroll = host.scrollTop;
  }
  function measure() {
    if (!host || !client) return;
    void measureBlocks(
      Math.abs(host.scrollTop - lastScroll) < 1 ? lastAnchor : visibleAnchor(),
    );
  }
  async function measureBlocks(saved = visibleAnchor()) {
    if (measuring || !host) return;
    measuring = true;
    for (const block of host.querySelectorAll<HTMLElement>(".reader-chunk"))
      layout.set(
        Number(block.dataset.section),
        block.getBoundingClientRect().height,
      );
    spacers();
    await tick();
    measuring = false;
    if (!client) return;
    expectedScroll = host.scrollTop;
    restoreVisible(saved);
    rememberAnchor();
    scheduleScroll();
  }
  const richSources = new WeakMap<HTMLElement, string>();
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- DOM retention accounting is not UI state.
  const richBytes = new Map<HTMLElement, { html: string; bytes: number }>();
  let hydrating = false,
    hydrationPending = false;
  async function hydrateVisible() {
    if (!client || !host) return;
    if (hydrating) {
      hydrationPending = true;
      return;
    }
    hydrating = true;
    const current = client;
    try {
      const bounds = host.getBoundingClientRect();
      for (const item of pages) {
        const block = host.querySelector<HTMLElement>(
          `[data-section="${item.index}"]`,
        );
        if (!block) continue;
        const rect = block.getBoundingClientRect();
        if (
          rect.bottom < bounds.top - host.clientHeight ||
          rect.top > bounds.bottom + host.clientHeight
        )
          continue;
        const eligible = (target: HTMLElement) => {
          const rect = target.getBoundingClientRect();
          return (
            rect.bottom >= bounds.top - host.clientHeight &&
            rect.top <= bounds.bottom + host.clientHeight
          );
        };
        if (
          ![
            ...block.querySelectorAll<HTMLElement>(
              '[data-rich]:not([data-hydrated="true"]),pre code:not([data-highlighted="true"])',
            ),
          ].some(
            (node) =>
              eligible(node) &&
              (node.dataset.rich !== undefined ||
                (!!node.textContent && node.textContent.length < 16000)),
          )
        )
          continue;
        const html =
          richBytes.get(block)?.html ??
          (block.querySelector(".reader-fence-tools")
            ? sanitizeReader(item.page.html)
            : block.innerHTML);
        const saved = visibleAnchor();
        const scrollAtStart = host.scrollTop;
        await hydrate(
          block,
          item.page.rich,
          () => current === client && block.isConnected,
          current.math,
          eligible,
        );
        if (current !== client) return;
        richBytes.set(block, { html, bytes: block.innerHTML.length * 2 });
        await measureBlocks(
          Math.abs(host.scrollTop - scrollAtStart) < 1
            ? saved
            : visibleAnchor(),
        );
        await alignSource(false);
      }
      let retained = 0;
      for (const [block, value] of richBytes) {
        if (!block.isConnected) richBytes.delete(block);
        else retained += value.bytes;
      }
      for (const [block, value] of richBytes) {
        if (retained <= 8 * 1024 * 1024) break;
        const rect = block.getBoundingClientRect();
        if (
          selected() ||
          (rect.bottom >= bounds.top - host.clientHeight &&
            rect.top <= bounds.bottom + host.clientHeight)
        )
          continue;
        removeFences(block);
        block.innerHTML = value.html;
        richBytes.delete(block);
        retained -= value.bytes;
        restoreImages(block);
        showFences(block);
      }
    } catch (e) {
      if (current === client) onerror(String(e));
    } finally {
      hydrating = false;
      if (hydrationPending && client) {
        hydrationPending = false;
        scheduleScroll();
      }
    }
  }
  function scheduleScroll() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      if (!client || !pages.length) return;
      rememberAnchor();
      const top = host.getBoundingClientRect().top;
      const blocks = [...host.querySelectorAll<HTMLElement>(".reader-chunk")];
      const block = blocks.find((b) => b.getBoundingClientRect().bottom > top);
      const inside =
        block && block.getBoundingClientRect().top <= top + host.clientHeight;
      const index = inside ? Number(block.dataset.section) : viewportIndex();
      if (at !== index) {
        at = index;
        onposition(sections[index]?.start ?? 0, anchor());
      }
      if (!whole && loadingSection === null) {
        const range = readerWindow(layout, index, host.clientHeight);
        if (!inside || range.from < from || range.to > to)
          void show(index, false);
      }
      void hydrateVisible();
      authorScroll();
    });
  }
  function scrollWithin(node: Element | null, center = false) {
    if (!node) return;
    moveTo(
      host.scrollTop +
        node.getBoundingClientRect().top -
        host.getBoundingClientRect().top -
        (center
          ? host.clientHeight / 2 - node.getBoundingClientRect().height / 2
          : 0),
    );
  }
  async function goHeading(heading: Heading, interactive = true) {
    if (interactive) intent();
    if (!(await show(heading.section))) return;
    activeAnchor = heading.id;
    const destination = host.querySelector<HTMLElement>(
      `#${CSS.escape(heading.id)}`,
    );
    if (destination) {
      if (interactive) {
        destination.tabIndex = -1;
        destination.focus({ preventScroll: true });
      }
      scrollWithin(destination);
    }
    onposition(sections[at].start, heading.id);
  }
  async function jumpMatch(section: number) {
    intent();
    if (!(await show(section))) return;
    const walker = document.createTreeWalker(
      host.querySelector(`[data-section="${section}"]`) ?? host,
      NodeFilter.SHOW_TEXT,
    );
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
    intentSerial++;
    syncUntil = 0;
    sourceTarget = null;
    manualScroll = true;
    activeAnchor = null;
    allSelected = false;
  }
  export async function revealByte(byte: number) {
    intent();
    const index = sections.findIndex((s) => byte >= s.start && byte < s.end);
    if (index >= 0) await show(index);
    else onerror("The saved section is not yet indexed.");
  }
  export async function revealSource(line: number, fraction = 0) {
    intentSerial++;
    activeAnchor = null;
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
    if (load && loadingSection !== null) return;
    if (!pages.some((page) => page.index === section)) {
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
    moveTo(
      host.scrollTop +
        node.getBoundingClientRect().top -
        host.getBoundingClientRect().top +
        ratio * node.getBoundingClientRect().height,
    );
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
  async function copyDocument() {
    if (!client || !complete) {
      notice = "Wait for document indexing before copying the whole document.";
      return;
    }
    if (copying) return;
    copying = true;
    const current = client;
    const revision = contextEpoch;
    try {
      // Read Markdown from the immutable native revision, independent of the
      // viewport. Never use innerText as a substitute for an entire document.
      const text = (async () => {
        let cursor = 0;
        const parts: string[] = [];
        while (cursor < total) {
          const part = await read(cursor);
          if (current !== client || revision !== contextEpoch)
            throw Error("Reader closed or changed");
          if (part.end <= cursor) throw Error("Document read made no progress");
          parts.push(part.text);
          cursor = part.end;
        }
        return parts.join("").replace(/^\uFEFF/, "");
      })();
      void text.catch(() => {});
      // Promise-backed ClipboardItem preserves the initiating user gesture in
      // WebKit while the immutable native ranges are being read.
      if (typeof ClipboardItem !== "undefined" && navigator.clipboard.write)
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/plain": text.then(
              (value) => new Blob([value], { type: "text/plain" }),
            ),
          }),
        ]);
      else await navigator.clipboard.writeText(await text);
      notice = "Document Markdown copied.";
    } catch (e) {
      notice = `Could not copy the document: ${String(e).slice(0, 180)}`;
    } finally {
      copying = false;
    }
  }
  function keydown(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      if (whole) {
        const range = document.createRange();
        range.selectNodeContents(host);
        const selection = document.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        return;
      }
      allSelected = true;
      notice =
        "Entire document selected. Copy copies its Markdown, including text outside the viewport.";
      return;
    }
    if (
      allSelected &&
      (event.ctrlKey || event.metaKey) &&
      event.key.toLowerCase() === "c"
    ) {
      event.preventDefault();
      void copyDocument();
      return;
    }
    if (
      ["ArrowDown", "ArrowUp", "PageDown", "PageUp", "Home", "End"].includes(
        event.key,
      )
    ) {
      intent();
      const target = event.target as HTMLElement;
      if (
        !target.closest("a,button,input,textarea,select,[contenteditable=true]")
      ) {
        event.preventDefault();
        const delta =
          event.key === "PageDown"
            ? host.clientHeight * 0.9
            : event.key === "PageUp"
              ? -host.clientHeight * 0.9
              : event.key === "ArrowDown"
                ? 40
                : -40;
        moveTo(
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? host.scrollHeight
              : host.scrollTop + delta,
        );
        scheduleScroll();
      }
    }
  }
  let findRequest = 0;
  function cancelFind() {
    findRequest++;
    finding = false;
    matches = [];
    client?.cancelFind();
  }
  async function find() {
    if (!client || !query.trim()) return;
    const current = client;
    const request = ++findRequest;
    finding = true;
    try {
      const result = await current.find(query);
      if (current !== client || request !== findRequest) return;
      matches = result.matches;
      notice = result.complete
        ? `${matches.length}${matches.length === 200 ? "+" : ""} matching locations`
        : "Results cover the scanned portion; search again when indexing finishes.";
    } catch (e) {
      if (current === client && request === findRequest) onerror(String(e));
    } finally {
      if (current === client && request === findRequest) finding = false;
    }
  }
  async function click(event: MouseEvent) {
    if ((event.target as Element).closest(".reader-fence-tools")) return;
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
          replaceImage(target, src);
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
  function replaceImage(target: HTMLElement, src: string) {
    const img = document.createElement("img");
    img.src = src;
    img.alt = target.textContent ?? "Image";
    img.decoding = "async";
    img.loading = "lazy";
    img.style.maxWidth = "100%";
    if (target.dataset.width) img.width = Number(target.dataset.width);
    if (target.dataset.height) img.height = Number(target.dataset.height);
    img.onload = measure;
    img.onerror = () => {
      const error = document.createElement("p");
      error.textContent =
        "Image decoding failed. The Markdown reference is preserved; choose another local image.";
      img.replaceWith(error);
    };
    target.replaceWith(img);
  }
  function restoreImages(block: HTMLElement) {
    for (const target of block.querySelectorAll<HTMLElement>(
      "button[data-image]",
    )) {
      const src = client?.approvedImages.get(target.dataset.image!);
      if (src) replaceImage(target, src);
    }
  }
  let requestedAnchor = $state<string | null>(null);
  $effect(() => {
    const anchor = requestedAnchor;
    if (anchor === null || !complete) return;
    requestedAnchor = null;
    void jumpAnchor(anchor);
  });
  export async function followAnchor(anchor: string) {
    if (complete) await jumpAnchor(anchor);
    else requestedAnchor = anchor;
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
    if (!client || !complete || !contextReady) {
      notice = "Wait for the reading view to finalize before exporting.";
      return;
    }
    const current = client;
    exporting = true;
    try {
      await exportStandalone(current, title, revision, presentation, dark);
      if (current === client) notice = "HTML export completed.";
    } catch (e) {
      if (current === client) onerror(String(e));
    } finally {
      if (current === client) exporting = false;
    }
  }
  export function action(command: "outline" | "find") {
    optionsOpen = false;
    if (command === "outline") void setOutline(!outline);
    if (command === "find") {
      void setOutline(true).then(() =>
        host
          .closest(".reader")
          ?.querySelector<HTMLInputElement>('[aria-label="Find in document"]')
          ?.focus(),
      );
    }
  }
  async function setOutline(open: boolean) {
    if (outline === open) return;
    const restore = captureLayout();
    outline = open;
    await tick();
    restore();
  }
  export function captureLayout() {
    if (!host) return () => {};
    const top = host.getBoundingClientRect().top;
    const node = [
      ...host.querySelectorAll("h1,h2,h3,p,pre,table,blockquote"),
    ].find((node) => node.getBoundingClientRect().bottom > top);
    const offset = node ? node.getBoundingClientRect().top - top : 0;
    let restored = false,
      expected = 0;
    return () => {
      if (!client || !host?.isConnected) return;
      // A delayed font load must not undo scrolling since the first restore.
      if (restored && Math.abs(host.scrollTop - expected) > 1) return;
      if (node?.isConnected) {
        moveTo(
          host.scrollTop +
            node.getBoundingClientRect().top -
            host.getBoundingClientRect().top -
            offset,
        );
        expected = host.scrollTop;
        restored = true;
      }
    };
  }
  export function focusReading(entered: boolean) {
    if (entered) host?.focus({ preventScroll: true });
    else focusToggle?.focus({ preventScroll: true });
  }
  export function anchor() {
    return (
      activeAnchor ??
      headings[headingBoundary(headings, at + 1) - 1]?.id ??
      null
    );
  }
  export function position() {
    return sections[at]?.start ?? start;
  }
  onMount(() => {
    if (focused) focusReading(true);
    observer = new ResizeObserver(measure);
    richObserver = new IntersectionObserver(
      () => {
        void hydrateVisible();
      },
      { root: host, rootMargin: "100% 0px" },
    );
    const selectionChanged = () => {
      if (!selected()) {
        if (deferredRefresh) {
          deferredRefresh = false;
          void show(at, false, true);
        } else scheduleScroll();
      }
    };
    document.addEventListener("selectionchange", selectionChanged);
    const closeOptions = (event: PointerEvent) => {
      if (optionsOpen && !optionsElement?.contains(event.target as Node))
        optionsOpen = false;
    };
    const escapeOptions = (event: KeyboardEvent) => {
      if (
        !optionsOpen ||
        event.key !== "Escape" ||
        event.defaultPrevented ||
        event.isComposing ||
        document.querySelector("dialog[open]")
      )
        return;
      event.preventDefault();
      optionsOpen = false;
      optionsElement?.querySelector("summary")?.focus();
    };
    document.addEventListener("pointerdown", closeOptions);
    document.addEventListener("keydown", escapeOptions);
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
        layout.resize(sections);
        if (complete && !wasComplete) {
          contextEpoch++;
        }
        if (complete && !sections.length) {
          ondisplay();
        }
        for (const [key, value] of cache) {
          if ((complete && !wasComplete) || key >= (update.replaceFrom ?? 0)) {
            cacheBytes -= pageSize(value);
            cache.delete(key);
          }
        }
        if (refreshTimer) clearTimeout(refreshTimer);
        if (!complete)
          refreshTimer = setTimeout(() => {
            if (client && restored) void show(at, false, true);
          }, 120);
        if (sourceTarget) {
          restored = true;
          if (complete && !wasComplete) void show(at, false, true);
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
            if (heading) void goHeading(heading, false);
            else void show(Math.max(0, found));
          }
        } else if (complete && !wasComplete) {
          const restoredHeading =
            !manualScroll && restoreAnchor
              ? headings.find(
                  (h) =>
                    h.id === restoreAnchor ||
                    h.title === restoreAnchor.replace(/^h-/, ""),
                )
              : undefined;
          if (restoredHeading) void goHeading(restoredHeading, false);
          else {
            const pending =
              loadingJump && loadingIntent === intentSerial
                ? loadingSection
                : null;
            void show(
              pending ?? (pages.length ? viewportIndex() : at),
              pending !== null,
              true,
            );
          }
        }
      },
      onerror,
    );
    client = current;
    return () => {
      request++;
      contextEpoch++;
      if (frame) cancelAnimationFrame(frame);
      if (refreshTimer) clearTimeout(refreshTimer);
      document.removeEventListener("selectionchange", selectionChanged);
      document.removeEventListener("pointerdown", closeOptions);
      document.removeEventListener("keydown", escapeOptions);
      observer?.disconnect();
      richObserver?.disconnect();
      cache.clear();
      richBytes.clear();
      removeFences();
      client = undefined;
      current.destroy();
      clearRichCache();
    };
  });
</script>

<div class="reader" class:reading-focus={focused} data-revision={revision}>
  {#if focused}
    <div
      class="reader-controls focus-controls"
      role="group"
      aria-label="Reading focus"
    >
      <span class="focus-title" dir="auto" {title}>{title}</span>
      <button
        class="reader-control"
        onclick={ontogglefocus}
        aria-label="Exit focus"
        title="Exit focus (Escape)"
      >
        <Icon name="focus" />Exit focus <kbd aria-hidden="true">Esc</kbd>
      </button>
    </div>
  {:else}
    <div
      class="reader-controls"
      class:docked={!!toolbarTarget}
      use:dock={toolbarTarget}
      role="group"
      aria-label="Reading controls"
    >
      <button
        class="reader-control outline-toggle"
        aria-expanded={outline}
        title="Outline & find"
        onclick={() => setOutline(!outline)}
      >
        <Icon name="list" /><span class="reader-control-label"
          >Outline &amp; find</span
        >
      </button>
      <details
        class="reader-options"
        bind:open={optionsOpen}
        bind:this={optionsElement}
      >
        <summary class="reader-control" title="Reading options"
          ><Icon name="type" /><span class="reader-control-label"
            >Reading options</span
          ></summary
        >
        <div class="reading-options-menu">
          <label
            >Theme <select
              aria-label="Reading theme"
              value={preferences.theme}
              onchange={(e) => onpreference("theme", e.currentTarget.value)}
              ><option value="system">Follow system</option
              >{#each Object.entries(themes) as [key, theme] (key)}<option
                  value={key}>{theme.label}</option
                >{/each}<option value="custom">Custom</option></select
            ></label
          >
          <label
            >Width <select
              aria-label="Reading width"
              value={String(preferences.width)}
              onchange={(e) =>
                onpreference("reader.width", e.currentTarget.value)}
              >{#if ![600, 760, 1000].includes(preferences.width)}<option
                  value={String(preferences.width)}
                  >{preferences.width}px</option
                >{/if}<option value="600">Narrow</option><option value="760"
                >Comfortable</option
              ><option value="1000">Wide</option></select
            ></label
          >
          <label
            >Text size <select
              aria-label="Reading text size"
              value={String(preferences.size)}
              onchange={(e) =>
                onpreference("reader.font_size", e.currentTarget.value)}
              >{#if ![16, 18, 22, 28].includes(preferences.size)}<option
                  value={String(preferences.size)}>{preferences.size}</option
                >{/if}<option value="16">16</option><option value="18"
                >18</option
              ><option value="22">22</option><option value="28">28</option
              ></select
            ></label
          >
          <button
            class="reader-copy"
            onclick={copyDocument}
            disabled={!complete || copying}
          >
            {copying ? "Copying…" : "Copy document Markdown"}
          </button>
        </div>
      </details>
      <button
        class="reader-control"
        onclick={exportHtml}
        disabled={!complete || !contextReady || exporting || operationBusy}
        title={exporting ? "Exporting…" : "Export HTML"}
      >
        <Icon name="export" /><span class="reader-control-label"
          >{exporting ? "Exporting…" : "Export HTML"}</span
        >
      </button>
      {#if !embedded}
        <button
          class="reader-control"
          bind:this={focusToggle}
          onclick={ontogglefocus}
          title="Focus reading"
        >
          <Icon name="focus" /><span class="reader-control-label"
            >Focus reading</span
          >
        </button>
      {/if}
      {#if onedit}<button
          class="reader-control"
          onclick={() => onedit?.(sections[at]?.start ?? 0)}
          disabled={operationBusy}>Edit here</button
        >{/if}
    </div>
  {/if}
  <span
    class="reader-progress"
    class:sr-only={contextReady}
    role="status"
    data-context-ready={contextReady}
    >{complete
      ? contextReady
        ? "Document ready"
        : "Finalizing reading view…"
      : `Resolving document context · ${total ? Math.round((scanned / total) * 100) : 0}%`}</span
  >
  {#if notice && !focused}<p class="reader-notice" role="status">
      {notice}
    </p>{/if}
  <div class="reading-layout">
    {#if outline && !focused}<aside
        class="outline"
        aria-label="Document navigation"
      >
        <label
          >Find in document<input
            aria-label="Find in document"
            type="search"
            bind:value={query}
            oninput={cancelFind}
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
              aria-current={h.id === currentHeading ? "location" : undefined}
              style:padding-inline-start={`${(h.level - 1) * 8 + 8}px`}
              onclick={() => goHeading(h)}><bdi>{h.title}</bdi></button
            >{/each}
        </nav>
        {#if headings.length > 200}<p>
            Filter to reach additional headings.
          </p>{/if}
      </aside>{/if}
    <!-- Navigation is delegated; document links never navigate the WebView. -->
    <span id={helpId} class="sr-only"
      >Large document: nearby text loads as you scroll. Outline and find
      searches the whole document. Export HTML provides the complete reading
      text; Copy document Markdown includes offscreen text.</span
    >
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
    <article
      class="reader-scroll prose"
      bind:this={host}
      aria-label="Markdown reader"
      onclick={click}
      onscroll={scrolled}
      tabindex="0"
      class:document-selected={allSelected}
      aria-describedby={whole ? undefined : helpId}
      onwheel={intent}
      ontouchmove={intent}
      onpointerdown={(event) => {
        if (event.button === 0) intent();
      }}
      oncopy={(event) => {
        if (allSelected) {
          event.preventDefault();
          void copyDocument();
        }
      }}
      onkeydown={keydown}
    >
      <div
        class="reader-spacer"
        aria-hidden="true"
        style:height={`${before}px`}
      ></div>
      {#each pages as item (`${item.index}:${item.page.provisional}`)}<div
          class="reader-chunk"
          data-section={item.index}
        >
          {#if item.page.fallback}<p class="context-note">
              {item.page.fallback}
            </p>{/if}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          {@html sanitizeReader(item.page.html)}
        </div>{/each}
      <div
        class="reader-spacer"
        aria-hidden="true"
        style:height={`${after}px`}
      ></div>
      {#if !pages.length}<p role="status">
          {complete && !sections.length
            ? "This document is empty."
            : "Opening document…"}
        </p>{/if}
    </article>
  </div>
</div>
