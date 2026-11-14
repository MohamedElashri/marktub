<script lang="ts">
  import { tick } from "svelte";
  import * as bridge from "./bridge";
  import {
    catalogue,
    fonts,
    preview,
    literals,
    put,
    appearance,
    currentPalette,
    osDark,
    themes,
    type Spec,
    type Values,
  } from "./preferences";
  import { chord } from "./commands";
  let {
    onpreview,
    oncommit,
  }: { onpreview: (v: Values | null) => void; oncommit: () => Promise<void> } =
    $props();
  let dialog: HTMLDialogElement;
  let specs = $state<Spec[]>([]),
    families = $state<string[]>([]),
    documentId = $state<string | null>(null),
    scope = $state("global"),
    category = $state("Appearance"),
    query = $state(""),
    text = $state(""),
    effective = $state<Values>({}),
    diagnostics = $state<{ line: number; message: string }[]>([]),
    error = $state(""),
    busy = $state(false),
    resetAll = $state(false);
  let version = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let capturedFocus: HTMLElement | null = null;
  const categories = [
    "Appearance",
    "Reading",
    "Source",
    "Custom colors",
    "Shortcuts",
    "Advanced",
  ];
  const assigned = $derived(literals(text));
  let shown = $derived(
    specs.filter(
      (s) =>
        s.key !== "schema.version" &&
        (query
          ? `${s.label} ${s.key} ${s.category}`
              .toLowerCase()
              .includes(query.toLowerCase())
          : s.category === category),
    ),
  );
  async function loadScope() {
    busy = true;
    const request = ++version;
    clearTimeout(timer);
    onpreview(null);
    try {
      const result = await bridge.getSettings(documentId, scope);
      if (request !== version) return;
      text = result.text;
      effective = result.resolved;
      diagnostics = result.diagnostics;
      error = "";
    } catch (e) {
      if (request === version) error = bridge.errorMessage(e);
    } finally {
      if (request === version) busy = false;
    }
    return request;
  }
  export async function show(id: string | null) {
    // Reserve before the first await: cold catalogue/font discovery must not
    // leave an editable draft that a later scope load can overwrite.
    busy = true;
    const opening = ++version;
    clearTimeout(timer);
    documentId = id;
    scope = "global";
    category = "Appearance";
    query = "";
    resetAll = false;
    error = "";
    capturedFocus = document.activeElement as HTMLElement | null;
    dialog.showModal();
    try {
      if (!specs.length)
        [specs, families] = await Promise.all([catalogue(), fonts()]);
      if (opening !== version || !dialog.open) return;
      const loaded = await loadScope();
      await tick();
      if (loaded === version && dialog.open)
        dialog
          .querySelector<HTMLInputElement>('[aria-label="Search preferences"]')
          ?.focus();
    } catch (e) {
      if (opening === version) {
        error = bridge.errorMessage(e);
        busy = false;
      }
    }
  }
  function cancel() {
    ++version;
    busy = false;
    clearTimeout(timer);
    onpreview(null);
    dialog.close();
    capturedFocus?.focus();
  }
  function changed() {
    const request = ++version;
    clearTimeout(timer);
    timer = setTimeout(() => {
      void validate(request);
    }, 160);
  }
  async function validate(request = ++version) {
    try {
      const result = await preview(documentId, scope, text);
      if (request !== version) return false;
      diagnostics = result.diagnostics;
      error = "";
      if (!diagnostics.length) {
        effective = result.resolved;
        onpreview(effective);
        return true;
      }
      return false;
    } catch (e) {
      if (request === version) error = bridge.errorMessage(e);
      return false;
    }
  }
  function set(spec: Spec, value: string | boolean) {
    if (spec.kind === "color" && effective.theme?.value !== "custom") {
      const palette = currentPalette(effective, $osDark);
      text = put(text, "theme", "custom");
      text = put(text, "theme.custom_mode", palette.mode);
      for (const [key, color] of Object.entries(palette.colors))
        text = put(text, `color.${key}`, JSON.stringify(color));
    }
    let literal = String(value);
    if (["font", "color", "shortcut"].includes(spec.kind))
      literal = JSON.stringify(value);
    text = put(text, spec.key, literal);
    changed();
  }
  function controlValue(spec: Spec) {
    const literal = assigned[spec.key];
    if (literal !== undefined) {
      try {
        return JSON.parse(literal);
      } catch {
        return literal;
      }
    }
    return effective[spec.key]?.value ?? spec.default;
  }
  function chooseTheme(theme: string) {
    text = put(text, "theme", theme);
    changed();
  }
  function customize() {
    const p = currentPalette(effective, $osDark);
    text = put(text, "theme", "custom");
    text = put(text, "theme.custom_mode", p.mode);
    for (const [key, color] of Object.entries(p.colors))
      text = put(text, `color.${key}`, JSON.stringify(color));
    category = "Custom colors";
    changed();
  }
  export async function apply() {
    clearTimeout(timer);
    if (!(await validate())) return;
    busy = true;
    try {
      const result = await bridge.updateSettings(documentId, scope, text);
      diagnostics = result.diagnostics;
      if (!diagnostics.length) {
        onpreview(null);
        await oncommit();
        dialog.close();
        capturedFocus?.focus();
      }
    } catch (e) {
      error = bridge.errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function repairReset() {
    if (!resetAll) return;
    busy = true;
    try {
      await bridge.resetSettings();
      onpreview(null);
      resetAll = false;
      await oncommit();
      await loadScope();
    } catch (e) {
      error = bridge.errorMessage(e);
    } finally {
      busy = false;
    }
  }
  async function imported() {
    const request = ++version;
    clearTimeout(timer);
    try {
      const imported = await bridge.importSettings();
      if (request !== version || !dialog.open) return;
      if (imported !== null) {
        text = imported;
        category = "Advanced";
        changed();
      }
    } catch (e) {
      if (request === version && dialog.open) error = bridge.errorMessage(e);
    }
  }
  async function exported() {
    try {
      if (await validate()) await bridge.exportSettings(text);
    } catch (e) {
      error = bridge.errorMessage(e);
    }
  }
  function capture(event: KeyboardEvent, spec: Spec) {
    if (event.key === "Tab" || event.key === "Escape") return;
    if (event.isComposing) return;
    event.preventDefault();
    event.stopPropagation();
    const c = chord(event);
    if (
      c &&
      !["Meta", "Ctrl", "Alt", "Shift", "Control"].includes(event.key) &&
      event.key !== "Dead"
    )
      set(
        spec,
        c.replace(
          navigator.platform.startsWith("Mac") ? "Meta+" : "Ctrl+",
          "Mod+",
        ),
      );
  }
</script>

<dialog
  bind:this={dialog}
  class="preferences-dialog"
  aria-label="Preferences"
  oncancel={(event) => {
    event.preventDefault();
    cancel();
  }}
>
  <header class="preferences-heading">
    <div>
      <h2 id="settings-title">Preferences</h2>
      <p>Make room for the way you read.</p>
    </div>
    <button onclick={cancel} aria-label="Close preferences">×</button>
  </header>
  <div class="preferences-top">
    <input
      aria-label="Search preferences"
      placeholder="Search preferences…"
      bind:value={query}
    /><label
      >Scope<select
        aria-label="Settings scope"
        value={scope}
        onchange={(event) => {
          scope = event.currentTarget.value;
          void loadScope();
        }}
        disabled={busy}
        ><option value="global">Global</option><option
          value="workspace"
          disabled={!documentId}>Workspace (app-managed)</option
        ><option value="file" disabled={!documentId}>File (app-managed)</option
        ><option value="session">Session only</option></select
      ></label
    >
  </div>
  <p class="preference-hint">
    Global → granted workspace root (or file parent) → file → session. Only
    explicit overrides are saved; document Markdown is untouched.
  </p>
  <div class="preferences-body">
    <nav aria-label="Preference categories">
      {#each categories as name (name)}<button
          aria-current={!query && category === name ? "page" : undefined}
          onclick={() => {
            category = name;
            query = "";
          }}>{name}</button
        >{/each}
    </nav>
    <fieldset
      class="preference-fields"
      aria-label="Preference controls"
      disabled={busy}
    >
      {#if busy}<p role="status">Loading preferences…</p>{/if}
      {#if category === "Appearance" && !query}<div class="theme-choices">
          {#each Object.entries(themes) as [key, theme] (key)}<button
              style={`background:${theme.colors.paper};color:${theme.colors.ink};border-color:${theme.colors.line}`}
              aria-pressed={effective.theme?.value === key}
              onclick={() => chooseTheme(key)}
              ><span style={`color:${theme.colors.accent}`}>Aa</span
              >{theme.label}<small>{theme.mode}</small></button
            >{/each}
        </div>
        <button onclick={customize}>Customize current theme</button>{/if}
      {#if category === "Advanced" && !query}<label class="settings-editor"
          >Settings text<textarea
            aria-label="Settings text"
            bind:value={text}
            oninput={changed}
            rows="12"
            spellcheck="false"
            disabled={busy}
          ></textarea></label
        >
        <p class="preference-hint">
          Literal $key=value only. No interpolation, CSS, file grants,
          executable paths or trust keys. Applying invalid input keeps the
          previous settings.
        </p>
        <label
          ><input type="checkbox" bind:checked={resetAll} />Reset all saved
          scopes and session preferences; keep document positions and backup the
          previous record.</label
        >
        <button onclick={repairReset} disabled={!resetAll || busy}
          >Reset all preferences</button
        >
      {:else}
        {#if category === "Shortcuts" && !query}<p class="preference-hint">
            Focus a binding and press a chord, or type a quoted chord in
            Advanced. Set “none” to disable. OS/menu and editor clipboard/undo
            bindings are reserved. Shortcuts are global or session-only.
          </p>{/if}
        {#if !shown.length}<p>No matching preference.</p>{/if}
        {#each shown as spec (spec.key)}<div class="preference-row">
            <label for={`pref-${spec.key}`}
              ><strong>{spec.label}</strong><small
                >{assigned[spec.key] !== undefined
                  ? "Override"
                  : `Inherited · ${spec.key}`}</small
              ><small
                >Effective: {String(
                  effective[spec.key]?.value ?? spec.default,
                )}</small
              ></label
            >
            <div>
              {#if spec.kind === "boolean"}<input
                  id={`pref-${spec.key}`}
                  type="checkbox"
                  checked={Boolean(controlValue(spec))}
                  onchange={(event) => set(spec, event.currentTarget.checked)}
                />
              {:else if spec.kind === "enum"}<select
                  id={`pref-${spec.key}`}
                  value={String(controlValue(spec))}
                  onchange={(event) => set(spec, event.currentTarget.value)}
                  >{#each spec.options as option (option)}<option value={option}
                      >{option}</option
                    >{/each}</select
                >
              {:else if spec.kind === "number"}<input
                  id={`pref-${spec.key}`}
                  type="number"
                  min={spec.min ?? undefined}
                  max={spec.max ?? undefined}
                  value={String(controlValue(spec))}
                  oninput={(event) => set(spec, event.currentTarget.value)}
                />
              {:else if spec.kind === "font"}<input
                  id={`pref-${spec.key}`}
                  list="installed-fonts"
                  maxlength="128"
                  value={String(controlValue(spec))}
                  oninput={(event) => set(spec, event.currentTarget.value)}
                />
              {:else if spec.kind === "color"}<input
                  id={`pref-${spec.key}`}
                  type="color"
                  value={String(controlValue(spec))}
                  oninput={(event) => set(spec, event.currentTarget.value)}
                /><span>{String(controlValue(spec))}</span>
              {:else}<input
                  id={`pref-${spec.key}`}
                  aria-label={`Shortcut ${spec.label}`}
                  value={String(controlValue(spec))}
                  onkeydown={(event) => capture(event, spec)}
                  readonly
                  disabled={["workspace", "file"].includes(scope)}
                /><button
                  onclick={() => set(spec, "none")}
                  disabled={["workspace", "file"].includes(scope)}>None</button
                >{/if}
              <button
                aria-label={`Inherit ${spec.label}`}
                disabled={assigned[spec.key] === undefined || busy}
                onclick={() => {
                  text = put(text, spec.key, null);
                  changed();
                }}>Inherit</button
              >
            </div>
          </div>{/each}
      {/if}
    </fieldset>
  </div>
  <datalist id="installed-fonts"
    >{#each families as family (family)}<option value={family}
      ></option>{/each}</datalist
  >
  <aside
    class="preferences-specimen"
    style={appearance(effective, $osDark)}
    aria-label="Live theme specimen"
  >
    <span class="specimen-title">A sentence worth keeping.</span><span
      >العربية · עברית · 漢字 · 🦀 <a
        href="#specimen"
        onclick={(e) => e.preventDefault()}>A local link</a
      ></span
    ><code
      ><span style="color:var(--keyword)">const</span> note =
      <span style="color:var(--string)">"words"</span>
      + <span style="color:var(--number)">42</span>;</code
    ><span
      ><mark style="color:var(--added);background:var(--added-bg)"
        >Added text specimen</mark
      >
      <mark style="color:var(--removed);background:var(--removed-bg)"
        >Removed text specimen</mark
      ></span
    >
  </aside>
  {#if error}<p role="alert">{error}</p>{/if}{#if diagnostics.length}<ul
      role="alert"
    >
      {#each diagnostics as d, index (index)}<li>
          Line {d.line}: {d.message}
        </li>{/each}
    </ul>{/if}
  <footer class="dialog-actions">
    <button onclick={imported} disabled={busy}>Import</button><button
      onclick={exported}
      disabled={busy}>Export sidecar</button
    ><button
      onclick={() => {
        text = "";
        changed();
      }}
      disabled={busy}>Reset scope to defaults</button
    ><button onclick={cancel}>Cancel</button><button
      class="primary"
      onclick={apply}
      disabled={busy}>Apply settings</button
    >
  </footer>
</dialog>
