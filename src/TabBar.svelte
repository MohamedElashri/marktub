<script lang="ts">
  import { tick } from "svelte";
  import { closeTargets, TAB_LIMIT, type CloseAction } from "./tabs";
  import Icon from "./ui/Icon.svelte";
  type Tab = { key: number; name: string; dirty: boolean; busy: boolean };
  let {
    tabs,
    active,
    locked,
    onselect,
    onclose,
    onmove,
    onnew,
  }: {
    tabs: Tab[];
    active: number;
    locked: boolean;
    onselect: (index: number) => void;
    onclose: (keys: number[]) => Promise<void>;
    onmove: (key: number, to: number) => void;
    onnew: () => void;
  } = $props();
  let strip: HTMLDivElement;
  let menuElement: HTMLDetailsElement;
  let menu = $state(false);
  let contextElement = $state<HTMLDivElement>();
  let context = $state<{ key: number; x: number; y: number } | null>(null);
  const contextIndex = $derived(
    tabs.findIndex((tab) => tab.key === context?.key),
  );
  const closeActions: [CloseAction, string][] = [
    ["current", "Close tab"],
    ["others", "Close other tabs"],
    ["left", "Close tabs to the left"],
    ["right", "Close tabs to the right"],
    ["all", "Close all tabs"],
  ];
  let drag = $state<{
    key: number;
    pointer: number;
    x: number;
    target: number;
    moving: boolean;
  } | null>(null);
  let suppressClick = false;
  const remaining = $derived(tabs.filter((tab) => tab.key !== drag?.key));
  const before = $derived(
    drag?.moving ? remaining[drag.target]?.key : undefined,
  );
  const after = $derived(
    drag?.moving && drag.target === remaining.length
      ? remaining.at(-1)?.key
      : undefined,
  );
  $effect(() => {
    const key = tabs[active]?.key;
    if (key === undefined) return;
    void tick().then(() => {
      const tab = document.getElementById(`document-tab-${key}`);
      if (!tab || !strip) return;
      const rect = tab.getBoundingClientRect(),
        viewport = strip.getBoundingClientRect();
      if (rect.left < viewport.left)
        strip.scrollLeft -= viewport.left - rect.left;
      else if (rect.right > viewport.right)
        strip.scrollLeft += rect.right - viewport.right;
    });
  });
  $effect(() => {
    if (!menu) return;
    const key = tabs[active]?.key;
    void tick().then(() => {
      if (!menu || tabs[active]?.key !== key) return;
      const panel = menuElement?.querySelector("div"),
        selected = panel?.querySelector<HTMLButtonElement>(
          'button[aria-pressed="true"]',
        );
      if (!panel || !selected) return;
      const item = selected.getBoundingClientRect(),
        viewport = panel.getBoundingClientRect();
      if (item.top < viewport.top) panel.scrollTop -= viewport.top - item.top;
      else if (item.bottom > viewport.bottom)
        panel.scrollTop += item.bottom - viewport.bottom;
    });
  });
  async function select(index: number, focus = false) {
    onselect(index);
    menu = false;
    if (focus) {
      await tick();
      document
        .getElementById(`document-tab-${tabs[index]?.key}`)
        ?.focus({ preventScroll: true });
    }
  }
  function focusTab(key: number) {
    document
      .getElementById(`document-tab-${key}`)
      ?.focus({ preventScroll: true });
  }
  async function move(index: number, to: number) {
    const key = tabs[index].key;
    onmove(key, to);
    menu = false;
    context = null;
    await tick();
    focusTab(key);
  }
  async function close(tabsToClose: Tab[]) {
    menu = false;
    context = null;
    await onclose(tabsToClose.map((tab) => tab.key));
    await tick();
    if (!document.querySelector("dialog[open]")) focusTab(tabs[active].key);
  }
  async function openContext(key: number, x: number, y: number) {
    menu = false;
    context = { key, x, y };
    await tick();
    if (context?.key !== key || !contextElement) return;
    const { width, height } = contextElement.getBoundingClientRect();
    context.x = Math.max(4, Math.min(x, innerWidth - width - 4));
    context.y = Math.max(4, Math.min(y, innerHeight - height - 4));
    contextElement.querySelector<HTMLButtonElement>("button:enabled")?.focus();
  }
  function finish(event: PointerEvent, cancel = false) {
    if (!drag || drag.pointer !== event.pointerId) return;
    const moving = drag;
    drag = null;
    if (moving.moving) {
      suppressClick = true;
      if (!cancel && !locked) onmove(moving.key, moving.target);
    }
  }
</script>

<svelte:window
  onpointermove={(event) => {
    if (!drag || drag.pointer !== event.pointerId || locked) return;
    if (!drag.moving && Math.abs(event.clientX - drag.x) < 6) return;
    drag.moving = true;
    const nodes = [
      ...strip.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
    ].filter((node) => node.id !== `document-tab-${drag!.key}`);
    const target = nodes.findIndex((node) => {
      const rect = node.getBoundingClientRect();
      return event.clientX < rect.left + rect.width / 2;
    });
    drag.target = target < 0 ? nodes.length : target;
    const rect = strip.getBoundingClientRect();
    if (event.clientX < rect.left + 24) strip.scrollLeft -= 20;
    if (event.clientX > rect.right - 24) strip.scrollLeft += 20;
  }}
  onpointerup={(event) => {
    const rect = strip.getBoundingClientRect();
    finish(
      event,
      event.clientY < rect.top - 12 || event.clientY > rect.bottom + 12,
    );
  }}
  onpointercancel={(event) => finish(event, true)}
  onkeydown={(event) => {
    if (event.key !== "Escape" || event.isComposing) return;
    if (drag) {
      drag = null;
      suppressClick = true;
    }
    if (context) {
      const { key } = context;
      context = null;
      event.preventDefault();
      focusTab(tabs[active]?.key ?? key);
    }
    if (menu) {
      event.preventDefault();
      menu = false;
      menuElement.querySelector("summary")?.focus({ preventScroll: true });
    }
  }}
  onpointerdown={(event) => {
    if (!(event.target instanceof Node)) return;
    if (menu && !menuElement?.contains(event.target)) menu = false;
    if (context && !contextElement?.contains(event.target)) context = null;
  }}
  onresize={() => (context = null)}
/>

<nav class="tabbar" aria-label="Open documents" data-tauri-drag-region>
  <div
    class="tabs"
    style={`width:${tabs.length * 200}px`}
    bind:this={strip}
    role="tablist"
    aria-label="Document tabs"
    data-tauri-drag-region
  >
    {#each tabs as tab, index (tab.key)}
      <div
        class="tab"
        class:active={index === active}
        class:dragging={drag?.moving && drag.key === tab.key}
        class:drop-before={before === tab.key}
        class:drop-after={after === tab.key}
      >
        <button
          role="tab"
          id={`document-tab-${tab.key}`}
          aria-controls={`document-panel-${tab.key}`}
          tabindex={index === active ? 0 : -1}
          aria-selected={index === active}
          title={`${tab.name}${tab.dirty ? " — unsaved changes" : ""}. Drag to reorder.`}
          onpointerdown={(event) => {
            if (event.button !== 0 || locked) return;
            suppressClick = false;
            onselect(index);
            event.currentTarget.setPointerCapture(event.pointerId);
            drag = {
              key: tab.key,
              pointer: event.pointerId,
              x: event.clientX,
              target: index,
              moving: false,
            };
          }}
          onlostpointercapture={(event) => finish(event, true)}
          oncontextmenu={(event) => {
            event.preventDefault();
            if (!locked)
              void openContext(tab.key, event.clientX, event.clientY);
          }}
          onauxclick={(event) => {
            if (event.button !== 1) return;
            event.preventDefault();
            if (!locked && !tab.busy) void close([tab]);
          }}
          onkeydown={(event) => {
            if (
              event.key === "ContextMenu" ||
              (event.shiftKey && event.key === "F10")
            ) {
              event.preventDefault();
              if (!locked) {
                const { left, bottom } =
                  event.currentTarget.getBoundingClientRect();
                void openContext(tab.key, left, bottom);
              }
              return;
            }
            if (event.altKey || event.ctrlKey || event.metaKey) return;
            if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
              return;
            event.preventDefault();
            void select(
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? tabs.length - 1
                  : (index +
                      (event.key === "ArrowRight" ? 1 : -1) +
                      tabs.length) %
                    tabs.length,
              true,
            );
          }}
          onclick={() => {
            if (suppressClick) {
              suppressClick = false;
              return;
            }
            void select(index);
          }}
          ><Icon name="file" /><span>{tab.name}{tab.dirty ? " *" : ""}</span
          ></button
        >
        <button
          class="tab-close"
          aria-label={`Close ${tab.name}`}
          disabled={locked || tab.busy}
          onclick={() => close([tab])}>×</button
        >
      </div>
    {/each}
  </div>
  <button
    class="new-tab-button"
    aria-label="New tab"
    title={tabs.length >= TAB_LIMIT
      ? `${TAB_LIMIT} tabs are open. Close one to open another.`
      : "New tab"}
    disabled={locked || tabs.length >= TAB_LIMIT}
    onclick={onnew}>+</button
  >
  <details class="tab-menu" bind:this={menuElement} bind:open={menu}>
    <summary aria-label="Tab actions" title="Open tabs and tab actions"
      >⌄</summary
    >
    <div>
      <span class="menu-caption">Open tabs</span>
      {#each tabs as tab, index (tab.key)}<button
          aria-pressed={index === active}
          onclick={() => select(index, true)}
          >{tab.name}{tab.dirty ? " *" : ""}</button
        >{/each}
      <hr />
      {@render tabActions(active)}
    </div>
  </details>
</nav>

{#snippet tabActions(index: number)}
  {#each closeActions as [action, label] (action)}
    {@const targets = closeTargets(tabs, index, action)}
    <button
      disabled={locked || !targets.length || targets.some((tab) => tab.busy)}
      onclick={() => close(targets)}>{label}</button
    >
  {/each}
  <hr />
  <button
    disabled={locked || index === 0}
    onclick={() => move(index, index - 1)}>Move tab left</button
  >
  <button
    disabled={locked || index === tabs.length - 1}
    onclick={() => move(index, index + 1)}>Move tab right</button
  >
{/snippet}

{#if context && contextIndex >= 0}
  <div
    class="tab-context-menu"
    role="group"
    aria-label={`Tab actions for ${tabs[contextIndex].name}`}
    style:left={`${context.x}px`}
    style:top={`${context.y}px`}
    bind:this={contextElement}
  >
    {@render tabActions(contextIndex)}
  </div>
{/if}
