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
  import { fontChoices } from "./fonts/catalogue";
  import { riwaqStatus, riwaqTypography } from "./fonts/presets";
  import { shaping } from "./fonts/arabic-compatibility";
  import { resolveTypography, sameAsBody } from "./fonts/typography";
  import { chord } from "./commands";
  import Updates from "./Updates.svelte";
  let {
    onpreview,
    oncommit,
    onrestart,
  }: {
    onpreview: (v: Values | null) => void;
    oncommit: () => Promise<void>;
    onrestart: () => Promise<boolean>;
  } = $props();
  let dialog: HTMLDialogElement;
  let body: HTMLDivElement;
  let open = $state(false);
  let specs = $state<Spec[]>([]),
    families = $state<string[]>([]),
    documentId = $state<string | null>(null),
    scope = $state("global"),
    category = $state("Appearance"),
    query = $state(""),
    text = $state(""),
    loadedText = $state(""),
    previewDocument = $state(true),
    validating = $state(false),
    effective = $state<Values>({}),
    diagnostics = $state<{ line: number; message: string }[]>([]),
    error = $state(""),
    busy = $state(false),
    committing = $state(false),
    resetAll = $state(false);
  let version = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let capturedFocus: HTMLElement | null = null;
  const categories = [
    "Appearance",
    "Workspace",
    "Reading",
    "Source",
    "Custom colors",
    "Shortcuts",
    "Advanced",
    "About",
  ];
  const about = $derived(category === "About" && !query);
  const assigned = $derived(literals(text));
  const dirty = $derived(text !== loadedText);
  const availableFonts = $derived(fontChoices(families));
  const presetStatus = $derived(
    validating || diagnostics.length || error
      ? ""
      : riwaqStatus(assigned, effective),
  );
  let discoveredFonts: Promise<void> | undefined;
  function discoverFonts() {
    discoveredFonts ??= fonts().then(
      (installed) => {
        families = installed;
      },
      () => {
        discoveredFonts = undefined;
      },
    );
  }
  const scopeLabels: Record<string, string> = {
    global: "This app",
    workspace: "This workspace",
    file: "This file",
    session: "Only this session",
  };
  const scopeHints: Record<string, string> = {
    global:
      "Defaults for every document. Workspace, file and session settings can override these choices.",
    workspace:
      "Applies to this selected folder and its documents. Your Markdown files stay unchanged.",
    file: "Applies only to this document. Your Markdown file stays unchanged.",
    session:
      "Temporary overrides until you quit Marktub. Your saved defaults stay unchanged.",
  };
  const help: Record<string, string> = $derived({
    "ui.font": "Used for tabs, menus and controls throughout the app.",
    "ui.scale": "Make the interface more compact or easier to read.",
    "library.width":
      "Drag the library divider to resize it, or enter a width here. Global or session-only.",
    "reader.font":
      $shaping === "webkit-compatibility"
        ? "This WebView uses Amiri for Arabic text to improve diacritic placement."
        : "Noto Naskh Arabic, Noto Sans Arabic and Amiri are bundled.",
    "reader.heading_font":
      "Applies to Arabic headings in the editor too. Same as body uses the reading font.",
    "reader.font_size":
      "Text size in Read, the Split preview, and Arabic prose while editing.",
    "reader.width":
      "Maximum text width; narrower columns keep long lines comfortable.",
    "reader.line_height": "Space between lines of document text.",
    "editor.font":
      "Arabic prose follows the Reading font, size, and line height.",
    "editor.font_size": "Text size for ordinary source lines and code.",
    "editor.line_height": "Line spacing for ordinary source lines and code.",
    "editor.tab_width":
      "The visual width of a tab character. Existing file bytes are preserved.",
    "editor.wrap":
      "Fit long lines within the editor instead of scrolling sideways.",
    "editor.line_numbers":
      "Start with line numbers visible. You can toggle them in the editor.",
    "editor.spellcheck":
      "Start with the system spellchecker enabled. Available languages depend on your system.",
    "code.style":
      "Choose colored syntax, a quiet code panel, or plain text colors.",
    "recovery.flush_ms":
      "How long edits can batch before recovery is flushed. This is not automatic file saving.",
  });
  const optionLabel = (spec: Spec, option: string) =>
    spec.key === "theme"
      ? ({
          system: "Follow system",
          light: "Light",
          dark: "Dark",
          paper: "Parchment",
          midnight: "Ink",
          riwaq_day: "Riwaq Day",
          riwaq_night: "Riwaq Night",
          custom: "Custom",
        }[option] ?? option)
      : option;
  function browse(name: string) {
    category = name;
    query = "";
    void tick().then(() => {
      if (body) body.scrollTop = 0;
    });
  }
  function revert() {
    text = loadedText;
    changed();
  }
  function resetCategory() {
    for (const spec of specs.filter((s) => s.category === category))
      text = put(text, spec.key, null);
    changed();
  }

  let shown = $derived(
    specs.filter(
      (s) =>
        s.key !== "schema.version" &&
        (s.key !== "ui.font" || scope === "global" || scope === "session") &&
        (query ||
          category !== "Appearance" ||
          (s.key !== "theme" &&
            (s.key !== "theme.custom_mode" ||
              effective.theme?.value === "custom"))) &&
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
    validating = false;
    try {
      const result = await bridge.getSettings(documentId, scope);
      if (request !== version) return;
      text = result.text;
      loadedText = result.text;
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
  export async function show(id: string | null, section = "Appearance") {
    if (dialog.open) {
      browse(section);
      return;
    }
    // Reserve before the first await: cold catalogue/font discovery must not
    // leave an editable draft that a later scope load can overwrite.
    busy = true;
    const opening = ++version;
    clearTimeout(timer);
    documentId = id;
    scope = "global";
    category = section;
    query = "";
    resetAll = false;
    previewDocument = true;
    error = "";
    capturedFocus = document.activeElement as HTMLElement | null;
    dialog.showModal();
    open = true;
    discoverFonts();
    try {
      if (!specs.length) specs = await catalogue();
      if (opening !== version || !dialog.open) return;
      const loaded = await loadScope();
      await tick();
      if (loaded === version && dialog.open) {
        body.scrollTop = 0;
        dialog
          .querySelector<HTMLElement>(
            about
              ? 'nav [aria-current="page"]'
              : '[aria-label="Search preferences"]',
          )
          ?.focus();
      }
    } catch (e) {
      if (opening === version) {
        error = bridge.errorMessage(e);
        busy = false;
      }
    }
  }
  function cancel() {
    if (committing) return;
    ++version;
    busy = false;
    clearTimeout(timer);
    onpreview(null);
    open = false;
    dialog.close();
    capturedFocus?.focus();
  }
  async function restartUpdate() {
    const action = document.activeElement;
    dialog.close();
    try {
      if (await onrestart()) {
        open = false;
        return;
      }
    } finally {
      if (open) {
        dialog.showModal();
        if (action instanceof HTMLElement) action.focus();
      }
    }
  }
  function changed() {
    const request = ++version;
    clearTimeout(timer);
    validating = true;
    timer = setTimeout(() => {
      void validate(request);
    }, 160);
  }
  async function validate(request = ++version) {
    validating = true;
    try {
      const result = await preview(documentId, scope, text);
      if (request !== version) return false;
      diagnostics = result.diagnostics;
      error = "";
      if (!diagnostics.length) {
        effective = result.resolved;
        if (previewDocument) onpreview(effective);
        return true;
      }
      return false;
    } catch (e) {
      if (request === version) error = bridge.errorMessage(e);
      return false;
    } finally {
      if (request === version) validating = false;
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
  function chooseTypography() {
    text = riwaqTypography(text, families);
    clearTimeout(timer);
    void validate();
  }
  function customize() {
    const p = currentPalette(effective, $osDark);
    text = put(text, "theme", "custom");
    text = put(text, "theme.custom_mode", p.mode);
    for (const [key, color] of Object.entries(p.colors))
      text = put(text, `color.${key}`, JSON.stringify(color));
    browse("Custom colors");
    changed();
  }
  export async function apply() {
    if (busy) return;
    clearTimeout(timer);
    if (!(await validate())) return;
    busy = true;
    committing = true;
    let applied = false;
    try {
      const result = await bridge.updateSettings(documentId, scope, text);
      diagnostics = result.diagnostics;
      if (!diagnostics.length) {
        applied = true;
        loadedText = text;
        onpreview(null);
        await oncommit();
        open = false;
        dialog.close();
        capturedFocus?.focus();
      }
    } catch (e) {
      error = applied
        ? `Settings were saved, but could not be refreshed: ${bridge.errorMessage(e)}`
        : bridge.errorMessage(e);
    } finally {
      busy = false;
      committing = false;
    }
  }
  async function repairReset() {
    if (!resetAll) return;
    busy = true;
    committing = true;
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
      committing = false;
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
  aria-busy={busy || validating}
  oncancel={(event) => {
    event.preventDefault();
    cancel();
  }}
>
  <header class="preferences-heading">
    <div>
      <h2 id="settings-title">Preferences</h2>
      <p>Reading, writing and a workspace that fits you.</p>
    </div>
    <button
      onclick={cancel}
      disabled={committing}
      aria-label="Close preferences">×</button
    >
  </header>
  <div class="preferences-top">
    <input
      aria-label="Search preferences"
      placeholder="Search preferences…"
      bind:value={query}
    />
    {#if !about}<label
        >Save settings for<select
          aria-label="Settings scope"
          value={scope}
          onchange={(event) => {
            scope = event.currentTarget.value;
            void loadScope();
          }}
          disabled={busy || dirty}
          ><option value="global">This app</option><option
            value="workspace"
            disabled={!documentId}>This workspace</option
          ><option value="file" disabled={!documentId}>This file</option><option
            value="session">Only this session</option
          ></select
        ></label
      >{/if}
  </div>
  {#if !about}
    <p class="preference-hint">
      {scopeHints[scope]}
    </p>
    {#if dirty}<p class="preference-hint" role="status">
        Apply or revert your changes before switching where settings are saved.
      </p>{/if}
    <div class="settings-preview-controls">
      <label
        ><input
          type="checkbox"
          bind:checked={previewDocument}
          disabled={busy}
          onchange={() =>
            onpreview(
              previewDocument && !diagnostics.length ? effective : null,
            )}
        />Preview changes in document</label
      >
      <span role="status"
        >{validating
          ? "Checking changes…"
          : diagnostics.length || error
            ? "Fix errors before applying"
            : dirty
              ? "Preview only · not saved yet"
              : "No unapplied changes"}</span
      >
    </div>
  {/if}
  <div class="preferences-body" bind:this={body}>
    <nav aria-label="Preference categories">
      {#each categories as name (name)}<button
          aria-current={!query && category === name ? "page" : undefined}
          onclick={() => browse(name)}>{name}</button
        >{/each}
    </nav>
    <Updates
      active={open && about}
      restartDisabled={dirty || busy || committing}
      onrestart={restartUpdate}
    />
    <fieldset
      class="preference-fields"
      aria-label="Preference controls"
      hidden={about}
      disabled={busy}
    >
      {#if busy}<p role="status">Loading preferences…</p>{/if}
      {#if category === "Appearance" && !query}
        <h3 class="settings-section-title">Color scheme</h3>
        <div class="theme-choices" role="group" aria-label="Color scheme">
          <button
            aria-pressed={effective.theme?.value === "system"}
            onclick={() => chooseTheme("system")}
            ><span>Aa</span>Follow system<small
              >Light or dark automatically</small
            ></button
          >
          {#each Object.entries(themes) as [key, theme] (key)}<button
              style={`background:${theme.colors.paper};color:${theme.colors.ink};border-color:${theme.colors.line}`}
              aria-label={`Use ${theme.label} theme`}
              aria-pressed={effective.theme?.value === key}
              onclick={() => chooseTheme(key)}
              ><span style={`color:${theme.colors.accent}`}>Aa</span
              >{theme.label}<small>{theme.mode}</small></button
            >{/each}
          <button
            aria-pressed={effective.theme?.value === "custom"}
            onclick={customize}
            ><span>Aa</span>Custom<small>Choose your own colors</small></button
          >
        </div>
        <div class="accent-control">
          <label for="quick-accent">Accent color</label>
          <input
            id="quick-accent"
            aria-label="Accent color"
            type="color"
            value={currentPalette(effective, $osDark).colors.accent}
            oninput={(event) => {
              const spec = specs.find((s) => s.key === "color.accent");
              if (spec) set(spec, event.currentTarget.value);
            }}
          />
          <button onclick={customize}>All theme colors</button>
        </div>
      {/if}
      {#if (category === "Appearance" || category === "Reading") && !query}
        <div class="riwaq-typography">
          <button onclick={chooseTypography}>Use Riwaq typography</button>
          <p class="preference-hint">
            Sets the reading and heading fonts, Cousine for source and code, and
            reading line spacing.
          </p>
          {#if presetStatus}<p class="preference-hint" role="status">
              {presetStatus}
            </p>{/if}
        </div>
      {/if}
      {#if category === "Workspace" && !query}<p class="preference-hint">
          Resize the library by dragging its divider. Tab actions also let you
          move tabs with the keyboard.
        </p>{/if}
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
        <div class="settings-transfer">
          <button onclick={imported} disabled={busy}>Import settings</button
          ><button onclick={exported} disabled={busy || validating}
            >Export settings</button
          ><button
            onclick={() => {
              text = "";
              changed();
            }}
            disabled={busy}>Reset this scope</button
          >
        </div>
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
        {#if !shown.length}<p>
            No matching preference. Try “font”, “width” or “shortcut”.
          </p>{/if}
        {#if !query}<div class="settings-section-heading">
            <h3>{category === "Appearance" ? "Interface" : category}</h3>
            <button
              onclick={resetCategory}
              disabled={!specs.some(
                (s) => s.category === category && assigned[s.key] !== undefined,
              )}>Reset {category.toLowerCase()}</button
            >
          </div>{/if}
        {#each shown as spec (spec.key)}<div class="preference-row">
            <label for={`pref-${spec.key}`}
              ><strong>{spec.label}</strong><small
                >{assigned[spec.key] !== undefined
                  ? `Set for ${scopeLabels[scope].toLowerCase()}`
                  : "Using inherited value"}</small
              ><small id={`help-${spec.key}`}
                >{help[spec.key] ??
                  (spec.kind === "color"
                    ? "Custom theme color, checked for readable contrast."
                    : "")}</small
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
                      >{optionLabel(spec, option)}</option
                    >{/each}</select
                >
              {:else if spec.kind === "number"}<input
                  type="range"
                  aria-label={`${spec.label} slider`}
                  min={spec.min ?? undefined}
                  max={spec.max ?? undefined}
                  step="1"
                  value={String(controlValue(spec))}
                  disabled={spec.key === "library.width" &&
                    ["workspace", "file"].includes(scope)}
                  oninput={(event) => set(spec, event.currentTarget.value)}
                /><input
                  id={`pref-${spec.key}`}
                  type="number"
                  aria-describedby={`help-${spec.key}`}
                  disabled={spec.key === "library.width" &&
                    ["workspace", "file"].includes(scope)}
                  min={spec.min ?? undefined}
                  max={spec.max ?? undefined}
                  value={String(controlValue(spec))}
                  oninput={(event) => set(spec, event.currentTarget.value)}
                />
              {:else if spec.kind === "font"}<input
                  id={`pref-${spec.key}`}
                  list={spec.key === "reader.heading_font"
                    ? "heading-fonts"
                    : "available-fonts"}
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
                }}>Reset</button
              >
            </div>
          </div>{/each}
      {/if}
    </fieldset>
  </div>
  {#if open}
    <datalist id="available-fonts">
      {#each availableFonts as family (family)}<option value={family}
        ></option>{/each}
    </datalist>
    <datalist id="heading-fonts">
      <option value={sameAsBody}></option>
      {#each availableFonts as family (family)}<option value={family}
        ></option>{/each}
    </datalist>
  {/if}
  {#if !about}<aside
      class="preferences-specimen"
      style={appearance(
        effective,
        $osDark,
        resolveTypography(effective, $shaping),
      )}
      aria-label="Live theme specimen"
    >
      <span class="specimen-title" dir="auto"
        >كلمات تستحق البقاء · A sentence worth keeping.</span
      ><span dir="auto" class="specimen-body"
        >العربية · <strong>نص عريض</strong> · עברית · 漢字 · 🦀
        <a href="#specimen" onclick={(e) => e.preventDefault()}>A local link</a
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
    </aside>{/if}
  {#if error}<p role="alert">{error}</p>{/if}{#if diagnostics.length}<ul
      role="alert"
    >
      {#each diagnostics as d, index (index)}<li>
          Line {d.line}: {d.message}
        </li>{/each}
    </ul>{/if}
  <footer class="dialog-actions">
    {#if about && !dirty}
      <button onclick={cancel} disabled={committing}>Close</button>
    {:else}
      <button onclick={revert} disabled={busy || !dirty}>Revert changes</button>
      <span class="settings-save-hint">{scopeLabels[scope]}</span>
      <button onclick={cancel} disabled={committing}>Cancel</button><button
        class="primary"
        onclick={apply}
        disabled={busy || validating || !!diagnostics.length || !dirty}
        >Apply settings</button
      >
    {/if}
  </footer>
</dialog>
