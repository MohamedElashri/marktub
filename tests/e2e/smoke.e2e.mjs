import { desktopBinary } from "../../scripts/desktop-paths.mjs";
import { browser, expect } from "@wdio/globals";
import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve, dirname, join } from "node:path";

const nativeWebView = process.platform === "darwin" ? "WKWebView" : "WebKitGTK";

function installedTool(role, command = role) {
  const configured = process.env[`MARKTUB_E2E_${role.toUpperCase()}`];
  if (configured) return resolve(configured);
  const found = spawnSync("which", [command], { encoding: "utf8" });
  if (found.status !== 0 || !found.stdout.trim())
    throw new Error(
      `Install ${command} or set MARKTUB_E2E_${role.toUpperCase()} to its native executable before running the export suite.`,
    );
  return found.stdout.trim();
}

// Mocha's root hook runs after the native service attaches browser.tauri. The
// config-level before hook runs too early in the pinned WDIO service.
before(async () => {
  await browser.tauri.switchWindow("main");
  expect(await browser.tauri.listWindows()).toEqual(["main"]);
  await browser.tauri.execute(() => {
    window.marktubTestErrors = [];
    const record = (error) => {
      window.marktubTestErrors.push(String(error).slice(0, 2000));
      window.marktubTestErrors = window.marktubTestErrors.slice(-20);
    };
    addEventListener("error", (event) => record(event.error ?? event.message));
    addEventListener("unhandledrejection", (event) => record(event.reason));
  });
});

afterEach(async function () {
  if (this.currentTest?.state !== "failed") return;
  const prefix = `artifacts/native-failure-${Date.now()}`;
  await mkdir("artifacts", { recursive: true });
  const state = await browser.tauri.execute(() => ({
    text: document.body.innerText.slice(0, 8000),
    errors: window.marktubTestErrors,
    userAgent: navigator.userAgent,
    tabs: [...document.querySelectorAll(".document-tab")].map((tab) => ({
      hidden: tab.hidden,
      mode: tab.querySelector('[aria-pressed="true"]')?.textContent,
      busy: tab.querySelector(".window-shell")?.getAttribute("data-busy"),
      editors: tab.querySelectorAll(".cm-editor").length,
    })),
    closeControls: [
      ...document.querySelectorAll('.tabbar button[aria-label^="Close "]'),
    ].map((button) => ({
      name: button.getAttribute("aria-label"),
      disabled: button.disabled,
    })),
  }));
  await writeFile(`${prefix}.json`, JSON.stringify(state, null, 2));
  await browser.saveScreenshot(`${prefix}.png`);
  console.error("NATIVE FAILURE SNAPSHOT", prefix, state.errors);
});

beforeEach(async () => {
  await browser.tauri.execute(({ core }) =>
    core.invoke("plugin:window|set_focus", { label: "main" }),
  );
  try {
    await browser.waitUntil(
      () =>
        browser.tauri.execute(async ({ core }) => {
          await core.invoke("plugin:window|set_focus", { label: "main" });
          return core.invoke("plugin:window|is_focused", { label: "main" });
        }),
      {
        timeout: 5000,
        timeoutMsg: "Native main window must have keyboard focus",
      },
    );
  } catch (error) {
    const state = await browser.tauri.execute(async ({ core }) => ({
      visibility: document.visibilityState,
      documentFocused: document.hasFocus(),
      focused: await core.invoke("plugin:window|is_focused", { label: "main" }),
      activeElement: document.activeElement?.tagName,
      text: document.body.innerText.slice(0, 2000),
    }));
    await mkdir("artifacts", { recursive: true });
    const prefix = `artifacts/native-focus-failure-${Date.now()}`;
    await writeFile(`${prefix}.json`, JSON.stringify(state, null, 2));
    await browser.saveScreenshot(`${prefix}.png`);
    console.error("NATIVE FOCUS FAILURE", state);
    throw error;
  }
});

async function documentAction(label) {
  const pane = browser.$(".document-tab:not([hidden])");
  const disclosure = pane.$(".document-actions");
  if (!(await disclosure.getProperty("open"))) {
    await pane.$(".document-actions summary").click();
  }
  const button = disclosure.$(`button=${label}`);
  await button.waitForEnabled({ timeout: 60000 });
  await button.click();
  if (label === "Settings")
    await browser.waitUntil(
      async () =>
        !(await browser
          .$('dialog[aria-label="Preferences"] fieldset')
          .getProperty("disabled")),
      { timeout: 60000 },
    );
}

describe("OS file open and accessibility", () => {
  async function closeDocuments() {
    for (let count = 0; count < 17; count++) {
      const tabs = await browser.$$('.tabs button[aria-label^="Close "]');
      if (!tabs.length) break;
      const countBefore = tabs.length;
      await tabs[tabs.length - 1].waitForEnabled({ timeout: 60000 });
      await tabs[tabs.length - 1].click();
      await browser.waitUntil(
        async () =>
          (await browser.$$('.tabs button[aria-label^="Close "]')).length <
            countBefore ||
          (await browser.$("button=Read the example").isDisplayed()) ||
          (await browser
            .$('dialog[open] [id$="-discard-title"]')
            .isDisplayed()),
      );
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      await browser.waitUntil(
        async () =>
          (await browser.$$('.tabs button[aria-label^="Close "]')).length <
            countBefore ||
          (await browser.$("button=Read the example").isDisplayed()),
      );
      if (await browser.$("button=Read the example").isDisplayed()) break;
    }
  }
  const invoke = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  beforeEach(closeDocuments);
  after(closeDocuments);

  it("hands native Unicode/space paths to the existing instance without replacing dirty Source", async () => {
    const fs = await import("node:fs/promises");
    const seed = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Native source\r\n",
    });
    const first = join(dirname(seed.path), "native العربية 🦀 one.md");
    const second = join(dirname(seed.path), "second 文.md");
    await fs.writeFile(first, "# Native source\r\n");
    await fs.writeFile(second, "# Second handoff\n");
    const binary = desktopBinary("debug");
    function handoff(path) {
      const run = spawnSync(binary, [path], {
        encoding: "utf8",
        timeout: 15000,
      });
      expect(run.status).toBe(0);
    }
    handoff(first);
    await expect(browser.$("article h1")).toHaveText("Native source");
    await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Source").click();
    await browser.$(".cm-content").waitForDisplayed();
    await browser.tauri.execute(() => {
      const content = document.querySelector(".cm-content");
      content.focus();
      document.execCommand("insertText", false, "unsaved ");
      window.handoffEditor = document.querySelector(".cm-editor");
    });
    await browser.waitUntil(async () =>
      (
        await browser.$('.tabs [role="tab"][aria-selected="true"]').getText()
      ).includes("*"),
    );
    const text = await browser.$(".cm-content").getText();
    handoff(second);
    await expect(
      browser.$(".document-tab:not([hidden]) article h1"),
    ).toHaveText("Second handoff");
    handoff(first);
    await expect(
      browser.$(".document-tab:not([hidden]) .cm-content"),
    ).toHaveText(text);
    expect(
      await browser.tauri.execute(
        () =>
          window.handoffEditor ===
          document.querySelector(".document-tab:not([hidden]) .cm-editor"),
      ),
    ).toBe(true);
    expect(await fs.readFile(first, "utf8")).toBe("# Native source\r\n");
    if (process.platform === "darwin") {
      handoff(resolve(dirname(binary), "../Resources/guide/Marktub Guide.md"));
      await browser.waitUntil(async () =>
        (await browser.$(".workbench > .notice").getText()).includes(
          "App-owned",
        ),
      );
      await expect(
        browser.$(".document-tab:not([hidden]) .cm-content"),
      ).toHaveText(text);
    }
    expect(await invoke("take_os_open", { path: "/etc/passwd" })).toEqual([]);
    await browser.tauri.execute(() => {
      delete window.handoffEditor;
    });
    await writeFile(
      "artifacts/native-native-open.json",
      JSON.stringify({
        unicodeAndSpaces: true,
        existingInstance: true,
        dirtyStateAndViewPreserved: true,
        diskUnchanged: true,
      }),
    );
  });

  it("finalizes boundary-spanning reference titles and collision-free heading/footnote anchors", async () => {
    const prefix =
      "# Same\n\n# Same\n\n# Same-1\n\n# Note-61\n\nRead [later][r] and note[^a].\n\n";
    const definition = '[r]: target.md\n  "continued title"\n';
    const boundary = 65536 - definition.indexOf("continued") - 4;
    const padding = "padding paragraph\n\n";
    const count = Math.floor((boundary - prefix.length - 2) / padding.length);
    const content =
      prefix +
      padding.repeat(count) +
      "x".repeat(boundary - prefix.length - count * padding.length - 2) +
      "\n\n" +
      definition +
      "\n[^a]: final note\n";
    const fixture = (action, text) =>
      invoke("e2e_fixture", { action, text: text ?? null });
    await fixture("seed", content);
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 20000 });
    await browser.$("button=Outline & find").click();
    await browser.$(".outline nav").$("button=Note-61").click();
    await expect(browser.$("article #h-note-61-1")).toHaveText("Note-61");
    await expect(browser.$('article a[title="continued title"]')).toExist();
    await browser.$("article a.footnote-ref").click();
    await expect(browser.$("article #h-note-61")).toHaveText("Footnote a");
    await fixture("export");
    await browser.$("button=Export HTML").click();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 40000 });
    const { readFile } = await import("node:fs/promises");
    const output = await fixture("read-export");
    const html = await readFile(output.path, "utf8");
    const ids = [...html.matchAll(/\bid="(h-[^"]+)"/g)].map(
      (match) => match[1],
    );
    expect(ids).toEqual([
      "h-same",
      "h-same-1",
      "h-same-1-1",
      "h-note-61-1",
      "h-note-61",
    ]);
    expect(html).toContain('title="continued title"');
    expect(html).toContain('href="#h-note-61"');
    expect(await readFile((await fixture("read")).path, "utf8")).toBe(content);
    await writeFile(
      "artifacts/native-parity-native.json",
      JSON.stringify({
        headingIds: ids,
        continuationTitle: true,
        lateFootnoteNavigation: true,
        finalizedStandaloneExport: true,
        originalBytesUnchanged: true,
      }),
    );
  });

  it("releases rich reader/editor DOM across twenty open and close cycles", async () => {
    const content =
      "# Lifecycle العربية 日本語\n\n$$\n\\sqrt{x}\n$$\n\n$$\n\\prescript{a}{b}{X}\n$$\n\n```mermaid\ngraph LR\nRead-->Review\n```\n";
    const samples = [];
    for (let cycle = 0; cycle < 20; cycle++) {
      const seed = await invoke("e2e_fixture", {
        action: "seed",
        text: content,
      });
      await browser.$("button.open-button").waitForEnabled();
      await browser.$("button.open-button").click();
      await expect(browser.$("article h1")).toHaveText(
        "Lifecycle العربية 日本語",
      );
      if (cycle % 2 === 0) {
        await browser.waitUntil(() =>
          browser.tauri.execute(
            () => document.querySelectorAll("[data-rich-kind]").length === 3,
          ),
        );
        const frames = await browser.tauri.execute(
          () =>
            new Promise((resolve) => {
              const times = [];
              let last = performance.now();
              let frameId;
              const finish = (complete) => {
                clearTimeout(timer);
                cancelAnimationFrame(frameId);
                resolve({
                  framesMs: times.slice(1),
                  complete,
                  visibility: document.visibilityState,
                  documentFocused: document.hasFocus(),
                });
              };
              const timer = setTimeout(() => finish(false), 2000);
              function frame(now) {
                times.push(now - last);
                last = now;
                if (times.length < 20) frameId = requestAnimationFrame(frame);
                else finish(true);
              }
              frameId = requestAnimationFrame(frame);
            }),
        );
        await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
        await browser.$("button=Source").click();
        await browser.$(".cm-content").waitForDisplayed();
        await browser.$("button=Visual").waitForEnabled({ timeout: 60000 });
        await browser.$("button=Visual").click();
        samples.push({
          cycle,
          scheduling: frames,
          scope:
            "debug rAF scheduling; not physical input/compositor paint or production SLA",
        });
      }
      await closeDocuments();
      const nodes = await browser.tauri.execute(() => ({
        nodes: document.querySelectorAll("*").length,
        readers: document.querySelectorAll("article").length,
        editors: document.querySelectorAll(".cm-editor").length,
      }));
      expect(nodes.readers).toBe(0);
      expect(nodes.editors).toBe(0);
      expect(nodes.nodes).toBeLessThan(500);
      if ((cycle + 1) % 5 === 0) {
        const memory = spawnSync(
          "python3",
          [
            "benchmarks/source_memory.py",
            "--pid",
            String(seed.pid),
            "--label",
            `close-cycle-${cycle + 1}`,
          ],
          { encoding: "utf8" },
        );
        expect(memory.status).toBe(0);
      }
    }
    await mkdir("artifacts", { recursive: true });
    await writeFile(
      "artifacts/native-followup-cycles.json",
      JSON.stringify({ cycles: 20, retiredDomAbsent: true, samples }, null, 2),
    );
  });

  it("uses roving document tabs and modal palette focus with native WebKit keyboard events", async () => {
    await browser.$('button[aria-label="New tab"]').click();
    let selected = browser.$('.tabs [role="tab"][aria-selected="true"]');
    await selected.click();
    await browser.keys("ArrowLeft");
    expect(
      await browser.tauri.execute(() => ({
        selected: document.activeElement?.getAttribute("aria-selected"),
        tabs: [...document.querySelectorAll('.tabs [role="tab"]')].filter(
          (t) => t.tabIndex === 0,
        ).length,
        panel: !!document.getElementById(
          document.activeElement?.getAttribute("aria-controls") ?? "",
        ),
      })),
    ).toEqual({ selected: "true", tabs: 1, panel: true });
    await browser.keys("End");
    selected = browser.$('.tabs [role="tab"][aria-selected="true"]');
    await expect(selected).toBeFocused();
    expect(
      await browser.tauri.execute(() => {
        const labels = [
          ...document.querySelectorAll("dialog[aria-labelledby]"),
        ].map((d) => d.getAttribute("aria-labelledby"));
        return labels.every(
          (id) =>
            document.querySelectorAll(`[id="${CSS.escape(id)}"]`).length === 1,
        );
      }),
    ).toBe(true);
    await browser.$("button=Commands").click();
    const search = browser.$('input[aria-label="Search commands"]');
    await expect(search).toBeFocused();
    await search.setValue("open");
    await browser.keys("ArrowDown");
    expect(
      await browser.tauri.execute(() => {
        const input = document.querySelector('[aria-label="Search commands"]');
        const option = document.getElementById(
          input.getAttribute("aria-activedescendant"),
        );
        return {
          role: option?.getAttribute("role"),
          tabIndex: option?.tabIndex,
          inside: document
            .querySelector("dialog[open]")
            ?.contains(document.activeElement),
        };
      }),
    ).toEqual({ role: "option", tabIndex: -1, inside: true });
    await browser.keys("Escape");
    await expect(browser.$("button=Commands")).toBeFocused();
    await writeFile(
      "artifacts/native-keyboard.json",
      JSON.stringify({
        rovingTabs: true,
        panelAssociations: true,
        comboboxActiveDescendant: true,
        nativeDialogFocusRestored: true,
        realOsImeOrScreenReader: false,
      }),
    );
  });
});

describe("Native WebView bootstrap", () => {
  it("opens a real bundled file through Rust and renders it in the worker", async () => {
    await browser.$("button=Read the example").click();
    await expect(browser.$("article h1")).toHaveText(
      "A place for unfinished thoughts",
    );
    const metrics = await browser.tauri.execute(() => ({
      url: window.location.href,
      nodes: document.querySelectorAll("*").length,
      resourceUrls: performance
        .getEntriesByType("resource")
        .map((entry) => entry.name),
      parseMs: document.querySelector("article")?.getAttribute("data-parse-ms"),
      text: document.querySelector("article")?.textContent,
    }));
    expect(metrics.text).toContain("العربية");
    expect(metrics.resourceUrls.some((url) => /^https:\/\//.test(url))).toBe(
      false,
    );
    expect(metrics.nodes).toBeLessThan(500);
    await mkdir("artifacts", { recursive: true });
    await writeFile(
      "artifacts/webview-smoke.json",
      JSON.stringify(metrics, null, 2),
    );
  });

  it("loads CodeMirror on demand and keeps it mounted across mode changes", async () => {
    await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Source").click();
    await expect(browser.$(".cm-content")).toBeDisplayed();
    await browser.$('input[type="checkbox"]').click();
    await expect(browser.$(".cm-live-heading")).toExist();
    await browser.$("button=Read").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Read").click();
    await expect(browser.$("article h1")).toBeDisplayed();
    const count = await browser.tauri.execute(
      () => document.querySelectorAll(".cm-editor").length,
    );
    expect(count).toBe(1);
    await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Source").click();
    await expect(browser.$(".cm-content")).toBeDisplayed();
  });

  it("denies unknown grants, filesystem plugin reads and absent runners via real IPC", async () => {
    const result = await browser.tauri.execute(async ({ core }) => {
      async function failure(command, args) {
        try {
          await core.invoke(command, args);
          return "UNEXPECTED_SUCCESS";
        } catch (error) {
          return JSON.stringify(error);
        }
      }
      return {
        enabled: await core.invoke("execution_status"),
        path: await failure("read_range", {
          id: "/etc/passwd",
          start: 0,
          limit: 64,
        }),
        edit: await failure("begin_edit", { id: "/etc/passwd" }),
        image: await failure("choose_image", {
          id: "/etc/passwd",
          link: "x.png",
        }),
        link: await failure("open_document_link", {
          id: "/etc/passwd",
          link: "x.md",
        }),
        save: await failure("save_document", {
          id: "/etc/passwd",
          generation: "guessed",
          revision: 0,
        }),
        restore: await failure("restore_recovery", { id: "../escape" }),
        plugin: await failure("plugin:fs|read_text_file", {
          path: "/etc/passwd",
        }),
        shell: await failure("run_shell", { command: "echo must-not-run" }),
        sql: await failure("run_sql", { sql: "SELECT 1" }),
      };
    });
    expect(result.enabled).toBe(false);
    expect(result.path).toContain("unauthorized");
    for (const key of ["edit", "save", "restore", "image", "link"])
      expect(result[key]).toContain("unauthorized");
    for (const key of ["plugin", "shell", "sql"])
      expect(result[key]).not.toBe("UNEXPECTED_SUCCESS");
  });
});

describe("Source integrity", () => {
  const original = "\uFEFFa\r\nb\nc\r🦀";
  const fixture = (action, text) =>
    browser.tauri.execute(
      ({ core }, action, text) =>
        core.invoke("e2e_fixture", { action, text: text ?? null }),
      action,
      text,
    );
  async function seed() {
    await fixture("seed", original);
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    const discard = browser.$('dialog[open] [id$="-discard-title"]');
    if (await discard.isDisplayed())
      await browser.$("button=Discard edits").click();
    await browser.$(".cm-content").waitForExist({ reverse: true });
    await browser.$("button=Source").waitForEnabled();
    await browser.$("button=Source").click();
    await expect(browser.$(".cm-content")).toBeDisplayed();
  }
  it("saves localized bytes, undoes across saves and reopens through Rust", async () => {
    await seed();
    await browser.$(".cm-content").addValue("文");
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.waitUntil(
      async () => (await fixture("read")).text === original + "文",
    );
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser.$('button[aria-label="Undo"]').click();
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.waitUntil(
      async () => (await fixture("read")).text === original,
    );
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser.$("button=Redo").click();
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.waitUntil(
      async () => (await fixture("read")).text === original + "文",
    );
    await fixture("open");
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await browser.$(".cm-content").waitForExist({ reverse: true });
    await browser.$("button=Source").waitForEnabled();
    await browser.$("button=Source").click();
    await expect(browser.$(".cm-content")).toHaveText(
      expect.stringContaining("🦀文"),
    );
  });
  it("keeps N+1 dirty while save N completes", async () => {
    await seed();
    await browser.$(".cm-content").addValue("N");
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.$(".cm-content").addValue("X");
    await browser.waitUntil(
      async () => (await fixture("read")).text === original + "N",
    );
    await expect(
      browser.$(".document-tab:not([hidden]) .statusbar"),
    ).toHaveText(expect.stringContaining("Unsaved changes"));
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.waitUntil(
      async () => (await fixture("read")).text === original + "NX",
    );
  });
  it("refuses an external writer, compares snapshots and saves an independent copy", async () => {
    await seed();
    await browser.$(".cm-content").addValue("local");
    await fixture("external", "external\n");
    await browser.tauri.execute(() => window.dispatchEvent(new Event("focus")));
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .click();
    await browser.waitUntil(
      async () =>
        (await browser.$(".window-shell").getText()).includes(
          "disk revision changed",
        ) ||
        (await browser.$(".window-shell").getText()).includes("file changed"),
    );
    expect((await fixture("read")).text).toBe("external\n");
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save")
      .waitForEnabled();
    await browser.$("button=Compare").click();
    await expect(browser.$("dialog.compare-dialog")).toBeDisplayed();
    await expect(browser.$("dialog.compare-dialog")).toHaveText(
      expect.stringContaining("external"),
    );
    await browser.$("dialog.compare-dialog").$("button=Keep local").click();
    await fixture("save-as");
    await browser
      .$(".document-tab:not([hidden]) .toolbar")
      .$("button=Save As")
      .click();
    await browser.waitUntil(
      async () => (await fixture("read-copy")).text === original + "local",
    );
    expect((await fixture("read")).text).toBe("external\n");
  });
  it("restores a durable recovery and requires Save As authorization", async () => {
    await seed();
    await browser.$(".cm-content").addValue("recover");
    await browser.$("button=Read").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Read").click();
    await browser.tauri.execute(({ core }) =>
      core.invoke("e2e_fixture", { action: "seed", text: "# next\n" }),
    );
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await expect(
      browser.$('dialog[open] [id$="-discard-title"]'),
    ).toBeDisplayed();
    await browser.$("button=Keep recovery").click();
    await expect(browser.$(".recovery-list")).toBeDisplayed();
    await browser.$(".recovery-list summary").click();
    await browser.$(".recovery-list").$("button=Restore").click();
    await expect(browser.$(".cm-content")).toHaveText(
      expect.stringContaining("recover"),
    );
    await expect(browser.$(".source-toolbar")).toHaveText(
      expect.stringContaining("new destination"),
    );
    await fixture("open");
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await browser.$("button=Discard edits").click();
    if (await browser.$("button=Exit focus").isDisplayed())
      await browser.$("button=Exit focus").click();
    await documentAction("Settings");
    await expect(browser.$("#settings-title")).toBeDisplayed();
    await browser
      .$('dialog[aria-label="Preferences"] select[aria-label="Settings scope"]')
      .selectByAttribute("value", "session");
    await browser.$("button=Advanced").click();
    await browser.$('textarea[aria-label="Settings text"]').waitForEnabled();
    await browser
      .$('textarea[aria-label="Settings text"]')
      .setValue("$reader.width=900\n$security.execution=true");
    await expect(browser.$('textarea[aria-label="Settings text"]')).toHaveValue(
      "$reader.width=900\n$security.execution=true",
    );
    await expect(browser.$("button=Apply settings")).toBeDisabled();
    await expect(browser.$('dialog ul[role="alert"]')).toHaveText(
      expect.stringContaining("Line 2"),
    );
    await browser
      .$('textarea[aria-label="Settings text"]')
      .setValue("$theme=dark\n$reader.width=900");
    await browser.$("button=Apply settings").waitForEnabled();
    await browser.$("button=Apply settings").click();
    await browser
      .$('dialog[aria-label="Preferences"]')
      .waitForDisplayed({ reverse: true });
    await expect(browser.$(".window-shell")).toHaveAttribute(
      "data-theme",
      "dark",
    );
  });
});

describe("Source measurements", () => {
  it("measures 1 and 10 MiB in the actual debug WebView with bounded DOM", async () => {
    const fixture = (action, text) =>
      browser.tauri.execute(
        ({ core }, action, text) =>
          core.invoke("e2e_fixture", { action, text: text ?? null }),
        action,
        text,
      );
    const pid = (await fixture("read")).pid;
    const sample = (label) => {
      const result = spawnSync(
        "python3",
        ["benchmarks/source_memory.py", "--pid", String(pid), "--label", label],
        { encoding: "utf8" },
      );
      expect(result.status).toBe(0);
    };
    sample("read-warm");
    const results = [];
    for (const tier of [1, 10]) {
      await fixture(`seed-${tier}MiB`);
      const started = performance.now();
      await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
      await browser.$("button.open-button").click();
      await browser.$(".cm-content").waitForExist({ reverse: true });
      await browser.$("button=Source").waitForEnabled();
      const read = performance.now();
      await browser.$("button=Source").click();
      await expect(browser.$(".cm-content")).toBeDisplayed();
      const opened = performance.now();
      const metrics = await browser.tauri.execute(() => ({
        nodes: document.querySelectorAll("*").length,
        sourceLines: document.querySelectorAll(".cm-line").length,
      }));
      expect(metrics.nodes).toBeLessThan(1500);
      results.push({
        fixtureMiB: tier,
        readOpenMs: read - started,
        sourceOpenMs: opened - read,
        ...metrics,
      });
      sample(`source-${tier}MiB`);
    }
    await writeFile(
      "artifacts/source-source-open.json",
      JSON.stringify(
        {
          profile:
            "debug e2e, warm imports, one sample/tier, picker injected; includes IPC/initial UI but not an input-paint SLA",
          samples: results,
        },
        null,
        2,
      ),
    );
  });
});

describe("Native close guard", () => {
  it("cancels a real native close request without discarding pending text", async () => {
    await browser.tauri.execute(({ core }) =>
      core.invoke("e2e_fixture", { action: "seed", text: "# Close guard\n" }),
    );
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await browser.$(".cm-content").waitForExist({ reverse: true });
    await browser.$("button=Source").waitForEnabled();
    await browser.$("button=Source").click();
    await browser.$(".cm-content").addValue("keep-on-close");
    await expect(
      browser.$(".document-tab:not([hidden]) .statusbar"),
    ).toHaveText(expect.stringContaining("Unsaved changes"));
    await browser.tauri.execute(({ core }) =>
      core.invoke("e2e_fixture", { action: "request-close", text: null }),
    );
    await expect(
      browser.$('dialog[open] [id$="-discard-title"]'),
    ).toBeDisplayed();
    await browser.$("button=Keep editing").click();
    await expect(browser.$(".cm-content")).toHaveText(
      expect.stringContaining("keep-on-close"),
    );
    await expect(
      browser.$(".document-tab:not([hidden]) .statusbar"),
    ).toHaveText(expect.stringContaining("Unsaved changes"));
  });
});

describe("Reader and local renderers", () => {
  const fixture = (action, text) =>
    browser.tauri.execute(
      ({ core }, action, text) =>
        core.invoke("e2e_fixture", { action, text: text ?? null }),
      action,
      text,
    );
  const content =
    "# Reader integrity\n\nA forward [link][later] and footnote[^one].\n\n![Approved pixel](pixel.png){width=32 height=32}\n\nInline $a^2$ and compatibility $\\prescript{a}{b}{X}$.\n\n```mermaid\ngraph TD\nA[Read] --> B[Keep source]\n```\n\n" +
    ("A paragraph about local documents. ".repeat(20) + "\n\n").repeat(100) +
    "# Far away\n\nNeedleAtTheEnd\n\n[later]: chapter.md\n[^one]: A final footnote.\n";
  it("opens Read without CodeMirror, renders local math/diagrams, and navigates unmounted text", async () => {
    await fixture("seed", content);
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
      await browser.$("button=Discard edits").click();
    await expect(browser.$("article h1")).toHaveText("Reader integrity");
    await expect(browser.$(".cm-editor")).not.toExist();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 20000 });
    await fixture("image");
    await browser.$("button.image-request").click();
    await expect(browser.$("article img")).toExist();
    await browser.waitUntil(
      async () => (await browser.$$(".rich-placeholder svg").length) >= 2,
      { timeout: 30000, timeoutMsg: "MathJax and Mermaid local SVGs" },
    );
    await expect(browser.$(".katex")).toExist();
    const resources = await browser.tauri.execute(() =>
      performance.getEntriesByType("resource").map((r) => r.name),
    );
    expect(
      resources.some((r) => /^https:\/\/(?!tauri\.localhost)/.test(r)),
    ).toBe(false);
    const richMetrics = await browser.tauri.execute(() => ({
      observedHydration: Array.from(
        document.querySelectorAll("[data-render-ms]"),
      ).map((el) => ({
        kind: el.getAttribute("data-rich-kind"),
        durationMs: Number(el.getAttribute("data-render-ms")),
        cacheHit: el.getAttribute("data-cache-hit") === "true",
      })),
      resources: performance.getEntriesByType("resource").map((r) => ({
        name: r.name.split("/").at(-1),
        durationMs: r.duration,
        transferSize: r.transferSize,
      })),
    }));
    await writeFile(
      "artifacts/reader-rich.json",
      JSON.stringify(richMetrics, null, 2),
    );
    await browser.saveScreenshot("artifacts/reader-renderers.png");
    await browser.$("button=Outline & find").click();
    await browser.$('.outline input[type="search"]').setValue("NeedleAtTheEnd");
    await browser.$("button=Find").click();
    await browser.$(".find-result").waitForExist({ timeout: 20000 });
    await browser.$(".find-result").click();
    await expect(browser.$("article")).toHaveText(
      expect.stringContaining("NeedleAtTheEnd"),
    );
    await browser.saveScreenshot("artifacts/reader-reader.png");
  });
  it("exports the coherent full saved revision including offscreen math/diagrams and footnotes", async () => {
    await fixture("export");
    await browser.$("button=Export HTML").click();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 40000 });
    await expect(browser.$(".reader-notice")).toHaveText(
      expect.stringContaining("export completed"),
    );
    const info = await fixture("read-export");
    const { readFile } = await import("node:fs/promises");
    const html = await readFile(info.path, "utf8");
    expect(html).toContain("NeedleAtTheEnd");
    expect(html).toContain("A final footnote.");
    expect(html).toContain("<svg");
    expect(html).toContain("data:font/woff2;base64,");
    expect(html).toContain("data:image/png;base64,");
    expect(html).not.toMatch(/<script\b|onload=|onclick=|<iframe\b/);
    await writeFile("artifacts/reader-export.html", html);
    await writeFile(
      "artifacts/reader-export-meta.json",
      JSON.stringify(
        {
          bytes: Buffer.byteLength(html),
          sections: (html.match(/<section>/g) || []).length,
          hasSvg: html.includes("<svg"),
          hasInlineFonts: html.includes("data:font/woff2;base64,"),
        },
        null,
        2,
      ),
    );
  });
  it("restores the semantic heading after a clean external reload", async () => {
    await browser.$(".outline nav").$("button=Far away").click();
    const revision = await browser.$(".reader").getAttribute("data-revision");
    await fixture(
      "external",
      "# Inserted before the document\n\n" +
        ("An introduction. ".repeat(100) + "\n\n").repeat(20) +
        content,
    );
    await browser
      .waitUntil(
        async () =>
          (await browser.$(".reader").getAttribute("data-revision")) !==
          revision,
        { timeout: 20000 },
      )
      .catch(async () => {
        throw Error(
          JSON.stringify(
            await browser.tauri.execute(() => ({
              notice: document.querySelector(".notice")?.textContent,
              reader: document
                .querySelector(".reader")
                ?.getAttribute("data-revision"),
              busy: document.querySelector("main")?.getAttribute("aria-busy"),
              name: document.querySelector(".document-name")?.textContent,
              app: document
                .querySelector(".window-shell")
                ?.outerHTML.slice(0, 300),
              footer: document.querySelector("footer")?.textContent,
            })),
          ),
        );
      });
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 20000 });
    await expect(browser.$("article h1")).toHaveText("Far away");
  });
  it("authorizes relative navigation through a new native file selection", async () => {
    if (!(await browser.$(".outline").isDisplayed()))
      await browser.$("button=Outline & find").click();
    await browser.$(".outline nav").$("button=Reader integrity").click();
    await browser.$("article").$("a=link").waitForExist();
    await fixture(
      "seed",
      "# Linked document\n\nChosen through native selection.\n",
    );
    await browser.$("article").$("a=link").click();
    await expect(browser.$("article h1")).toHaveText("Linked document");
  });
  it("keeps malicious renderers and document HTML inert in the actual WebView", async () => {
    await fixture(
      "seed",
      "# Hostile input\n\n<script>window.__MARKTUB_PWN__=true</script>\n\n![remote](https://attacker.invalid/image.png)\n\n```mermaid\n%%{init:{securityLevel:loose}}%%\ngraph TD; A-->B\n```\n\n$$\n\\includegraphics{file:///etc/passwd}\n$$\n\nAfter blocked blocks.\n",
    );
    await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
    await browser.$("button.open-button").click();
    await expect(browser.$("article h1")).toHaveText("Hostile input");
    // Deterministic focus revalidation in the actual WebView; watcher delivery
    // itself is covered separately and remains a hint, not a correctness oracle.
    await browser.tauri.execute(() => window.dispatchEvent(new Event("focus")));
    await browser.waitUntil(
      async () => (await browser.$$(".render-error").length) >= 2,
      { timeout: 15000 },
    );
    await expect(browser.$("article")).toHaveText(
      expect.stringContaining("After blocked blocks."),
    );
    const safety = await browser.tauri.execute(() => ({
      executed: window.__MARKTUB_PWN__ === true,
      remote: performance
        .getEntriesByType("resource")
        .some((r) => r.name.includes("attacker.invalid")),
    }));
    expect(safety.executed).toBe(false);
    expect(safety.remote).toBe(false);
    const csp = await browser.tauri.execute(
      () =>
        new Promise((resolve) => {
          let image;
          const timer = setTimeout(() => {
            document.removeEventListener("securitypolicyviolation", blocked);
            image?.remove();
            resolve("NO_CSP_EVENT");
          }, 1500);
          function blocked(event) {
            if (!event.blockedURI.includes("attacker.invalid")) return;
            clearTimeout(timer);
            document.removeEventListener("securitypolicyviolation", blocked);
            image?.remove();
            resolve(event.violatedDirective);
          }
          document.addEventListener("securitypolicyviolation", blocked);
          image = document.createElement("img");
          image.src = "https://attacker.invalid/csp-probe.png";
          document.body.append(image);
        }),
    );
    expect(csp).toContain("img-src");
  });
});

describe("Working reader and export", () => {
  it("derives Read and export from the unsaved Source revision without clearing dirty state", async () => {
    await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Source").click();
    await expect(browser.$(".cm-content"))
      .toBeDisplayed()
      .catch(async () => {
        throw Error(
          JSON.stringify(
            await browser.tauri.execute(() => ({
              notice: document.querySelector(".notice")?.textContent,
              busy: document.querySelector("main")?.getAttribute("aria-busy"),
              status: document.querySelector("footer")?.textContent,
            })),
          ),
        );
      });
    await browser.$(".cm-content").addValue("DirtyExportMarker");
    await browser.$("button=Read").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Read").click();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 20000 });
    await browser.tauri.execute(async ({ core }) => {
      await core.invoke("e2e_fixture", {
        action: "seed",
        text: "Independent picker folder",
      });
      await core.invoke("e2e_fixture", { action: "export", text: null });
    });
    await browser.$("button=Export HTML").click();
    await browser.$("button=Export HTML").waitForEnabled({ timeout: 20000 });
    await expect(browser.$(".reader-notice")).toHaveText(
      expect.stringContaining("export completed"),
    );
    const info = await browser.tauri.execute(({ core }) =>
      core.invoke("e2e_fixture", { action: "read-export", text: null }),
    );
    const { readFile } = await import("node:fs/promises");
    const html = await readFile(info.path, "utf8");
    expect(html).toContain("DirtyExportMarker");
    expect(html).toContain("Working revision");
    await browser.$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.$("button=Source").click();
    await expect(
      browser.$(".document-tab:not([hidden]) .statusbar"),
    ).toHaveText(expect.stringContaining("Unsaved changes"));
  });
});

describe("Large reader measurements", () => {
  it("opens 1/10/100 MiB with bounded DOM and reaches the final indexed section", async function () {
    this.timeout(180000);
    const samples = [];
    for (const tier of [1, 10, 100]) {
      await browser.tauri.execute(
        ({ core }, action) =>
          core.invoke("e2e_fixture", { action, text: null }),
        `seed-reader-${tier}MiB`,
      );
      const started = performance.now();
      await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
      await browser.$("button.open-button").click();
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      await browser.waitUntil(
        async () =>
          (await browser.$("main").getAttribute("aria-busy")) === "false",
        { timeout: 30000 },
      );
      await browser.waitUntil(
        async () =>
          await browser
            .$("article h1")
            .getText()
            .then((t) => t === "Reader benchmark")
            .catch(() => false),
        { timeout: 30000 },
      );
      const firstViewportMs = performance.now() - started;
      await browser.$("button=Export HTML").waitForEnabled({ timeout: 120000 });
      const finalizedMs = performance.now() - started;
      const metrics = await browser.tauri.execute(() => ({
        nodes: document.querySelectorAll("*").length,
        sections: document.querySelector('input[aria-label="Go to section"]')
          ?.max,
        editors: document.querySelectorAll(".cm-editor").length,
      }));
      expect(metrics.nodes).toBeLessThan(1500);
      expect(metrics.editors).toBe(0);
      await browser
        .$('input[aria-label="Go to section"]')
        .setValue(metrics.sections);
      await browser.keys("Tab");
      await expect(browser.$("article")).toHaveText(
        expect.stringContaining("FINAL_READER_SENTINEL"),
      );
      const pid = (
        await browser.tauri.execute(({ core }) =>
          core.invoke("e2e_fixture", { action: "read", text: null }),
        )
      ).pid;
      const memory = spawnSync(
        "python3",
        [
          "benchmarks/source_memory.py",
          "--pid",
          String(pid),
          "--label",
          `reader-${tier}MiB`,
        ],
        { encoding: "utf8" },
      );
      expect(memory.status).toBe(0);
      samples.push({
        fixtureMiB: tier,
        firstViewportMs,
        finalizedMs,
        ...metrics,
      });
      await writeFile(
        "artifacts/reader-reader-open-progress.json",
        JSON.stringify(samples, null, 2),
      );
    }
    await writeFile(
      "artifacts/reader-reader-open.json",
      JSON.stringify(
        {
          profile:
            "Debug e2e; native-generated ordinary structured fixtures; warm imports; one sample/tier; time includes native snapshot/open IPC and visible text assertion, not compositor paint",
          samples,
        },
        null,
        2,
      ),
    );
  });
});

describe("Ordinary open latency", () => {
  it("records twenty warm native 1 MiB opens", async () => {
    const samples = [];
    for (let i = 0; i < 20; i++) {
      await browser.tauri.execute(({ core }) =>
        core.invoke("e2e_fixture", { action: "seed-reader-1MiB", text: null }),
      );
      const start = performance.now();
      await browser.$("button.open-button").waitForEnabled({ timeout: 20000 });
      await browser.$("button.open-button").click();
      await browser.waitUntil(
        async () =>
          (await browser.$("main").getAttribute("aria-busy")) === "false",
        { timeout: 10000 },
      );
      await expect(browser.$("article h1")).toHaveText("Reader benchmark");
      samples.push(performance.now() - start);
    }
    const sorted = [...samples].sort((a, b) => a - b);
    await writeFile(
      "artifacts/reader-ordinary-latency.json",
      JSON.stringify(
        {
          profile: `Debug ${nativeWebView}, warm imports/OS cache not controlled; native picker injected; main idle and first text assertion, not compositor paint; no compilation during run`,
          count: 20,
          samplesMs: samples,
          medianMs: (sorted[9] + sorted[10]) / 2,
          p95Ms: sorted[18],
        },
        null,
        2,
      ),
    );
  });
});

describe("Library", () => {
  let root, folder, source, target, appPid;
  const invoke = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  const activePane = () => browser.$(".document-tab:not([hidden])");
  async function complete(id) {
    let progress;
    await browser.waitUntil(
      async () => {
        progress = await invoke("library_job", { id, cancel: false });
        return !progress.running;
      },
      { timeout: 30000 },
    );
    return progress;
  }
  async function operation(kind, entries, destination, name = null) {
    const review = await invoke("library_prepare", {
      request: { kind, entries, destination, name },
    });
    return complete(await invoke("library_commit", { id: review.id }));
  }
  it("pages a real 3000-entry root and keeps the frontend bounded", async () => {
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "\ufeff# Library source\r\n\r\nneedle العربية\nlast\r",
    });
    appPid = fixture.pid;
    folder = fixture.path.slice(0, fixture.path.lastIndexOf("/"));
    const fs = await import("node:fs/promises");
    await fs.mkdir(`${folder}/destination`);
    await fs.mkdir(`${folder}/.git`);
    await fs.writeFile(`${folder}/.git/secret.txt`, "excluded");
    await fs.writeFile(`${folder}/.gitignore`, "ignored*.txt\n");
    await fs.writeFile(`${folder}/ignored-file.txt`, "ignored");
    await fs.symlink(folder, `${folder}/loop`);
    for (let start = 0; start < 3000; start += 100)
      await Promise.all(
        Array.from({ length: 100 }, (_, i) =>
          fs.writeFile(
            `${folder}/note-${String(start + i).padStart(4, "0")}.txt`,
            `# Note ${start + i}\n`,
          ),
        ),
      );
    const example = browser.$("button=Read the example");
    if (await example.isExisting()) {
      await example.click();
      await browser.$("article h1").waitForExist();
    }
    const focus = browser.$("button=Exit focus");
    if (await focus.isExisting()) await focus.click();
    await browser.$("button=Library").click();
    const start = performance.now();
    await browser.$("button=Add folder").click();
    await browser.$("button=File details").waitForEnabled({ timeout: 60000 });
    await browser.$("button=File details").click();
    await browser.waitUntil(
      async () =>
        (await browser.$(".listing-status").getText()).includes(
          "listing complete",
        ),
      { timeout: 20000 },
    );
    root = (await invoke("library_state")).roots[0].id;
    const page = await invoke("library_page", {
      id: root,
      offset: 3000,
      sort: "name",
      hidden: false,
      filter: "all",
      refresh: false,
      ignored: false,
    });
    source = page.items.find((i) => i.name === "source.txt").id;
    target = (
      await invoke("library_page", {
        id: root,
        offset: 0,
        sort: "name",
        hidden: false,
        filter: "all",
        refresh: false,
        ignored: false,
      })
    ).items.find((i) => i.name === "destination").directory;
    const metrics = await browser.tauri.execute(() => ({
      nodes: document.querySelectorAll("*").length,
      rows: document.querySelectorAll(".file-row").length,
      cm: document.querySelectorAll(".cm-editor").length,
    }));
    expect(metrics.rows).toBe(100);
    expect(metrics.nodes).toBeLessThan(2500);
    expect(page.total).toBe(3003); // destination, loop, source plus 3000 notes; ignored excluded
    await writeFile(
      "artifacts/library-folder.json",
      JSON.stringify(
        {
          ...metrics,
          nativeEntries: page.total,
          completeListingMs: performance.now() - start,
          profile: `debug ${nativeWebView}; injected native picker; fixture creation excluded; one sample`,
        },
        null,
        2,
      ),
    );
    await browser.saveScreenshot("artifacts/library-library.png");
    expect(
      spawnSync(
        "python3",
        [
          "benchmarks/source_memory.py",
          "--pid",
          String(appPid),
          "--label",
          "library-3000",
        ],
        { encoding: "utf8" },
      ).status,
    ).toBe(0);
  });
  it("previews and searches without opening every document", async () => {
    // An opaque ID, never a frontend path, is the authority for the preview.
    const preview = await invoke("library_preview", { id: source });
    expect(preview).toContain("needle العربية");
    const progress = await complete(
      await invoke("library_scan", {
        query: "needle",
        content: true,
        hidden: false,
        ignored: false,
      }),
    );
    expect(progress.results.map((h) => h.item.name)).toEqual(["source.txt"]);
    expect(progress.scanned).toBeGreaterThan(3000);
    expect(progress.results.some((h) => h.item.name === "secret.txt")).toBe(
      false,
    );
    const id = await invoke("library_scan", {
      query: "note",
      content: false,
      hidden: false,
      ignored: false,
    });
    await invoke("library_job", { id, cancel: true });
    const canceled = await complete(id);
    expect(canceled.canceled || canceled.truncated).toBe(true);
    const quick = browser.$("button=Search folders");
    await quick.click();
    await browser.$('[aria-label="Workspace query"]').setValue("source");
    await browser
      .$('dialog[aria-label="Quick open and search"]')
      .$("button=Search")
      .click();
    await browser
      .$(".workspace-search-results")
      .$('button[aria-label="Open source.txt"]')
      .waitForExist({ timeout: 20000 });
    await browser.$(".workspace-search-results").$("button=Preview").click();
    await expect(browser.$(".quick-preview article h1")).toHaveText(
      "Library source",
    );
    await browser.$("button=Close preview").click();
    await browser
      .$(".workspace-search-results")
      .$('button[aria-label="Open source.txt"]')
      .click();
    await browser.waitUntil(
      async () =>
        !(await browser
          .$('dialog[aria-label="Quick open and search"]')
          .getProperty("open")),
      { timeout: 60000 },
    );
    await expect(
      browser.$(".document-tab:not([hidden]) article h1"),
    ).toHaveText("Library source");
  });
  it("keeps a tab's Source state and blocks mutations while Source is live", async () => {
    const pane = activePane();
    await pane.$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane.$("button=Source").click();
    await pane.$(".cm-content").click();
    await browser.keys([
      process.platform === "darwin" ? "META" : "CONTROL",
      "END",
      "NULL",
    ]);
    await browser.keys(" tab survives");
    await browser.waitUntil(
      async () =>
        (await activePane().$(".window-shell").getAttribute("data-dirty")) ===
        "true",
    );
    const review = await invoke("library_prepare", {
      request: {
        kind: "createFile",
        entries: [],
        destination: root,
        name: "blocked.txt",
      },
    });
    let rejected = false;
    try {
      await invoke("library_commit", { id: review.id });
    } catch {
      rejected = true;
    }
    expect(rejected).toBe(true);
    expect((await import("node:fs")).existsSync(`${folder}/blocked.txt`)).toBe(
      false,
    );
    // Switch away and back without destroying CodeMirror or its undo state.
    const tabs = await browser.$$(".tabs [role=tab]");
    expect(tabs.length).toBeGreaterThanOrEqual(2);
    expect(
      spawnSync(
        "python3",
        [
          "benchmarks/source_memory.py",
          "--pid",
          String(appPid),
          "--label",
          "library-two-tabs-source",
        ],
        { encoding: "utf8" },
      ).status,
    ).toBe(0);
    await tabs[0].click();
    await tabs[tabs.length - 1].click();
    await expect(activePane().$(".cm-content")).toHaveText(
      expect.stringContaining("tab survives"),
    );
    const close = await browser.$$(".tabs .tab > button:last-child");
    await close[close.length - 1].click();
    await activePane().$("button=Keep recovery").click();
    await browser.waitUntil(
      async () => (await browser.$$(".cm-editor")).length === 0,
    );
  });
  it("creates, copies and moves only selected entries with exact bytes and collision refusal", async () => {
    const created = await operation("createFile", [], root, "created.txt");
    expect(created.outcomes[0].status).toBe("completed");
    const copied = await operation("copy", [source], target);
    expect(copied.outcomes[0].status).toBe("completed");
    const fs = await import("node:fs/promises");
    const before = await fs.readFile(`${folder}/source.txt`);
    expect(await fs.readFile(`${folder}/destination/source.txt`)).toEqual(
      before,
    );
    const collision = await operation("copy", [source], target);
    expect(collision.outcomes[0].status).toBe("failed");
    // Open a read snapshot, then the native move result rebinds its grant/title.
    await browser.$("button=Search folders").click();
    await browser.$('[aria-label="Workspace query"]').setValue("source");
    await browser
      .$('dialog[aria-label="Quick open and search"]')
      .$("button=Search")
      .click();
    await browser
      .$(".workspace-search-results")
      .$('button[aria-label="Open source.txt"]')
      .waitForExist({ timeout: 20000 });
    await browser
      .$(".workspace-search-results")
      .$('button[aria-label="Open source.txt"]')
      .click();
    await browser.waitUntil(
      async () =>
        !(await browser
          .$('dialog[aria-label="Quick open and search"]')
          .getProperty("open")),
      { timeout: 60000 },
    );
    await expect(
      browser.$(".document-tab:not([hidden]) article h1"),
    ).toHaveText("Library source");
    // Results appear while discovery is provisional. Preserve the native
    // no-active-job mutation gate: finish this scan before requesting rename.
    const searchJob = (await invoke("library_state")).job;
    if (searchJob) await complete(searchJob);
    const moved = await operation("rename", [source], root, "renamed.txt");
    expect(moved.outcomes[0].status).toBe("completed");
    await expect(activePane().$(".document-name")).toHaveText("renamed.txt");
    expect(await fs.readFile(`${folder}/renamed.txt`)).toEqual(before);
    expect((await import("node:fs")).existsSync(`${folder}/source.txt`)).toBe(
      false,
    );
    const state = await invoke("library_state");
    expect(state.recent.some((h) => h.name === "renamed.txt")).toBe(true);
    await expect(
      browser.$(".document-tab:not([hidden]) article h1"),
    ).toHaveText("Library source");
  });
  it("moves only a selected Linux entry to isolated system Trash with exact restore metadata", async function () {
    if (process.platform !== "linux") this.skip();
    const fs = await import("node:fs/promises");
    const data = process.env.XDG_DATA_HOME;
    expect(
      data?.startsWith(
        join((await import("node:os")).tmpdir(), "marktub-e2e-data-"),
      ),
    ).toBe(true);
    const name = "000-native-trash العربية 🦀.txt";
    const path = join(folder, name);
    const bytes = Buffer.from("\ufeffnative Trash\r\nالعربية\nend\r");
    await fs.writeFile(path, bytes);
    const request = {
      id: root,
      offset: 0,
      sort: "name",
      hidden: false,
      filter: "all",
      refresh: true,
      ignored: false,
    };
    let page = await invoke("library_page", request);
    request.refresh = false;
    await browser.waitUntil(
      async () => {
        page = await invoke("library_page", request);
        return page.complete;
      },
      { timeout: 30000 },
    );
    const entry = page.items.find((item) => item.name === name);
    expect(entry).toBeDefined();
    const before = new Set(
      await fs.readdir(join(data, "Trash/info")).catch(() => []),
    );
    const result = await operation("trash", [entry.id], null);
    expect(result.outcomes[0].status).toBe("completed");
    await expect(fs.stat(path)).rejects.toThrow();
    const added = (await fs.readdir(join(data, "Trash/info"))).filter(
      (name) => !before.has(name),
    );
    expect(added).toHaveLength(1);
    const metadata = await fs.readFile(
      join(data, "Trash/info", added[0]),
      "utf8",
    );
    const original = metadata
      .split("\n")
      .find((line) => line.startsWith("Path="))
      .slice(5);
    expect(decodeURIComponent(original)).toBe(path);
    expect(metadata).toMatch(
      /DeletionDate=\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/,
    );
    const stored = join(
      data,
      "Trash/files",
      added[0].replace(/\.trashinfo$/, ""),
    );
    expect(await fs.readFile(stored)).toEqual(bytes);
    await fs.rename(stored, path);
    expect(await fs.readFile(path)).toEqual(bytes);
    await writeFile(
      "artifacts/native-linux-trash.json",
      JSON.stringify(
        {
          restoredBytes: bytes.length,
          exactBytes: true,
          rawPathMetadataVerified: true,
          isolatedDataHome: true,
          scope:
            "actual WebKitGTK/native grant/mutation/Trash; independent filesystem restore, not physical file-manager input",
        },
        null,
        2,
      ),
    );
  });
  it("reviews native create/rename actions and restores the moved selection in the UI", async () => {
    const fs = await import("node:fs");
    const actions = browser.$(".file-actions");
    await actions.$("summary").click();
    await browser
      .$('[aria-label="New filename"]')
      .setValue("000-ui-created.txt");
    await actions.$("button=New file").waitForEnabled();
    await actions.$("button=New file").click();
    const dialog = browser.$('dialog[aria-label="Review file operation"]');
    await dialog.waitForDisplayed();
    await dialog.$("button=Cancel").click();
    expect(fs.existsSync(`${folder}/000-ui-created.txt`)).toBe(false);
    await actions.$("button=New file").waitForEnabled();
    await actions.$("button=New file").click();
    await dialog.waitForDisplayed();
    await dialog.$("button=Confirm operation").click();
    await browser.waitUntil(async () =>
      fs.existsSync(`${folder}/000-ui-created.txt`),
    );
    const selected = browser.$('[aria-label="Select 000-ui-created.txt"]');
    await selected.waitForExist();
    await selected.click();
    await browser
      .$('[aria-label="New filename"]')
      .setValue("001-ui-renamed.txt");
    await actions.$("button=Rename").waitForEnabled();
    await actions.$("button=Rename").click();
    await dialog.waitForDisplayed();
    await dialog.$("button=Confirm operation").click();
    await browser.waitUntil(async () =>
      fs.existsSync(`${folder}/001-ui-renamed.txt`),
    );
    const moved = browser.$('[aria-label="Select 001-ui-renamed.txt"]');
    await moved.waitForExist();
    await expect(moved).toBeSelected();
    await browser.$(".library-job").$("button=Open 001-ui-renamed.txt").click();
    try {
      await browser.waitUntil(
        async () =>
          (await browser
            .$(".document-tab:not([hidden]) .document-name")
            .getText()) === "001-ui-renamed.txt",
      );
    } catch (error) {
      throw new Error(
        JSON.stringify(
          await browser.tauri.execute(() => ({
            notices: [
              ...document.querySelectorAll(".notice,.library-message"),
            ].map((n) => n.textContent),
            tabs: [...document.querySelectorAll(".tabs [role=tab]")].map(
              (n) => n.textContent,
            ),
            dialogs: [...document.querySelectorAll("dialog[open]")].map(
              (n) => n.textContent,
            ),
          })),
        ) + String(error),
        { cause: error },
      );
    }
    expect(fs.existsSync(`${folder}/000-ui-created.txt`)).toBe(false);
  });
  it("accepts only native drop offers and keeps revoked roots unauthorized", async () => {
    let refused = false;
    try {
      await invoke("library_accept_drop", { id: "forged", action: "folder" });
    } catch {
      refused = true;
    }
    expect(refused).toBe(true);
    await invoke("e2e_fixture", { action: "library-drop", text: null });
    await browser.$('dialog[aria-label="Dropped items"]').waitForDisplayed();
    await browser
      .$('dialog[aria-label="Dropped items"]')
      .$("button=Add folder")
      .click();
    await browser.waitUntil(
      async () =>
        !(await browser.$('dialog[aria-label="Dropped items"]').isDisplayed()),
    );
    // Fixture exercises the backend offer path; physical OS dragging is a separate manual gate.
    await browser.waitUntil(async () =>
      (await browser.$(".listing-status").getText()).includes(
        "listing complete",
      ),
    );
    const livePage = await invoke("library_page", {
      id: root,
      offset: 0,
      sort: "name",
      hidden: false,
      filter: "all",
      refresh: false,
      ignored: false,
    });
    const live = livePage.items.find(
      (item) => item.name === "001-ui-renamed.txt",
    );
    expect(await invoke("library_preview", { id: live.id })).toBe("");
    await invoke("library_remove_root", { id: root });
    let denied = false;
    try {
      await invoke("library_preview", { id: live.id });
    } catch {
      denied = true;
    }
    expect(denied).toBe(true);
  });
});

describe("Preferences and commands", () => {
  afterEach(async () => {
    await browser.tauri.execute(() => {
      delete window.preferenceEditorContent;
      delete window.preferenceEditorSelection;
      delete window.preferenceReadingParagraph;
    });
  });
  const invoke = (command, args = {}) =>
    browser.tauri.execute(({ core }, c, a) => core.invoke(c, a), command, args);
  const pane = () => browser.$(".document-tab:not([hidden])");
  const prefs = () => browser.$('dialog[aria-label="Preferences"]');
  const original = "\uFEFF# Phase four\r\nالعربية עברית 漢字 🦀\nlast\rend";
  const advanced = async (text) => {
    await prefs().$("button=Advanced").click();
    await prefs().$('textarea[aria-label="Settings text"]').setValue(text);
    await browser.waitUntil(
      async () => (await prefs().getAttribute("aria-busy")) === "false",
    );
  };
  it("uses bounded installed font names and refuses invalid literals, contrast and shortcut conflicts without persistence", async () => {
    const catalogue = await invoke("preference_catalogue");
    const fonts = await invoke("list_fonts");
    expect(catalogue.some((s) => s.key === "reader.font")).toBe(true);
    expect(fonts.length).toBeGreaterThan(4);
    expect(fonts.length).toBeLessThanOrEqual(512);
    await mkdir("artifacts", { recursive: true });
    await writeFile(
      "artifacts/preferences-native-catalogue.json",
      JSON.stringify(
        { fontFamilies: fonts.length, settings: catalogue.length },
        null,
        2,
      ),
    );
    const before = await invoke("get_settings", { id: null, scope: "global" });
    for (const text of [
      '$theme=custom\n$color.ink="#ffffff"',
      '$shortcut.save="Mod+Q"',
      '$shortcut.save="Mod+O"',
      '$ui.font="url(https://evil)"\n$network=true',
    ]) {
      const rejected = await invoke("update_settings", {
        id: null,
        scope: "global",
        text,
      });
      expect(rejected.diagnostics.length).toBeGreaterThan(0);
    }
    const draft = await invoke("preview_settings", {
      id: null,
      scope: "global",
      text: "$reader.width=890",
    });
    expect(draft.resolved["reader.width"].value).toBe(890);
    expect(
      (await invoke("get_settings", { id: null, scope: "global" })).text,
    ).toBe(before.text);
    await invoke("update_settings", { id: null, scope: "session", text: "" });
  });
  it("previews all four themes and typography while preserving Source node, caret, dirty state and byte-exact undo", async () => {
    await invoke("e2e_fixture", { action: "seed", text: original });
    await pane().$("button.open-button").waitForEnabled();
    await pane().$("button.open-button").click();
    if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
      await browser.$("button=Discard edits").click();
    await pane().$("button=Source").waitForEnabled();
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed();
    await pane().$(".cm-content").addValue("文");
    await browser.tauri.execute(() => {
      window.preferenceEditorContent = document.querySelector(
        ".document-tab:not([hidden]) .cm-content",
      );
      window.preferenceEditorSelection =
        window.preferenceEditorContent.cmTile.root.view.state.selection.main.head;
    });
    await documentAction("Settings");
    await prefs().waitForDisplayed();
    for (const [label, hex] of [
      ["Light", "#ffffff"],
      ["Parchment", "#fbf5e9"],
      ["Dark", "#202127"],
      ["Ink", "#17202c"],
    ]) {
      await prefs().$(`button[aria-label="Use ${label} theme"]`).click();
      await browser.waitUntil(
        async () =>
          (await browser.tauri.execute(() =>
            document
              .querySelector(".document-tab:not([hidden]) .window-shell")
              .style.getPropertyValue("--paper"),
          )) === hex,
      );
    }
    await prefs()
      .$('input[aria-label="Search preferences"]')
      .setValue("Source size");
    await prefs().$('input[id="pref-editor.font_size"]').setValue("19");
    await browser.waitUntil(
      async () =>
        (await browser.tauri.execute(() =>
          document
            .querySelector(".document-tab:not([hidden]) .window-shell")
            .style.getPropertyValue("--source-size"),
        )) === "19px",
    );
    const footer = await browser.tauri.execute(() => {
      const r = document
        .querySelector('dialog[aria-label="Preferences"] button.primary')
        .getBoundingClientRect();
      return (
        r.top >= 0 &&
        r.bottom <= innerHeight &&
        r.left >= 0 &&
        r.right <= innerWidth
      );
    });
    expect(footer).toBe(true);
    await browser.saveScreenshot("artifacts/preferences-preferences.png");
    await prefs().$("button=Cancel").click();
    expect(
      await browser.tauri.execute(
        () =>
          window.preferenceEditorContent ===
          document.querySelector(".document-tab:not([hidden]) .cm-content"),
      ),
    ).toBe(true);
    await documentAction("Settings");
    await prefs()
      .$('select[aria-label="Settings scope"]')
      .selectByAttribute("value", "session");
    await browser.waitUntil(
      async () => !(await prefs().$("fieldset").getProperty("disabled")),
    );
    await advanced(
      "$theme=dark\n$editor.font_size=17\n$editor.wrap=false\n$ui.scale=110\n$reader.line_height=190",
    );
    await prefs().$("button=Apply settings").click();
    await prefs().waitForDisplayed({ reverse: true });
    expect(
      await browser.tauri.execute(
        () =>
          window.preferenceEditorContent ===
          document.querySelector(".document-tab:not([hidden]) .cm-content"),
      ),
    ).toBe(true);
    expect(await pane().$(".window-shell").getAttribute("data-dirty")).toBe(
      "true",
    );
    expect(
      await browser.tauri.execute(
        () =>
          window.preferenceEditorContent.cmTile.root.view.state.selection.main
            .head,
      ),
    ).toBe(await browser.tauri.execute(() => window.preferenceEditorSelection));
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane().$("header").$("button=Save").click();
    await browser.waitUntil(
      async () =>
        (await invoke("e2e_fixture", { action: "read", text: null })).text ===
        original + "文",
    );
    await pane()
      .$('button[aria-label="Undo"]')
      .waitForEnabled({ timeout: 60000 });
    await pane().$('button[aria-label="Undo"]').click();
    await browser.waitUntil(
      async () =>
        (await pane().$(".window-shell").getAttribute("data-dirty")) === "true",
      { timeout: 60000 },
    );
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane().$("header").$("button=Save").click();
    await browser.waitUntil(
      async () =>
        (await invoke("e2e_fixture", { action: "read", text: null })).text ===
        original,
      { timeout: 60000 },
    );
  });
  it("dispatches contextual fuzzy actions and rebound chords without firing during composition", async () => {
    await pane().$("button=Read").waitForEnabled();
    await browser.$("button=Commands").click();
    const palette = () => browser.$('dialog[aria-label="Command palette"]');
    await palette().$("input").setValue("outline");
    expect(
      await palette().$("#command-outline").getAttribute("aria-disabled"),
    ).toBe("true");
    await palette().$("input").setValue("rdmd");
    await expect(palette().$("#command-read")).toHaveAttribute(
      "aria-disabled",
      "false",
    );
    await palette().$("#command-read").click();
    await palette().waitForDisplayed({ reverse: true });
    await pane().$("article h1").waitForDisplayed();
    await browser.$("button=Commands").click();
    await palette().$("input").setValue("save");
    expect(
      await palette().$("#command-save").getAttribute("aria-disabled"),
    ).toBe("false");
    await palette().$("button=Esc").click();
    await invoke("update_settings", {
      id: null,
      scope: "session",
      text: '$theme=paper\n$shortcut.palette="Mod+Alt+P"',
    });
    await browser.waitUntil(
      async () =>
        (await browser.tauri.execute(
          () =>
            document.querySelector(".document-tab:not([hidden]) .window-shell")
              .dataset.theme,
        )) === "paper",
    );
    await browser.tauri.execute(() =>
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "p",
          code: "KeyP",
          metaKey: navigator.platform.startsWith("Mac"),
          ctrlKey: !navigator.platform.startsWith("Mac"),
          altKey: true,
          isComposing: true,
          bubbles: true,
        }),
      ),
    );
    expect(await palette().isDisplayed()).toBe(false);
    await browser.tauri.execute(() =>
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "p",
          code: "KeyP",
          metaKey: navigator.platform.startsWith("Mac"),
          ctrlKey: !navigator.platform.startsWith("Mac"),
          altKey: true,
          bubbles: true,
        }),
      ),
    );
    await palette().waitForDisplayed();
    await palette().$("input").setValue("srcmd");
    await palette().$("#command-source").click();
    await pane().$(".cm-content").waitForDisplayed();
    await invoke("update_settings", { id: null, scope: "session", text: "" });
    await browser.saveScreenshot("artifacts/preferences-source.png");
  });
  it("keeps the visible reading paragraph through typography changes and renders local math across light and dark palettes", async () => {
    const rich =
      '# Fonts and equations\n\nالعربية עברית 漢字 🦀\n\nInline $a^2$ and compatibility $\\prescript{a}{b}{X}$.\n\n```js\nconst words = "local" + 42;\n```\n\n' +
      Array.from(
        { length: 70 },
        (_, i) => `Paragraph ${i}: ` + "Words kept in place. ".repeat(15),
      ).join("\n\n");
    await invoke("e2e_fixture", { action: "seed", text: rich });
    await pane().$("button.open-button").waitForEnabled();
    await pane().$("button.open-button").click();
    await pane().$("article h1").waitForDisplayed();
    await browser.waitUntil(
      async () =>
        (await pane().$$(".katex")).length > 0 &&
        (await pane().$$(".rich-placeholder svg")).length > 0,
      { timeout: 30000 },
    );
    for (const theme of ["light", "paper", "dark", "midnight"]) {
      await invoke("update_settings", {
        id: null,
        scope: "session",
        text: `$theme=${theme}`,
      });
      await browser.waitUntil(
        async () =>
          (await pane().$(".window-shell").getAttribute("data-theme")) ===
          theme,
      );
      await browser.saveScreenshot(`artifacts/preferences-reader-${theme}.png`);
    }
    const before = await browser.tauri.execute(() => {
      const host = document.querySelector(
        ".document-tab:not([hidden]) article",
      );
      host.scrollTop = 700;
      const top = host.getBoundingClientRect().top;
      window.preferenceReadingParagraph = [...host.querySelectorAll("p")].find(
        (p) => p.getBoundingClientRect().bottom > top,
      );
      return {
        text: window.preferenceReadingParagraph.textContent,
        offset:
          window.preferenceReadingParagraph.getBoundingClientRect().top - top,
      };
    });
    await invoke("update_settings", {
      id: null,
      scope: "session",
      text: '$theme=paper\n$reader.font_size=26\n$reader.width=600\n$reader.line_height=200\n$reader.font="Serif"',
    });
    await browser.waitUntil(
      async () =>
        (await browser.tauri.execute(() =>
          document
            .querySelector(".document-tab:not([hidden]) .window-shell")
            .style.getPropertyValue("--reader-size"),
        )) === "26px",
    );
    const after = await browser.tauri.execute(() => {
      const host = document.querySelector(
        ".document-tab:not([hidden]) article",
      );
      return {
        connected: window.preferenceReadingParagraph.isConnected,
        text: window.preferenceReadingParagraph.textContent,
        offset:
          window.preferenceReadingParagraph.getBoundingClientRect().top -
          host.getBoundingClientRect().top,
        dpr: devicePixelRatio,
      };
    });
    expect(after.connected).toBe(true);
    expect(after.text).toBe(before.text);
    expect(Math.abs(after.offset - before.offset)).toBeLessThan(2);
    await writeFile(
      "artifacts/preferences-reading-position.json",
      JSON.stringify({ before, after }, null, 2),
    );
    await pane().$("button=Source").waitForEnabled();
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed();
    for (const theme of ["light", "paper", "dark", "midnight"]) {
      await invoke("update_settings", {
        id: null,
        scope: "session",
        text: `$theme=${theme}`,
      });
      await browser.waitUntil(
        async () =>
          (await pane().$(".window-shell").getAttribute("data-theme")) ===
          theme,
      );
      await browser.saveScreenshot(`artifacts/preferences-source-${theme}.png`);
    }
    await invoke("update_settings", { id: null, scope: "session", text: "" });
  });
});

describe("Author experience", () => {
  const invoke = (c, a = {}) =>
    browser.tauri.execute(({ core }, c, a) => core.invoke(c, a), c, a);
  const pane = () => browser.$(".document-tab:not([hidden])");
  const tools = () => browser.$('dialog[aria-label="Edit block"]');
  const content =
    "\uFEFF# Author 🦀\r\n\r\nalpha العربية עברית 漢字.\n\n- [ ] task\r\n\r\n| first | second |\n| :--- | ---: |\r\n| `a\\|b` | cell |\n\nMath $a^2$.\r\n\r\n```mermaid\ngraph TD\nA[Read] --> B[Write]\n```\n\nTail\r";
  let fixture;
  const view = async (fn, ...args) =>
    browser.tauri.execute(
      new Function(
        "_context",
        "...args",
        `const v=document.querySelector('.document-tab:not([hidden]) .cm-content')?.cmTile?.root?.view;return (${fn.toString()})(v,...args);`,
      ),
      ...args,
    );
  async function seed(text = content, action = "seed") {
    fixture = await invoke("e2e_fixture", { action, text });
    await pane().$("button.open-button").waitForEnabled({ timeout: 60000 });
    await pane().$("button.open-button").click();
    if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
      await browser.$("button=Discard edits").click();
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
  }
  async function select(text) {
    await view((v, text) => {
      const at = v.state.doc.toString().indexOf(text);
      if (at < 0) throw Error("Missing source span " + text);
      v.dispatch({
        selection: { anchor: at, head: at + text.length },
        scrollIntoView: true,
      });
    }, text);
  }
  async function saved(expected) {
    await pane().$("header").$("button=Save").waitForEnabled();
    await pane().$("header").$("button=Save").click();
    await browser.waitUntil(
      async () =>
        (await invoke("e2e_fixture", { action: "read", text: null })).text ===
        expected,
      { timeout: 30000 },
    );
    await pane()
      .$('.window-shell[data-busy="false"][data-dirty="false"]')
      .waitForExist({ timeout: 30000 });
  }
  it("syncs actual viewport positions both ways without moving the editor selection", async () => {
    let ending = 0;
    const source =
      "\uFEFF" +
      Array.from(
        { length: 80 },
        (_, i) =>
          `## Section ${i}\n\n${"A wrapped paragraph العربية 日本語 🦀 for scroll mapping. ".repeat(8)}\n\n`,
      )
        .join("")
        .replace(/\n/g, () => ["\r\n", "\n", "\r"][ending++ % 3]);
    await seed(source);
    const initialText = await view((v) => v.state.doc.toString());
    await pane().$("button=Split").click();
    await pane().$(".reader-scroll").waitForDisplayed({ timeout: 60000 });
    await view((v) =>
      v.dispatch({ selection: { anchor: v.state.doc.line(5).from } }),
    );
    const selected = await view((v) => v.state.selection.main.anchor);
    await view((v) => {
      v.scrollDOM.dispatchEvent(
        new WheelEvent("wheel", { bubbles: true, deltaY: 1 }),
      );
      v.scrollDOM.scrollTop +=
        v.documentTop +
        v.lineBlockAt(v.state.doc.line(123).from).top -
        v.scrollDOM.getBoundingClientRect().top;
    });
    await browser.waitUntil(
      () =>
        browser.tauri.execute(() => {
          const pane = document.querySelector(".document-tab:not([hidden])");
          const reader = pane.querySelector(".reader-scroll");
          const node =
            reader.querySelector("#h-section-30")?.nextElementSibling;
          const v = pane.querySelector(".cm-content")?.cmTile?.root?.view;
          if (!node || !v) return false;
          const y = Math.max(
            0,
            v.scrollDOM.getBoundingClientRect().top - v.documentTop + 1,
          );
          const block = v.lineBlockAtHeight(y);
          const fraction = (y - block.top) / Math.max(1, block.height);
          const renderedFraction =
            (reader.getBoundingClientRect().top -
              node.getBoundingClientRect().top) /
            Math.max(1, node.getBoundingClientRect().height);
          return (
            v.state.doc.lineAt(block.from).number === 123 &&
            Math.abs(fraction - renderedFraction) < 0.03
          );
        }),
      { timeout: 10000 },
    );
    expect(await view((v) => v.state.selection.main.anchor)).toBe(selected);
    await browser.tauri.execute(() => {
      const reader = document.querySelector(
        ".document-tab:not([hidden]) .reader-scroll",
      );
      const node = reader.querySelector("#h-section-31")?.nextElementSibling;
      reader.dispatchEvent(
        new WheelEvent("wheel", { bubbles: true, deltaY: 1 }),
      );
      reader.scrollTop +=
        node.getBoundingClientRect().top -
        reader.getBoundingClientRect().top +
        node.getBoundingClientRect().height * 0.4;
    });
    await browser.waitUntil(
      async () =>
        (await view(
          (v) =>
            v.state.doc.lineAt(
              v.lineBlockAtHeight(
                Math.max(
                  0,
                  v.scrollDOM.getBoundingClientRect().top - v.documentTop + 1,
                ),
              ).from,
            ).number,
        )) === 127,
      { timeout: 10000 },
    );
    expect(await view((v) => v.state.selection.main.anchor)).toBe(selected);
    // Reverse direction promptly: genuine input must override a pending sync.
    await browser.tauri.execute(() => {
      const reader = document.querySelector(
        ".document-tab:not([hidden]) .reader-scroll",
      );
      const node = reader.querySelector("#h-section-30")?.nextElementSibling;
      reader.dispatchEvent(
        new WheelEvent("wheel", { bubbles: true, deltaY: -1 }),
      );
      reader.scrollTop +=
        node.getBoundingClientRect().top - reader.getBoundingClientRect().top;
    });
    await browser.waitUntil(
      async () =>
        (await view(
          (v) =>
            v.state.doc.lineAt(
              v.lineBlockAtHeight(
                Math.max(
                  0,
                  v.scrollDOM.getBoundingClientRect().top - v.documentTop + 1,
                ),
              ).from,
            ).number,
        )) === 123,
      { timeout: 10000 },
    );
    await browser.tauri.execute(() => {
      window.splitReaderBefore = document.querySelector(
        ".document-tab:not([hidden]) .reader",
      );
    });
    await view((v) =>
      v.dispatch({ changes: { from: v.state.doc.length, insert: "\nTail" } }),
    );
    await browser.waitUntil(
      () =>
        browser.tauri.execute(() => {
          const reader = document.querySelector(
            ".document-tab:not([hidden]) .reader",
          );
          const host = reader?.querySelector(".reader-scroll");
          const node = host?.querySelector("#h-section-30")?.nextElementSibling;
          return (
            reader &&
            reader !== window.splitReaderBefore &&
            node &&
            Math.abs(
              node.getBoundingClientRect().top -
                host.getBoundingClientRect().top,
            ) < 8
          );
        }),
      { timeout: 30000 },
    );
    expect(await view((v) => v.state.selection.main.anchor)).toBe(selected);
    const undo = pane().$(".source-toolbar").$("button=Undo");
    await undo.waitForEnabled();
    await undo.click();
    await browser.waitUntil(
      async () => (await view((v) => v.state.doc.toString())) === initialText,
    );
    expect(await view((v) => v.state.doc.toString())).toBe(initialText);
    expect(await view((v) => v.state.selection.main.anchor)).toBe(selected);
    await saved(source);
  });
  it("keeps the same editor, document, caret, folds and undo across Source/Split/Visual/Read", async () => {
    await seed();
    await select("alpha");
    await pane().$(".author-controls").$('button[aria-label="Bold"]').click();
    await select("```mermaid");
    await pane().$(".author-controls").$("button=Fold").click();
    await pane().$(".cm-foldPlaceholder").waitForDisplayed();
    await view((v) => {
      window.authorView = v;
      window.authorDoc = v.state.doc;
      window.authorCaret = v.state.selection.main.head;
    });
    await pane().$("button=Split").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Split").click();
    await pane().$("article").waitForDisplayed();
    await expect(pane().$("article")).toHaveText(
      expect.stringContaining("alpha"),
    );
    const position = await pane()
      .$(".split-divider")
      .getAttribute("aria-label");
    expect(position).toBe("Resize source and preview");
    await browser.saveScreenshot("artifacts/author-split.png");
    for (const mode of ["Visual", "Read", "Source"]) {
      await pane().$(`button=${mode}`).click();
      if (mode === "Read") await pane().$("article").waitForDisplayed();
      else await pane().$(".cm-content").waitForDisplayed();
      expect(
        await view(
          (v) =>
            v === window.authorView &&
            v.state.doc === window.authorDoc &&
            v.state.selection.main.head === window.authorCaret,
        ),
      ).toBe(true);
      expect(await pane().$(".cm-foldPlaceholder").isExisting()).toBe(true);
    }
    await saved(content.replace("alpha", "**alpha**"));
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
    await pane().$(".author-controls").$("button=Find / replace").click();
    await pane().$(".cm-search").waitForDisplayed();
    await pane().$('.cm-search input[name="search"]').setValue("alpha");
    await pane().$('.cm-search input[name="replace"]').setValue("replacement");
    await pane().$('.cm-search button[name="replaceAll"]').click();
    await saved(content.replace("alpha", "replacement"));
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
  });
  it("edits escaped-code table cells and shape through Visual controls with byte-exact undo", async () => {
    await seed();
    if (!(await browser.$(".library-host").isDisplayed()))
      await browser.$("button=Library").click();
    // Constrain the real WebView pane even when the OS maximizes the test window.
    await browser.tauri.execute(() => {
      const host = document.querySelector(".document-host");
      host.style.maxWidth = "714px";
    });
    await select("alpha");
    await pane().$("button=Visual").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Visual").click();
    await pane().$(".cm-author-block table").waitForDisplayed();
    await pane().$(".cm-author-block table").$("button=cell").click();
    await tools().waitForDisplayed();
    expect(
      await tools().$('input[aria-label="Table cell text"]').getValue(),
    ).toBe("cell");
    await tools()
      .$('input[aria-label="Table cell text"]')
      .setValue("文 | next");
    await tools().$("button=Apply cell").click();
    await tools().$("button=Close").click();
    await saved(content.replace("cell", "文 \\| next"));
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
    await pane().$(".cm-author-block table").$("button=cell").click();
    await tools().$("button=Add column").click();
    await tools().$("button=Close").click();
    expect(await view((v) => v.state.doc.toString())).toContain(
      "| first | second |   |",
    );
    await saved(
      content
        .replace("| first | second |", "| first | second |   |")
        .replace("| :--- | ---: |", "| :--- | ---: | --- |")
        .replace("| `a\\|b` | cell |", "| `a\\|b` | cell |   |"),
    );
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
    const layout = await browser.tauri.execute(() => {
      const tab = document.querySelector(".document-tab:not([hidden])");
      const shell = tab.querySelector(".window-shell");
      const box = tab.getBoundingClientRect();
      return {
        scrollLeft: tab.scrollLeft,
        shellLeft: shell.getBoundingClientRect().left - box.left,
      };
    });
    expect(layout.scrollLeft).toBe(0);
    expect(layout.shellLeft).toBeGreaterThanOrEqual(0);
    await browser.saveScreenshot("artifacts/author-visual.png");
    await browser.tauri.execute(() => {
      document.querySelector(".document-host").style.maxWidth = "";
    });
  });
  it("edits math and diagrams locally, rejects resource directives, and keeps one source transaction per Apply", async () => {
    await seed();
    await select("a^2");
    await pane().$(".author-controls").$("button=Math").click();
    await tools().$("textarea").setValue("b^3");
    await tools().$("button=Preview locally").click();
    await tools().$(".katex").waitForExist();
    await tools().$("button=Apply block").click();
    await saved(content.replace("$a^2$", "$b^3$"));
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
    await select("graph TD");
    await pane().$(".author-controls").$("button=Diagram").click();
    await tools().$("button=Preview locally").click();
    await tools().$("svg").waitForExist({ timeout: 30000 });
    await tools()
      .$("textarea")
      .setValue('graph TD\nclick A "https://evil.invalid"');
    await tools().$("button=Preview locally").click();
    await tools().$('p[role="alert"]').waitForDisplayed();
    expect(await tools().$('p[role="alert"]').getText()).toContain("disabled");
    await tools().$("button=Close").click();
    await select("graph TD");
    await pane().$(".author-controls").$("button=Diagram").click();
    await tools().$("textarea").setValue("graph TD\nA --> C");
    await tools().$("button=Apply block").click();
    await saved(
      content.replace("graph TD\nA[Read] --> B[Write]", "graph TD\nA --> C"),
    );
    await pane().$('button[aria-label="Undo"]').click();
    await saved(content);
  });
  it("copies only native-selected images, preserves assets on text undo and refuses collisions", async () => {
    await seed("# Assets\r\n\r\n");
    await view((v) =>
      v.dispatch({ selection: { anchor: v.state.doc.length } }),
    );
    await pane().$(".author-controls").$("button=Image").click();
    await invoke("e2e_fixture", { action: "image", text: null });
    await invoke("e2e_fixture", { action: "save-as", text: null });
    await tools().$("button=Choose and copy image").click();
    await browser.waitUntil(
      async () =>
        (await tools().$('input[aria-label="Image path"]').getValue()) ===
        "asset.png",
    );
    await tools().$('input[aria-label="Image alt text"]').setValue("文 🦀");
    await tools().$("button=Apply block").click();
    await saved("# Assets\r\n\r\n![文 🦀](asset.png)");
    await select("# Assets");
    await pane().$("button=Visual").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Visual").click();
    await pane().$(".cm-author-block img").waitForDisplayed();
    await pane().$(".cm-author-block").$("button=Edit image").click();
    await tools().$("button=Move block up").click();
    await saved("# Assets\r\n![文 🦀](asset.png)\r\n");
    await pane().$('button[aria-label="Undo"]').click();
    await saved("# Assets\r\n\r\n![文 🦀](asset.png)");
    const fs = await import("node:fs");
    const path = await import("node:path");
    const folder = path.dirname(fixture.path);
    expect(
      fs
        .readFileSync(path.join(folder, "asset.png"))
        .equals(fs.readFileSync(path.join(folder, "pixel.png"))),
    ).toBe(true);
    await pane().$('button[aria-label="Undo"]').click();
    await saved("# Assets\r\n\r\n");
    expect(fs.existsSync(path.join(folder, "asset.png"))).toBe(true);
    await pane().$(".author-controls").$("button=Image").click();
    await invoke("e2e_fixture", { action: "image", text: null });
    await invoke("e2e_fixture", { action: "save-as", text: null });
    await tools().$("button=Choose and copy image").click();
    await tools().$('p[role="alert"]').waitForDisplayed();
    expect(await tools().$('p[role="alert"]').getText()).toContain("unused");
    await tools().$("button=Close").click();
    expect(await view((v) => v.state.doc.toString())).toBe("# Assets\n\n");
    const heightOnly = "# Size\r\n\r\n![one](pixel.png){height=120}\n";
    await seed(heightOnly);
    await select("![one]");
    await pane().$(".author-controls").$("button=Image").click();
    await tools().$('input[aria-label="Image alt text"]').setValue("edited");
    await tools().$("button=Apply block").click();
    await saved(heightOnly.replace("![one]", "![edited]"));
    await pane().$('button[aria-label="Undo"]').click();
    await saved(heightOnly);
  });
  it("reviews an opaque native image drop before inserting a source reference", async () => {
    await seed("# Drop\n\n");
    await invoke("e2e_fixture", { action: "image", text: null });
    await invoke("e2e_fixture", { action: "save-as", text: null });
    await invoke("e2e_fixture", { action: "author-drop", text: null });
    await tools().waitForDisplayed();
    await browser.waitUntil(
      async () =>
        (await tools().$('input[aria-label="Image path"]').getValue()) ===
        "asset.png",
    );
    await tools().$("button=Close").click();
    expect(await view((v) => v.state.doc.toString())).toBe("# Drop\n\n");
  });
  it("reveals syntax during composition events and keeps writing focus reversible", async () => {
    await seed();
    await select("alpha");
    await pane().$("button=Visual").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Visual").click();
    const before = await view((v) => v.state.doc.toString());
    await pane().$(".author-controls").$("button=Writing focus").click();
    await pane().$("button=Exit writing focus").waitForDisplayed();
    await browser.saveScreenshot("artifacts/author-writing-focus.png");
    await pane().$("button=Exit writing focus").click();
    await browser.tauri.execute(() =>
      document
        .querySelector(".document-tab:not([hidden]) .cm-content")
        .dispatchEvent(
          new CompositionEvent("compositionstart", { bubbles: true, data: "" }),
        ),
    );
    await browser.waitUntil(
      async () => await view((v) => v.compositionStarted),
    );
    await pane().$(".author-controls").$('button[aria-label="Bold"]').click();
    expect(await view((v) => v.state.doc.toString())).toBe(before);
    await browser.tauri.execute(() =>
      document
        .querySelector(".document-tab:not([hidden]) .cm-content")
        .dispatchEvent(
          new CompositionEvent("compositionend", { bubbles: true, data: "" }),
        ),
    );
    await browser.waitUntil(
      async () => !(await view((v) => v.compositionStarted)),
    );
    // This checks event guards/decorations, not an OS input method or candidate conversion.
  });
  it("edits two ranges of a real 100 MiB document and saves only intended bytes", async () => {
    await seed(null, "seed-reader-100MiB");
    const fs = await import("node:fs");
    const original = fs.readFileSync(fixture.path);
    const expected = Buffer.from(original);
    await pane().$(".range-controls").waitForDisplayed();
    expect(await view((v) => v.state.doc.length)).toBeLessThanOrEqual(131072);
    await view((v) => v.dispatch({ changes: { from: 0, to: 1, insert: "A" } }));
    await pane().$('input[aria-label="Range start"]').setValue("1000000");
    await pane().$("button=Go to range").click();
    const rangeStartLabel = await browser.tauri.execute(() =>
      (1000000).toLocaleString(),
    );
    await browser.waitUntil(
      async () =>
        (await pane().$(".range-controls strong").getText()).startsWith(
          `Range ${rangeStartLabel}–`,
        ),
      { timeout: 30000 },
    );
    await view((v) =>
      v.dispatch({ changes: { from: 10, to: 11, insert: "Z" } }),
    );
    await pane().$('input[aria-label="Range start"]').setValue("0");
    await pane().$("button=Go to range").click();
    await browser.waitUntil(
      async () => (await view((v) => v.state.doc.sliceString(0, 1))) === "A",
      { timeout: 30000 },
    );
    expected[0] = 65;
    let byte = 0,
      logical = 0;
    while (logical < 1000010) {
      if (original[byte] === 13 && original[byte + 1] === 10) byte += 2;
      else byte++;
      logical++;
    }
    expected[byte] = 90;
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane().$("header").$("button=Save").click();
    await browser.waitUntil(
      async () => fs.readFileSync(fixture.path).equals(expected),
      { timeout: 60000 },
    );
    expect(await view((v) => v.state.doc.length)).toBeLessThanOrEqual(131072);
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await writeFile(
      "artifacts/author-range.json",
      JSON.stringify(
        {
          bytes: expected.length,
          windowUnits: await view((v) => v.state.doc.length),
          twoRangesExact: true,
        },
        null,
        2,
      ),
    );
    await browser.saveScreenshot("artifacts/author-range.png");
    const measured = spawnSync(
      "python3",
      [
        "benchmarks/source_memory.py",
        "--pid",
        String(fixture.pid),
        "--label",
        "author-range-100MiB",
      ],
      { encoding: "utf8" },
    );
    expect(measured.status).toBe(0);
    await pane().$("button=Read").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Read").click();
    try {
      await pane().$("article").waitForDisplayed({ timeout: 30000 });
      await browser.waitUntil(
        async () =>
          (await pane().$("article").getText()).includes("A Reader benchmark"),
        {
          timeout: 60000,
          timeoutMsg:
            "Full native working snapshot did not replace the saved Reader input",
        },
      );
    } catch (error) {
      await browser.saveScreenshot("artifacts/author-range-read-failure.png");
      console.error((await pane().getText()).slice(0, 1600));
      console.error(
        await browser.tauri.execute(() => ({
          revision: document.querySelector(".reader")?.dataset.revision,
          section: document.querySelector("article section")?.dataset.section,
          prefix: document.querySelector("article")?.innerHTML.slice(0, 500),
        })),
      );
      throw error;
    }
    await pane()
      .$("button=Edit this section")
      .waitForEnabled({ timeout: 60000 });
    await pane().$("button=Edit this section").click();
    // Range rotation replaces the editor; query the live node on each poll.
    await browser.waitUntil(async () => pane().$(".cm-content").isDisplayed(), {
      timeout: 30000,
      timeoutMsg: "Current range editor must be visible",
    });
    expect(await view((v) => v.state.doc.sliceString(0, 1))).toBe("A");
  });
});

describe("Git workbench", () => {
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      await browser.saveScreenshot("artifacts/git-failure.png");
      if (await panel().isExisting())
        console.error(
          "GIT FAILURE STATE",
          (await panel().getText()).slice(0, 2200),
        );
    }
  });
  const call = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  const panel = () => browser.$('dialog[aria-label="Git workbench"]');
  // Embedded macOS driver's option.click() does not change a native select.
  // Exercise the real DOM change handler; this is not physical OS input evidence.
  const baseline = async (side, value) => {
    await panel()
      .$(`select[aria-label="Git ${side}"]`)
      .waitForEnabled({ timeout: 30000 });
    await browser.tauri.execute(
      (_api, side, value) => {
        const select = document.querySelector(
          `select[aria-label="Git ${side}"]`,
        );
        select.value = value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      },
      side,
      value,
    );
    await browser.waitUntil(
      async () =>
        (await panel().$(`select[aria-label="Git ${side}"]`).getValue()) ===
        value,
    );
    await panel()
      .$(`select[aria-label="Git ${side}"]`)
      .waitForEnabled({ timeout: 30000 });
  };
  let fixture, base, changed;
  const git = async (args) => {
    const { execFileSync } = await import("node:child_process");
    const { dirname } = await import("node:path");
    return execFileSync(
      "git",
      [
        "-C",
        dirname(fixture.path),
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "protocol.allow=never",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "maintenance.auto=false",
        "-c",
        "gc.auto=0",
        ...args,
      ],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          GIT_CONFIG_NOSYSTEM: "1",
          GIT_CONFIG_GLOBAL: "/dev/null",
          GIT_TERMINAL_PROMPT: "0",
          GIT_ALLOW_PROTOCOL: "",
        },
      },
    );
  };
  async function closeTabs() {
    if (await panel().isDisplayed()) await panel().$("button=Close").click();
    let serial = 0;
    while (true) {
      const selector = '.tabbar .tab button[aria-label^="Close "]';
      const count = await browser.$$(selector).length;
      if (
        !count ||
        (count === 1 &&
          (await browser
            .$('.tabbar .tab button[aria-label="Close New tab"]')
            .isExisting()))
      )
        break;
      await browser
        .$('.document-tab:not([hidden]) .window-shell[data-busy="false"]')
        .waitForExist({ timeout: 30000 });
      const marker = `git-closing-${Date.now()}-${serial++}`;
      await browser.tauri.execute(
        (_api, marker) =>
          document
            .querySelector(".tabbar .tab")
            .setAttribute("data-git-closing", marker),
        marker,
      );
      const marked = () =>
        browser.$(`.tabbar .tab[data-git-closing="${marker}"]`);
      await marked()
        .$('button[aria-label^="Close "]')
        .waitForEnabled({ timeout: 30000 });
      await marked().$('button[aria-label^="Close "]').click();
      await browser.waitUntil(
        async () =>
          (await browser
            .$('dialog[open] [id$="-discard-title"]')
            .isDisplayed()) || !(await marked().isExisting()),
        { timeout: 30000 },
      );
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      await marked().waitForExist({ reverse: true, timeout: 30000 });
    }
  }
  async function seed(large = false) {
    await closeTabs();
    base = large
      ? Array.from({ length: 10000 }, (_, i) => `const row${i} = ${i};\n`).join(
          "",
        )
      : "\ufeff" +
        Array.from(
          { length: 50 },
          (_, i) =>
            `Line ${String(i).padStart(2, "0")} العربية 文 🦀${i % 3 === 0 ? "\r\n" : i % 3 === 1 ? "\n" : "\r"}`,
        ).join("");
    fixture = await call("e2e_fixture", { action: "seed", text: base });
    await git(["init", "-b", "main"]);
    await git(["config", "user.name", "Native Test"]);
    await git(["config", "user.email", "native@example.invalid"]);
    await git(["config", "core.autocrlf", "false"]);
    const fs = await import("node:fs");
    const path = await import("node:path");
    if (large) {
      fixture.path = path.join(path.dirname(fixture.path), "code.ts");
      fs.writeFileSync(fixture.path, base);
    }
    await git(["add", "--", path.basename(fixture.path)]);
    await git(["commit", "-m", "Native baseline"]);
    changed = large
      ? base.replaceAll(" = ", " = edited + ")
      : base
          .replace("Line 01", "First changed")
          .replace("Line 45", "Second changed");
    fs.writeFileSync(fixture.path, changed);
    await browser.$(".workspace-actions").$("button=Git").click();
    await panel().waitForDisplayed();
    await panel()
      .$("button=Choose repository")
      .waitForEnabled({ timeout: 30000 });
    await panel().$("button=Choose repository").click();
    await browser.waitUntil(
      async () =>
        (await panel().$(".git-heading p").getText()).startsWith(
          path.basename(path.dirname(fixture.path)),
        ) && (await panel().$("button=Choose repository").isEnabled()),
      {
        timeout: 30000,
        timeoutMsg: "New native repository selection must settle",
      },
    );
    await panel().$(".git-files button").waitForDisplayed({ timeout: 30000 });
    await panel().$(".git-files button").waitForEnabled({ timeout: 30000 });
    await panel().$(".git-files button").click();
    await panel()
      .$(".git-diff-title strong")
      .waitForDisplayed({ timeout: 30000 });
    await panel().$(".git-row").waitForExist({ timeout: 30000 });
  }
  it("stages only selected native hunks, unstages against HEAD and refuses live Source mutation", async () => {
    await seed();
    const fs = await import("node:fs");
    const choosingAtClose = await browser.tauri.execute(async () => {
      const workbench = document.querySelector("dialog.git-workbench");
      const choose = workbench.querySelector("button.primary");
      choose.click();
      await Promise.resolve();
      const pending = choose.disabled;
      workbench
        .querySelector('button[aria-label="Close Git workbench"]')
        .click();
      await Promise.resolve();
      return { pending, stillBusy: choose.disabled };
    });
    expect(choosingAtClose).toEqual({ pending: true, stillBusy: true });
    await browser.$(".workspace-actions").$("button=Git").click();
    await panel()
      .$("button=Choose repository")
      .waitForEnabled({ timeout: 30000 });
    await panel().$(".git-files button").waitForEnabled({ timeout: 30000 });
    const loadingAtClose = await browser.tauri.execute(async () => {
      const workbench = document.querySelector("dialog.git-workbench");
      workbench.querySelector(".git-files button").click();
      await Promise.resolve();
      const loading = workbench.querySelector("button.primary").disabled;
      workbench
        .querySelector('button[aria-label="Close Git workbench"]')
        .click();
      return loading;
    });
    expect(loadingAtClose).toBe(true);
    await browser.$(".workspace-actions").$("button=Git").click();
    await panel()
      .$("button=Choose repository")
      .waitForEnabled({ timeout: 30000 });
    await panel().$(".git-files button").click();
    await panel()
      .$(".git-diff-title strong")
      .waitForDisplayed({ timeout: 30000 });
    await panel().$(".git-row").waitForExist({ timeout: 30000 });
    expect(await panel().$$(".git-hunk-list input").length).toBe(2);
    await panel().$('input[aria-label="Select hunk 1"]').click();
    await panel()
      .$("button=Stage selected hunks")
      .waitForEnabled({ timeout: 30000 });
    await panel().$("button=Stage selected hunks").click();
    await browser.waitUntil(
      async () =>
        (await git(["show", ":source.txt"])) ===
        base.replace("Line 01", "First changed"),
      { timeout: 30000 },
    );
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(changed);
    await panel()
      .$("button=Stage selected hunks")
      .waitForEnabled({ reverse: true });
    await panel()
      .$('select[aria-label="Git before"]')
      .waitForEnabled({ timeout: 30000 });
    await baseline("before", "head");
    await panel()
      .$('select[aria-label="Git after"]')
      .waitForEnabled({ timeout: 30000 });
    await baseline("after", "index");
    await panel().$(".git-hunk-list input").waitForExist();
    await panel().$("button=Select all hunks").click();
    await panel().$("button=Unstage selected hunks").waitForEnabled();
    await panel().$("button=Unstage selected hunks").click();
    await browser.waitUntil(
      async () => (await git(["show", ":source.txt"])) === base,
      { timeout: 30000 },
    );
    await panel()
      .$('select[aria-label="Git before"]')
      .waitForEnabled({ timeout: 30000 });
    await baseline("before", "index");
    await panel()
      .$('select[aria-label="Git after"]')
      .waitForEnabled({ timeout: 30000 });
    await baseline("after", "worktree");
    await panel().$(".git-line-number").waitForEnabled();
    await panel().$(".git-line-number").click();
    await browser
      .$(".document-tab:not([hidden]) .cm-content")
      .waitForDisplayed({ timeout: 30000 });
    await browser.$(".workspace-actions").$("button=Git").click();
    await panel().$("button=Stage selected hunks").waitForExist();
    expect(await panel().$("button=Stage selected hunks").isEnabled()).toBe(
      false,
    );
    const rejected = await browser.tauri.execute(async ({ core }) => {
      try {
        await core.invoke("git_mutate", {
          id: "forged",
          preview: "forged",
          action: "stage",
          hunks: ["forged"],
        });
        return false;
      } catch (e) {
        return e.code === "editing-active";
      }
    });
    expect(rejected).toBe(true);
    await browser.saveScreenshot("artifacts/git-source-gate.png");
    await panel().$("button=Close").click();
  });
  it("confirms discard, preserves exact recovery bytes and exports only to a new native destination", async () => {
    await seed();
    const fs = await import("node:fs");
    const path = await import("node:path");
    await panel().$("button=Discard file changes…").click();
    const confirm = await browser.$('dialog[aria-label="Confirm Git discard"]');
    await confirm.waitForDisplayed();
    await confirm.$("button=Keep changes").click();
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(changed);
    await panel().$("button=Discard file changes…").click();
    await confirm.$("button=Discard and keep recovery copy").click();
    await browser.waitUntil(
      () => fs.readFileSync(fixture.path, "utf8") === base,
      { timeout: 30000 },
    );
    await panel().$("button=Recovery 1").click();
    await panel().$(".git-backup strong").waitForDisplayed();
    await call("e2e_fixture", { action: "save-as", text: null });
    await panel().$("button=Export recovery copy").click();
    const copy = path.join(path.dirname(fixture.path), "copy.txt");
    await browser.waitUntil(() => fs.existsSync(copy), { timeout: 30000 });
    expect(fs.readFileSync(copy, "utf8")).toBe(changed);
    await browser.saveScreenshot("artifacts/git-recovery.png");
    await panel().$("button=Close").click();
  });
  it("keeps a real 10k-line diff bounded through split, intraline, history and scoped Library badges", async () => {
    const start = Date.now();
    await seed(true);
    const initial = await browser.tauri.execute(() => ({
      rows: document.querySelectorAll(".git-row").length,
      nodes: document
        .querySelector("dialog.git-workbench")
        .querySelectorAll("*").length,
      total: document.querySelector(".git-canvas").style.height,
    }));
    expect(initial.rows).toBeLessThanOrEqual(200);
    expect(initial.nodes).toBeLessThan(5000);
    await panel().$("button=Side by side").click();
    await panel().$(".git-cell.old").waitForExist();
    await browser.waitUntil(async () => await panel().$("mark").isExisting());
    await browser.tauri.execute(() => {
      const v = document.querySelector(".git-viewport");
      v.scrollTop = v.scrollHeight;
      v.dispatchEvent(new Event("scroll"));
    });
    await browser.waitUntil(
      async () =>
        await browser.tauri.execute(() =>
          [...document.querySelectorAll(".git-row .git-line-number")].some(
            (n) => Number(n.textContent) > 9900,
          ),
        ),
      { timeout: 30000 },
    );
    await writeFile(
      "artifacts/git-diff-10k.json",
      JSON.stringify(
        {
          ...initial,
          setupAndDiffInteractionsMs: Date.now() - start,
          profile: `debug ${nativeWebView}; includes fixture generation/commit, capture, split/intraline and scroll assertions; not input-to-paint`,
        },
        null,
        2,
      ),
    );
    await browser.saveScreenshot("artifacts/git-diff-split.png");
    const measured = spawnSync(
      "python3",
      [
        "benchmarks/source_memory.py",
        "--pid",
        String(fixture.pid),
        "--label",
        "git-10k-warm",
      ],
      { encoding: "utf8" },
    );
    expect(measured.status).toBe(0);
    await panel().$("button=Previous hunk").waitForExist();
    await panel().$("button=History").click();
    await panel().$(".git-commit").click();
    await panel().$(".git-files button").click();
    await panel().$(".git-diff-title").waitForDisplayed();
    expect(await panel().$("button=Stage selected hunks").isEnabled()).toBe(
      false,
    );
    await browser.saveScreenshot("artifacts/git-history.png");
    await panel().$("button=Close").click();
    if (!(await browser.$(".library-host").isDisplayed()))
      await browser.$("button=Library").click();
    await browser.$("button=Add folder").click();
    await browser.$(".library-git-badge").waitForExist({ timeout: 30000 });
    expect(
      await browser.$(".library-git-badge").getAttribute("title"),
    ).toContain("modified");
    await browser.$(".workspace-actions").$("button=Git").click();
    await panel().$("button=Revoke repository").click();
    await expect(panel()).toHaveText(
      expect.stringContaining("Repository access revoked for this session."),
    );
    await expect(panel()).not.toHaveText(
      expect.stringContaining("Repository authorized for this session."),
    );
    await panel().$("button=Close").click();
    await browser.$(".library-git-badge").waitForExist({ reverse: true });
  });
});

describe("Document review", () => {
  const pane = () => browser.$(".document-tab:not([hidden])");
  const panel = () => pane().$('aside[aria-label="Document review"]');
  const call = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  let fixture;
  const content =
    "\uFEFF# Review 文 🦀\r\n\r\nParagraph to discuss.\r\n\r\nA second paragraph.\n";
  const hash = async (text) =>
    (await import("node:crypto"))
      .createHash("sha256")
      .update(text)
      .digest("hex");
  async function closeTabs() {
    const git = browser.$('dialog[aria-label="Git workbench"]');
    if (await git.isDisplayed()) await git.$("button=Close").click();
    const archive = browser.$('dialog[aria-label="Stored document reviews"]');
    if (await archive.isDisplayed())
      await archive.$("button=Close stored reviews").click();
    let serial = 0;
    while (true) {
      const selector = '.tabbar .tab button[aria-label^="Close "]';
      const count = await browser.$$(selector).length;
      if (
        !count ||
        (count === 1 &&
          (await browser
            .$('.tabbar .tab button[aria-label="Close New tab"]')
            .isExisting()))
      )
        break;
      await pane()
        .$('.window-shell[data-busy="false"]')
        .waitForExist({ timeout: 30000 });
      const marker = `closing-${Date.now()}-${serial++}`;
      await browser.tauri.execute(
        (_api, marker) =>
          document
            .querySelector(".tabbar .tab")
            .setAttribute("data-review-closing", marker),
        marker,
      );
      const marked = () =>
        browser.$(`.tabbar .tab[data-review-closing="${marker}"]`);
      await marked().$('button[aria-label^="Close "]').click();
      await browser.waitUntil(
        async () =>
          (await browser
            .$('dialog[open] [id$="-discard-title"]')
            .isDisplayed()) || !(await marked().isExisting()),
        { timeout: 30000 },
      );
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      const draft = browser.$('dialog[aria-label="Unsaved review draft"]');
      if (await draft.isDisplayed())
        await draft.$("button=Discard review draft and leave").click();
      await marked().waitForExist({ reverse: true, timeout: 30000 });
    }
  }

  async function seed(text = content) {
    await closeTabs();
    fixture = await call("e2e_fixture", { action: "seed", text });
    await pane().$("button.open-button").waitForEnabled({ timeout: 30000 });
    await pane().$("button.open-button").click();
    await pane().$("button.open-button").waitForEnabled({ timeout: 30000 });
    await pane()
      .$('.window-shell[data-busy="false"]')
      .waitForExist({ timeout: 30000 });
    expect(
      (await call("e2e_fixture", { action: "read", text: null })).path,
    ).toBe(fixture.path);
    await pane().$("article").waitForDisplayed({ timeout: 30000 });
    const focus = pane().$("button=Exit focus");
    if (await focus.isDisplayed()) await focus.click();
  }
  async function enable() {
    await pane().$("button=Review").waitForEnabled({ timeout: 30000 });
    await pane().$("button=Review").click();
    await panel().waitForExist({ timeout: 30000 });
    await panel().$(".review-baseline").waitForDisplayed({ timeout: 30000 });
    await panel().$("button=Capture disk").waitForEnabled({ timeout: 30000 });
  }
  async function selectBlock(text) {
    await panel().$("button*=Comments").click();
    await browser.tauri.execute((_api, text) => {
      const select = document.querySelector(
        '.document-tab:not([hidden]) select[aria-label="Comment block"]',
      );
      const option = [...select.options].find((o) =>
        o.textContent.includes(text),
      );
      if (!option) throw Error("Missing block");
      select.value = option.value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    }, text);
  }
  async function replace(text) {
    const fs = await import("node:fs");
    const file = fixture.path + ".next";
    fs.writeFileSync(file, text);
    fs.renameSync(file, fixture.path);
  }
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      await browser.saveScreenshot("artifacts/review-failure.png");
      if (await panel().isExisting())
        console.error(
          "REVIEW FAILURE STATE",
          (await panel().getText()).slice(0, 3000),
        );
    }
  });
  it("keeps review opt-in, verifies comments and copies/exports consent-scoped portable feedback", async () => {
    await seed();
    expect(await pane().$(".review-margin").isExisting()).toBe(false);
    expect(await call("review_list")).toEqual([]);
    await enable();
    expect(await panel().getText()).toContain(
      "without an earlier reviewed baseline",
    );
    await selectBlock("Paragraph to discuss");
    await panel()
      .$('textarea[aria-label="Review comment"]')
      .setValue("Please clarify 🦀.");
    await panel().$("button=Turn Review off").click();
    const draft = browser.$('dialog[aria-label="Unsaved review draft"]');
    await draft.waitForDisplayed();
    await draft.$("button=Keep editing review draft").click();
    expect(
      await panel().$('textarea[aria-label="Review comment"]').getValue(),
    ).toBe("Please clarify 🦀.");
    await call("e2e_fixture", { action: "open", text: null });
    const otherDoc = await call("choose_document");
    const other = await call("review_begin", { id: otherDoc.id });
    await call("review_update", {
      token: other.token,
      id: otherDoc.id,
      version: other.version,
      revision: other.currentHash,
      action: { kind: "decision", value: "changes_requested" },
    });
    await panel().$("button=Save comment").click();
    await browser.waitUntil(async () =>
      (await panel().getText()).includes("changed in another view"),
    );
    expect(
      await panel().$('textarea[aria-label="Review comment"]').getValue(),
    ).toBe("Please clarify 🦀.");
    await call("review_end", { token: other.token });
    await call("release_document", { id: otherDoc.id });
    await panel().$("button=Refresh review").click();
    await panel().$("button=Capture disk").waitForEnabled();
    await selectBlock("Paragraph to discuss");
    expect(
      await panel().$('textarea[aria-label="Review comment"]').getValue(),
    ).toBe("Please clarify 🦀.");
    await panel().$("button=Save comment").click();
    await panel().$(".review-thread").waitForDisplayed();
    expect(
      await panel().$(".review-thread").getAttribute("data-anchor-status"),
    ).toBe("anchored");
    await panel()
      .$('input[aria-label="Reply to comment"]')
      .setValue("An anchored reply.");
    await panel().$("button=Add reply").click();
    await browser.waitUntil(async () =>
      (await panel().$(".review-thread").getText()).includes(
        "An anchored reply.",
      ),
    );
    await panel().$("button=Edit comment").click();
    await panel()
      .$('textarea[aria-label="Review comment"]')
      .setValue("Clarify this saved paragraph.");
    await panel().$("button=Save comment").click();
    await browser.waitUntil(async () =>
      (await panel().$(".review-thread").getText()).includes(
        "Clarify this saved paragraph.",
      ),
    );
    await panel().$("button=Approve capture").click();
    await browser.waitUntil(async () =>
      (await panel().getText()).includes("Decision: approved"),
    );
    await panel().$("button=Feedback").click();
    await panel().$("button=Prepare feedback").click();
    const output = panel().$('textarea[aria-label="Prepared review feedback"]');
    await output.waitForDisplayed();
    const report = JSON.parse(await output.getValue());
    expect(report.decision).toBe("approved");
    expect(report.decisionRevision).toBe(report.currentRevision);
    expect(report.file).toBe(null);
    expect(report.snippetsIncluded).toBe(false);
    expect(report.comments[0].anchor.quote).toBeUndefined();
    expect(report.comments[0].currentAnchor.status).toBe("anchored");
    expect(report.currentRevision).toBe(await hash(content));
    if (process.platform === "darwin") {
      const { execFileSync } = await import("node:child_process");
      const previous = execFileSync("pbpaste");
      try {
        await panel().$("button=Copy prepared feedback").click();
        expect(
          JSON.parse(execFileSync("pbpaste", { encoding: "utf8" }))
            .currentRevision,
        ).toBe(report.currentRevision);
      } finally {
        execFileSync("pbcopy", { input: previous });
      }
    }
    const checks = await panel().$$('input[type="checkbox"]');
    await checks[0].click();
    await checks[1].click();
    const destination = await call("e2e_fixture", {
      action: "save-as",
      text: null,
    });
    expect(destination.path).toBe(fixture.path);
    await panel().$("button=Export feedback…").click();
    const fs = await import("node:fs");
    const path = await import("node:path");
    const copy = path.join(path.dirname(fixture.path), "copy.txt");
    await browser.waitUntil(() => fs.existsSync(copy), { timeout: 30000 });
    const saved = JSON.parse(fs.readFileSync(copy, "utf8"));
    expect(saved.file).toBe(fs.realpathSync(fixture.path));
    expect(saved.comments[0].anchor.quote).toContain("Paragraph to discuss");
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(content);
    await browser.saveScreenshot("artifacts/review-feedback.png");
    const measured = spawnSync(
      "python3",
      [
        "benchmarks/source_memory.py",
        "--pid",
        String(fixture.pid),
        "--label",
        "review-small",
      ],
      { encoding: "utf8" },
    );
    expect(measured.status).toBe(0);
    const geometry = await browser.tauri.execute(() => {
      const p = document.querySelector(
          ".document-tab:not([hidden]) .window-shell",
        ),
        r = p.querySelector(".review-margin");
      return {
        documentWidth: p.clientWidth,
        reviewWidth: r.clientWidth,
        reviewNodes: r.querySelectorAll("*").length,
        bodyOverflow: document.documentElement.scrollWidth > innerWidth,
      };
    });
    expect(geometry.bodyOverflow).toBe(false);
    await writeFile(
      "artifacts/review-review-layout.json",
      JSON.stringify(geometry, null, 2),
    );
    await panel().$("button=Manage stored reviews").click();
    const archive = browser.$('dialog[aria-label="Stored document reviews"]');
    await archive.waitForDisplayed();
    await archive.$("button=Prepare archived feedback").click();
    const archived = archive.$(
      'textarea[aria-label="Archived review feedback"]',
    );
    await archived.waitForDisplayed();
    const data = JSON.parse(await archived.getValue());
    expect(data.file).toBe(null);
    expect(data.comments[0].currentAnchor.status).toBe("not_evaluated");
    await archive.$("button=Delete stored review…").click();
    const confirm = browser.$('dialog[aria-label="Confirm review deletion"]');
    await confirm.waitForDisplayed();
    await confirm.$("button=Keep review data").click();
    expect((await call("review_list")).length).toBe(1);
    await archive.$("button=Delete stored review…").click();
    await confirm.$("button=Delete review data").click();
    await browser.waitUntil(
      async () => (await call("review_list")).length === 0,
    );
    await archive.$("button=Close stored reviews").click();
    await panel().waitForExist({ reverse: true });
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(content);
  });
  it("freezes displayed/reviewed baselines across replacements and preserves dirty Source against new disk bytes", async () => {
    await seed();
    await enable();
    await panel().$("button=Turn Review off").click();
    await panel().waitForExist({ reverse: true });
    const v2 = content.replace(
      "Paragraph to discuss.",
      "External version two.",
    );
    await replace(v2);
    await browser.waitUntil(
      async () =>
        (await pane().$("article").getText()).includes("External version two."),
      { timeout: 30000 },
    );
    await enable();
    const v1hash = await hash(content),
      v2hash = await hash(v2);
    expect(await panel().$(".review-baseline").getText()).toContain(
      "Previous displayed " + v1hash.slice(0, 12),
    );
    expect(await panel().$(".review-baseline").getText()).toContain(
      "Last reviewed " + v1hash.slice(0, 12),
    );
    await panel().$("button=Mark saved capture reviewed").click();
    await browser.waitUntil(async () =>
      (await panel().$(".review-baseline").getText()).includes(
        "Last reviewed " + v2hash.slice(0, 12),
      ),
    );
    const v3 = v2.replace("External version two.", "External version three.");
    await replace(v3);
    await browser.waitUntil(
      async () =>
        (await panel().$(".review-baseline").getText()).includes(
          "Current saved " + (await hash(v3)).slice(0, 12),
        ),
      { timeout: 30000 },
    );
    expect(await panel().$(".review-baseline").getText()).toContain(
      "Previous displayed " + v1hash.slice(0, 12),
    );
    expect(await panel().$(".review-baseline").getText()).toContain(
      "Last reviewed " + v2hash.slice(0, 12),
    );
    await panel().$("button=Saved source diff").click();
    await panel().$(".review-source-diff").waitForDisplayed();
    expect(await panel().$(".review-source-diff").getText()).toContain(
      "External version three.",
    );
    // Capture disk can finish before the saved Read grant reload. Wait for the
    // ordinary UI reservation, then exercise Source against that saved revision.
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await browser.waitUntil(
      async () =>
        (await pane().$("article").getText()).includes(
          "External version three.",
        ),
      { timeout: 60000 },
    );
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    // Dispatch both handlers in the same task. Reload must reserve synchronously,
    // before its leave-decision promise yields, so Source cannot start mid-reload.
    await browser.tauri.execute(() => {
      const p = document.querySelector(".document-tab:not([hidden])");
      const buttons = [...p.querySelectorAll("button")];
      buttons.find((b) => b.textContent.trim() === "Reload").click();
      buttons.find((b) => b.textContent.trim() === "Source").click();
    });
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    expect(await pane().$(".window-shell").getAttribute("data-editing")).toBe(
      "false",
    );
    expect(await pane().$("button=Read").getAttribute("aria-pressed")).toBe(
      "true",
    );
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await browser.tauri.execute(() => {
      const v = document.querySelector(
        ".document-tab:not([hidden]) .cm-content",
      ).cmTile.root.view;
      v.dispatch({
        changes: { from: v.state.doc.length, insert: "\nHuman unsaved 🦀." },
      });
    });
    const v4 = v3.replace("External version three.", "External version four.");
    await replace(v4);
    await browser.waitUntil(
      async () =>
        (await panel().$(".review-baseline").getText()).includes(
          "Current saved " + (await hash(v4)).slice(0, 12),
        ),
      { timeout: 30000 },
    );
    const local = await browser.tauri.execute(() =>
      document
        .querySelector(".document-tab:not([hidden]) .cm-content")
        .cmTile.root.view.state.doc.toString(),
    );
    expect(local).toContain("Human unsaved 🦀.");
    expect(local).toContain("External version three.");
    expect(local).not.toContain("External version four.");
    await panel().$("button=Compare editor and disk").click();
    await pane().$(".compare-dialog").waitForDisplayed();
    expect(await pane().$(".compare-dialog").getText()).toContain(
      "Human unsaved",
    );
    expect(await pane().$(".compare-dialog").getText()).toContain(
      "External version four.",
    );
    await pane().$(".compare-dialog").$("button=Keep local").click();
    await pane()
      .$(".toolbar")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane().$(".toolbar").$("button=Save").click();
    const fs = await import("node:fs");
    await browser.waitUntil(
      async () =>
        (await pane().$(".notice").getText()).toLowerCase().includes("changed"),
      { timeout: 10000 },
    );
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(v4);
    expect(
      await browser.tauri.execute(() =>
        document
          .querySelector(".document-tab:not([hidden]) .cm-content")
          .cmTile.root.view.state.doc.toString(),
      ),
    ).toContain("Human unsaved");
    await browser.saveScreenshot("artifacts/review-dirty-conflict.png");
  });
  it("recognizes a same-parent rename without reauthorizing a path and denies forged review grants", async () => {
    await seed();
    await enable();
    const fs = await import("node:fs");
    const path = await import("node:path");
    const renamed = path.join(path.dirname(fixture.path), "renamed-review.txt");
    fs.renameSync(fixture.path, renamed);
    await panel().$("button=Capture disk").click();
    await browser.waitUntil(
      async () => (await panel().getText()).includes("renamed-review.txt"),
      { timeout: 30000 },
    );
    expect(await pane().$("article").getText()).toContain(
      "Paragraph to discuss.",
    );
    const denied = await browser.tauri.execute(async ({ core }) => {
      let n = 0;
      for (const [command, args] of [
        ["review_begin", { id: "forged" }],
        [
          "review_export",
          { token: "forged", id: "forged", includePath: true, snippets: true },
        ],
        ["review_archive_export", { record: "../../metadata", snippets: true }],
      ]) {
        try {
          await core.invoke(command, args);
        } catch {
          n++;
        }
      }
      return n;
    });
    expect(denied).toBe(3);
    expect(fs.readFileSync(renamed, "utf8")).toBe(content);
    await browser.saveScreenshot("artifacts/review-rename.png");
  });
});

describe("Export and local jobs", () => {
  const pane = () => browser.$(".document-tab:not([hidden])");
  const panel = () => pane().$('dialog[aria-label="Export and local jobs"]');
  const call = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  let fixture;
  const choose = async (role, path) =>
    call("e2e_fixture", {
      action: "power-pick",
      text: JSON.stringify({ role, path }),
    });
  const consent = async (yes = true) =>
    call("e2e_fixture", { action: "power-confirm", text: yes ? "yes" : "no" });
  async function closeTabs() {
    if (await panel().isDisplayed())
      await panel().$("button=Close export and jobs").click();
    const oldReview = browser.$('dialog[aria-label="Stored document reviews"]');
    if (await oldReview.isDisplayed())
      await oldReview.$("button=Close stored reviews").click();
    let serial = 0;
    while (true) {
      const all = await browser.$$('.tabbar .tab button[aria-label^="Close "]');
      if (
        !all.length ||
        (all.length === 1 &&
          (await browser
            .$('.tabbar .tab button[aria-label="Close New tab"]')
            .isExisting()))
      )
        break;
      await pane()
        .$('.window-shell[data-busy="false"]')
        .waitForExist({ timeout: 60000 });
      const key = `power-close-${serial++}-${Date.now()}`;
      await browser.tauri.execute(
        (_api, key) =>
          document
            .querySelector(".tabbar .tab")
            .setAttribute("data-power-close", key),
        key,
      );
      const tab = () => browser.$(`.tabbar .tab[data-power-close="${key}"]`);
      await tab().$('button[aria-label^="Close "]').click();
      await browser.waitUntil(
        async () =>
          (await browser
            .$('dialog[open] [id$="-discard-title"]')
            .isDisplayed()) ||
          (await browser
            .$('dialog[aria-label="Unsaved review draft"]')
            .isDisplayed()) ||
          !(await tab().isExisting()),
        { timeout: 60000 },
      );
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      const draft = browser.$('dialog[aria-label="Unsaved review draft"]');
      if (await draft.isDisplayed())
        await draft.$("button=Discard review draft and leave").click();
      await tab().waitForExist({ reverse: true, timeout: 60000 });
    }
  }
  async function seed(text) {
    await closeTabs();
    fixture = await call("e2e_fixture", { action: "seed", text });
    await pane().$("button.open-button").waitForEnabled({ timeout: 60000 });
    await pane().$("button.open-button").click();
    await pane()
      .$('.window-shell[data-busy="false"]')
      .waitForExist({ timeout: 60000 });
    await pane().$("article").waitForDisplayed({ timeout: 60000 });
    for (const kind of ["shell", "sqlite", "tex"])
      await call("power_enable", { kind, on: false });
    await call("power_clear");
  }
  async function open() {
    await documentAction("Export & run");
    await panel().waitForDisplayed();
    await panel().$(".power-hash").waitForDisplayed({ timeout: 60000 });
  }
  async function tool(role, path, label) {
    await choose("tool-" + role, path);
    await consent();
    await panel().$(`button=${label}`).click();
    await panel()
      .$("button=Close export and jobs")
      .waitForEnabled({ timeout: 60000 });
    expect((await call("power_state")).tools.some((t) => t.role === role)).toBe(
      true,
    );
  }
  async function select(selector, value) {
    await browser.tauri.execute(
      (_api, selector, value) => {
        const s = document.querySelector(
          ".document-tab:not([hidden]) " + selector,
        );
        const o = [...s.options].find(
          (o) => o.value === value || o.textContent.includes(value),
        );
        if (!o) throw Error("Missing option " + value);
        s.value = o.value;
        s.dispatchEvent(new Event("change", { bubbles: true }));
      },
      selector,
      value,
    );
  }
  async function finish() {
    await browser.waitUntil(
      async () =>
        (await panel()
          .$('section[aria-label="Local job result"] h3')
          .isExisting()) &&
        (await panel()
          .$('section[aria-label="Local job result"] h3')
          .getText()) !== "running",
      { timeout: 90000 },
    );
    return panel().$('section[aria-label="Local job result"]').getText();
  }
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      await browser.saveScreenshot("artifacts/export-native-failure.png");
      if (await panel().isExisting())
        console.error("POWER FAILURE", (await panel().getText()).slice(-5000));
    }
  });
  it("keeps execution off, converts a selected saved revision with approved images/diagrams and exports an exclusive real PDF", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const { execFileSync } = await import("node:child_process");
    const source =
      "# Native PDF\n\nEnglish, العربية, 中文 and $E=mc^2$.\n\n$\\eqref{later}$\n\n$$\\gdef\\nativevalue{q}$$\n\n$$\\begin{equation}\\nativevalue=x\\label{later}\\end{equation}$$\n\n$$\\int_0^1 x^2\\,dx=\\frac{1}{3}$$\n\n| Item | Value |\n|---|---|\n| Test | 42 |\n\nA note.[^one]\n\n[^one]: Exported footnote.\n\n![Approved](selected.png){width=120px height=60px}\n\n![No grant](http://127.0.0.1:9/no-fetch)\n\n```mermaid\ngraph TD\nRead-->Review\n```\n\n```sh\nprintf NEVER_AUTOMATIC > never-automatic.txt\n```\n";
    await seed(source);
    const folder = path.dirname(fixture.path);
    await open();
    const geometry = await browser.tauri.execute(() => {
      const p = document.querySelector(
        ".document-tab:not([hidden]) .power-panel",
      );
      return {
        width: p.clientWidth,
        viewport: innerWidth,
        overflow: p.scrollWidth > p.clientWidth,
      };
    });
    expect(geometry.width).toBeGreaterThan(
      Math.min(700, geometry.viewport * 0.8),
    );
    expect(geometry.overflow).toBe(false);
    await writeFile(
      "artifacts/export-layout.json",
      JSON.stringify(geometry, null, 2),
    );
    expect(await call("execution_status")).toBe(false);
    expect(fs.existsSync(path.join(folder, "never-automatic.txt"))).toBe(false);
    await tool("pandoc", installedTool("pandoc"), "Choose Pandoc…");
    await tool("typst", installedTool("typst"), "Choose Typst…");
    await choose("image", path.resolve("tests/fixtures/approved-image.png"));
    await panel().$(".power-image button").click();
    await panel().$("button=Prepare PDF…").waitForEnabled({ timeout: 60000 });
    await consent();
    await panel().$("button=Prepare PDF…").click();
    expect(await finish()).toContain("PDF ready");
    expect(await panel().getText()).not.toContain(
      "Document math remains source",
    );
    expect(await panel().getText()).not.toContain(
      "Context-dependent or unsafe math",
    );
    expect(await call("execution_status")).toBe(false);
    expect(fs.existsSync(path.join(folder, "never-automatic.txt"))).toBe(false);
    const output = path.join(folder, "marktub-export-export.pdf");
    await choose("pdf-destination", output);
    await panel().$("button=Export PDF…").click();
    await browser.waitUntil(async () => fs.existsSync(output), {
      timeout: 30000,
    });
    const bytes = fs.readFileSync(output);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");
    const text = execFileSync("pdftotext", [output, "-"], { encoding: "utf8" });
    expect(text).toContain("Exported footnote");
    expect(text).toContain("Native PDF");
    expect(text).not.toContain("PRIVATE_CANARY");
    fs.copyFileSync(output, "artifacts/export-webview-pdf.pdf");
    const info = execFileSync("pdfimages", ["-list", output], {
      encoding: "utf8",
    });
    await writeFile("artifacts/export-webview-pdf-images.txt", info);
    const imageRows = info
      .split("\n")
      .filter((line) => /^\s*\d+\s+\d+\s+image\b/.test(line));
    expect(imageRows.length).toBeGreaterThanOrEqual(7);
    // E=mc² and q=x with its equation tag must include their complete SVGs,
    // rather than the former first-inline-fragment / clipped table rasters.
    expect(Number(imageRows[0].trim().split(/\s+/)[3])).toBeGreaterThan(100);
    expect(Number(imageRows[3].trim().split(/\s+/)[3])).toBeGreaterThan(150);
    await panel().$("button=Open PDF to inspect / print").click();
    await panel().$("button=Close export and jobs").waitForEnabled();
    if (process.platform === "linux") {
      execFileSync(
        "/usr/bin/python3",
        ["scripts/close-test-viewer.py", output],
        {
          stdio: "inherit",
        },
      );
      await browser.waitUntil(
        () => browser.tauri.execute(() => document.hasFocus()),
        {
          timeout: 5000,
          timeoutMsg:
            "Wayland did not return focus after closing the PDF viewer",
        },
      );
    }
    await choose("pdf-destination", output);
    await panel().$("button=Export PDF…").click();
    await expect(panel().$('[role="alert"]')).toHaveText(
      expect.stringContaining("unused filename"),
    );
    expect(fs.readFileSync(output)).toEqual(bytes);
    await browser.saveScreenshot("artifacts/export-pdf-panel.png");
    await panel().$("button=Close export and jobs").click();
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(source);
  });
  it("requires enable, per-folder trust and single-use native confirmation for shell; cancellation and stale capture keep Source intact", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const source =
      "# Local actions\n\n```sh\nprintf confirmed > confirmed.txt\nprintf shell-ok\n```\n\n```sh\nsleep 20\nprintf too-late > canceled.txt\n```\n".replaceAll(
        "\n",
        "\r\n",
      );
    await seed(source);
    await open();
    await panel().$("button*=Document fences").click();
    expect(await panel().$("button=Run captured fence…").isEnabled()).toBe(
      false,
    );
    await consent(false);
    await panel().$("button=Enable shell").click();
    await expect(panel().$('[role="alert"]')).toHaveText(
      expect.stringContaining("canceled"),
    );
    expect((await call("power_state")).shell).toBe(false);
    await consent();
    await panel().$("button=Enable shell").click();
    await browser.waitUntil(async () => (await call("power_state")).shell);
    await choose("root-shell", path.dirname(fixture.path));
    await consent();
    await panel().$("button=Trust folder for shell…").click();
    await panel().$("button=Choose shell…").waitForEnabled();
    await tool("shell", "/bin/sh", "Choose shell…");
    await consent(false);
    await panel().$("button=Run captured fence…").click();
    await panel().$("button=Close export and jobs").waitForEnabled();
    expect(
      fs.existsSync(path.join(path.dirname(fixture.path), "confirmed.txt")),
    ).toBe(false);
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("shell-ok");
    expect(
      fs.readFileSync(
        path.join(path.dirname(fixture.path), "confirmed.txt"),
        "utf8",
      ),
    ).toBe("confirmed");
    const summary = (
      await call("e2e_fixture", {
        action: "power-read-confirmation",
        text: null,
      })
    ).text;
    expect(summary).toContain("printf confirmed");
    expect(summary).toContain("SHA-256");
    expect(summary).toContain("Working directory:");
    await call("e2e_fixture", { action: "open", text: null });
    const doc = await call("choose_document");
    const captured = await call("power_capture", {
      input: {
        document: doc.id,
        editor: null,
        generation: null,
        revision: null,
        saved: true,
      },
    });
    const state = await call("power_state");
    const request = {
      capture: captured.id,
      block: captured.blocks[0].id,
      root: state.roots.find((r) => r.kind === "shell").id,
      database: null,
      tool: state.tools.find((t) => t.role === "shell").id,
      pandoc: null,
      mode: "shell",
      toc: false,
    };
    const once = await call("power_prepare", { request });
    await consent(false);
    expect(await call("power_run", { token: once.token })).toBe(null);
    expect(
      await browser.tauri.execute(async ({ core }, token) => {
        try {
          await core.invoke("power_run", { token });
          return false;
        } catch {
          return true;
        }
      }, once.token),
    ).toBe(true);
    const stale = await call("power_prepare", { request });
    fs.writeFileSync(fixture.path, source + "\nExternal changed revision.\n");
    await consent();
    expect(
      await browser.tauri.execute(async ({ core }, token) => {
        try {
          await core.invoke("power_run", { token });
          return false;
        } catch {
          return true;
        }
      }, stale.token),
    ).toBe(true);
    fs.writeFileSync(fixture.path, source);
    await call("power_release", { capture: captured.id });
    await call("release_document", { id: doc.id });
    const options = await panel()
      .$('select[aria-label="Captured fence"]')
      .$$("option");
    const next = await options[1].getAttribute("value");
    await select('select[aria-label="Captured fence"]', next);
    await consent();
    await panel().$("button=Run captured fence…").click();
    await panel().$("button=Cancel job").waitForDisplayed();
    await panel().$("button=Cancel job").click();
    expect(await finish()).toContain("canceled");
    expect(
      fs.existsSync(path.join(path.dirname(fixture.path), "canceled.txt")),
    ).toBe(false);
    await panel().$("button=Close export and jobs").click();
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await browser.tauri.execute(() => {
      const v = document.querySelector(
        ".document-tab:not([hidden]) .cm-content",
      ).cmTile.root.view;
      v.dispatch({
        changes: {
          from: v.state.doc.length,
          insert: "\nHuman working revision.\n",
        },
      });
    });
    await open();
    await select('select[aria-label="Capture revision"]', "false");
    await panel().$("button=Capture selected revision").click();
    await panel()
      .$("button=Close export and jobs")
      .waitForEnabled({ timeout: 60000 });
    expect(await panel().$(".power-capture").getText()).toContain(
      "Working native revision",
    );
    expect(
      await panel().$('section[aria-label="Local job result"]').isExisting(),
    ).toBe(false);
    await panel().$("button*=Document fences").click();
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("shell-ok");
    await browser.tauri.execute(() => {
      const v = document.querySelector(
        ".document-tab:not([hidden]) .cm-content",
      ).cmTile.root.view;
      v.dispatch({
        changes: { from: v.state.doc.length, insert: "Late working change.\n" },
      });
    });
    await consent(false);
    await panel().$("button=Run captured fence…").click();
    await panel().$('[role="alert"]').waitForDisplayed({ timeout: 60000 });
    await expect(panel().$('[role="alert"]')).toHaveText(
      expect.stringContaining("revision"),
    );
    await browser.saveScreenshot("artifacts/export-shell-panel.png");
    await panel().$("button=Close export and jobs").click();
    expect(await pane().$(".cm-content").getText()).toContain(
      "Human working revision",
    );
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(source);
  });
  it("uses selected SQLite readonly by default, commits confirmed DML and rolls back canceled writes; trusted TeX remains independent", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const { execFileSync } = await import("node:child_process");
    const source =
      "# Selected database\n\n```sql\nSELECT n FROM items;\n```\n\n```sql\nUPDATE items SET n=2;\n```\n\n```sql\nWITH RECURSIVE x(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM x WHERE n<100000000) UPDATE items SET n=(SELECT sum(n) FROM x);\n```\n\n```tex\n\\documentclass{article}\n\\begin{document}Trusted native TeX $\\frac{1}{2}$.\\end{document}\n```\n";
    await seed(source);
    const folder = path.dirname(fixture.path),
      db = path.join(folder, "selected.db");
    execFileSync("sqlite3", [
      db,
      "CREATE TABLE items(n INTEGER); INSERT INTO items VALUES(1);",
    ]);
    await open();
    await panel().$("button*=Document fences").click();
    await consent();
    await panel().$("button=Enable sqlite").click();
    await browser.waitUntil(async () => (await call("power_state")).sqlite);
    await choose("root-sqlite", folder);
    await consent();
    await panel().$("button=Trust folder for sqlite…").click();
    await panel().$("button=Choose SQLite database…").waitForEnabled();
    await choose("database", db);
    await panel().$("button=Choose SQLite database…").click();
    await panel().$("button=Close export and jobs").waitForEnabled();
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("1 rows");
    let options = await panel()
      .$('select[aria-label="Captured fence"]')
      .$$("option");
    await select(
      'select[aria-label="Captured fence"]',
      await options[1].getAttribute("value"),
    );
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("failed");
    expect(
      execFileSync("sqlite3", [db, "SELECT n FROM items;"], {
        encoding: "utf8",
      }).trim(),
    ).toBe("1");
    await panel().$('input[type="checkbox"]').click();
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("completed");
    expect(
      execFileSync("sqlite3", [db, "SELECT n FROM items;"], {
        encoding: "utf8",
      }).trim(),
    ).toBe("2");
    options = await panel()
      .$('select[aria-label="Captured fence"]')
      .$$("option");
    await select(
      'select[aria-label="Captured fence"]',
      await options[2].getAttribute("value"),
    );
    await consent();
    await panel().$("button=Run captured fence…").click();
    await panel().$("button=Cancel job").waitForDisplayed();
    await panel().$("button=Cancel job").click();
    expect(await finish()).toContain("canceled");
    expect(
      execFileSync("sqlite3", [db, "SELECT n FROM items;"], {
        encoding: "utf8",
      }).trim(),
    ).toBe("2");
    await consent();
    await panel().$("button=Enable tex").click();
    await browser.waitUntil(async () => (await call("power_state")).tex);
    await choose("root-tex", folder);
    await consent();
    await panel().$("button=Trust folder for tex…").click();
    await panel().$("button=Choose TeX engine…").waitForEnabled();
    await tool("tex", installedTool("tex", "xelatex"), "Choose TeX engine…");
    options = await panel()
      .$('select[aria-label="Captured fence"]')
      .$$("option");
    await select(
      'select[aria-label="Captured fence"]',
      await options[3].getAttribute("value"),
    );
    await consent();
    await panel().$("button=Run captured fence…").click();
    expect(await finish()).toContain("PDF ready");
    await browser.saveScreenshot("artifacts/export-sqlite-tex-panel.png");
    for (const command of [
      "power_run",
      "power_poll",
      "power_cancel",
      "power_open_pdf",
    ]) {
      const arg =
        command === "power_run"
          ? { token: "forged" }
          : command === "power_open_pdf"
            ? { export: "forged" }
            : { job: "forged" };
      const denied = await browser.tauri.execute(
        async ({ core }, command, arg) => {
          try {
            await core.invoke(command, arg);
            return false;
          } catch {
            return true;
          }
        },
        command,
        arg,
      );
      expect(denied).toBe(true);
    }
    await panel().$("button=Close export and jobs").click();
    expect(fs.readFileSync(fixture.path, "utf8")).toBe(source);
    for (const kind of ["shell", "sqlite", "tex"])
      await call("power_enable", { kind, on: false });
    expect(await call("execution_status")).toBe(false);
  });
});

describe("Everyday workflows", () => {
  const pane = () => browser.$(".document-tab:not([hidden])");
  const invoke = (command, args = {}) =>
    browser.tauri.execute(
      ({ core }, command, args) => core.invoke(command, args),
      command,
      args,
    );
  const selected = () => browser.$('.tabs [role="tab"][aria-selected="true"]');
  async function ready() {
    await browser.waitUntil(
      async () =>
        (await pane().$('.window-shell[data-busy="false"]').isExisting()) &&
        (await browser
          .$(
            '.tabs .tab:has([aria-selected="true"]) button[aria-label^="Close "]',
          )
          .isEnabled()),
      {
        timeout: 60000,
        timeoutMsg: "Active pane and shell close reservation must be ready",
      },
    );
  }
  const readWaits = [];
  async function readDocument(title) {
    const started = Date.now();
    await browser.waitUntil(
      async () => {
        const heading = pane().$("article h1");
        return (
          (await heading.isExisting()) && (await heading.getText()) === title
        );
      },
      { timeout: 60000, timeoutMsg: `Expected active document: ${title}` },
    );
    await ready();
    readWaits.push({ title, milliseconds: Date.now() - started });
  }
  async function closeDocuments() {
    for (const [label, close] of [
      ["Quick preview", "Close preview"],
      ["Quick open and search", "Close search"],
    ]) {
      const modal = browser.$(`dialog[aria-label="${label}"][open]`);
      if (await modal.isExisting()) await modal.$(`button=${close}`).click();
    }
    for (let i = 0; i < 17; i++) {
      await ready();
      const tabs = await browser.$$(".tabs .tab");
      if (
        tabs.length === 1 &&
        (await tabs[0].$('button[aria-label="Close New tab"]').isExisting())
      )
        break;
      await pane()
        .$('.window-shell[data-busy="false"]')
        .waitForExist({ timeout: 30000 });
      const tab = tabs[tabs.length - 1];
      const id = await tab.$('[role="tab"]').getAttribute("id");
      await tab
        .$('button[aria-label^="Close "]')
        .waitForEnabled({ timeout: 60000 });
      await tab.$('button[aria-label^="Close "]').click();
      await browser.waitUntil(
        async () =>
          !(await browser.$(`#${id}`).isExisting()) ||
          (await browser
            .$('dialog[open] [id$="-discard-title"]')
            .isDisplayed()),
        { timeout: 60000 },
      );
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
      await browser.waitUntil(
        async () => !(await browser.$(`#${id}`).isExisting()),
        { timeout: 60000 },
      );
    }
    if (await browser.$(".library-host").isDisplayed())
      await browser.$(".workspace-actions").$("button=Library").click();
  }
  function handoff(path) {
    const binary = desktopBinary("debug");
    const result = spawnSync(binary, [path], {
      encoding: "utf8",
      timeout: 15000,
    });
    expect(result.status).toBe(0);
  }
  async function openHomeFile() {
    await pane()
      .$('.window-shell[data-busy="false"]')
      .waitForExist({ timeout: 30000 });
    await pane()
      .$(".empty-actions")
      .$("button=Open file")
      .waitForEnabled({ timeout: 30000 });
    await pane().$(".empty-actions").$("button=Open file").click();
    await pane().$("article").waitForDisplayed({ timeout: 60000 });
    await ready();
  }
  afterEach(async function () {
    if (this.currentTest?.state === "failed") {
      const failure = this.currentTest.title.split(" ").slice(0, 4).join("-");
      await browser.saveScreenshot(
        `artifacts/workflow-native-failure-${failure}.png`,
      );
      await writeFile(
        `artifacts/workflow-native-failure-${failure}.json`,
        JSON.stringify(
          await browser.tauri.execute(() => ({
            text: document.body.innerText.slice(0, 10000),
            panes: [...document.querySelectorAll(".document-tab")].map((p) => ({
              hidden: p.hidden,
              editing: p.querySelector(".window-shell")?.dataset.editing,
              busy: p.querySelector(".window-shell")?.dataset.busy,
            })),
          })),
          null,
          2,
        ),
      );
    }
  });
  beforeEach(closeDocuments);
  after(async () => {
    await closeDocuments();
    await writeFile(
      "artifacts/workflow-readiness-waits.json",
      JSON.stringify(
        {
          scope:
            "debug native conditional document+reservation waits; not input/paint or production latency certification",
          observations: readWaits,
        },
        null,
        2,
      ),
    );
  });

  it("opens folders and recent files without restoring revoked path authority", async () => {
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Recent العربية\n",
    });
    await pane().$("button=Open folder").click();
    const rootName = dirname(fixture.path).split("/").at(-1);
    await browser
      .$(`button[aria-label="Remove root ${rootName}"]`)
      .waitForExist();
    await openHomeFile();
    await ready();
    await readDocument("Recent العربية");
    await ready();
    await browser
      .$('.tabs .tab:has([aria-selected="true"]) button[aria-label^="Close "]')
      .click();
    await ready();
    const recent = () => pane().$(".recent-files li button");
    await expect(recent()).toHaveText(expect.stringContaining("source.txt"));
    await expect(recent()).toHaveText(expect.stringContaining("Open"));
    await recent().waitForEnabled({ timeout: 30000 });
    await recent().click();
    await ready();
    await readDocument("Recent العربية");
    await ready();
    await browser
      .$('.tabs .tab:has([aria-selected="true"]) button[aria-label^="Close "]')
      .click();
    await ready();
    await browser.$(`button[aria-label="Remove root ${rootName}"]`).click();
    await expect(recent()).toHaveText(expect.stringContaining("Select folder"));
    const hint = (await invoke("library_state")).recent[0];
    expect(hint.authorized).toBe(false);
    const denied = await browser.tauri.execute(async ({ core }, id) => {
      try {
        await core.invoke("library_open", { id, recent: true });
        return false;
      } catch {
        return true;
      }
    }, hint.id);
    expect(denied).toBe(true);
    await recent().waitForEnabled({ timeout: 30000 });
    await recent().click();
    await browser
      .$(`button[aria-label="Remove root ${rootName}"]`)
      .waitForExist();
    await expect(pane().$(".empty-state h1")).toBeDisplayed();
    await expect(recent()).toHaveText(expect.stringContaining("Open"));
    await recent().waitForEnabled({ timeout: 30000 });
    await recent().click();
    await ready();
    await readDocument("Recent العربية");
    await ready();
    expect((await invoke("e2e_fixture", { action: "read" })).text).toBe(
      "# Recent العربية\n",
    );
  });

  it("distinguishes duplicate recent filenames using bounded native context", async () => {
    const fs = await import("node:fs/promises");
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Context root\n",
    });
    const base = dirname(fixture.path);
    const files = [
      join(base, "alpha", "same.txt"),
      join(base, "beta", "same.txt"),
    ];
    for (let i = 0; i < files.length; i++) {
      await fs.mkdir(dirname(files[i]), { recursive: true });
      await fs.writeFile(files[i], `# Context ${i}\n`);
      handoff(files[i]);
      await readDocument(`Context ${i}`);
    }
    const hints = (await invoke("library_state")).recent.filter(
      (hint) => hint.name === "same.txt",
    );
    expect(hints.length).toBe(2);
    expect(new Set(hints.map((hint) => hint.context)).size).toBe(2);
    expect(
      hints.every(
        (hint) => hint.context.length <= 120 && !hint.context.startsWith("/"),
      ),
    ).toBe(true);
    await closeDocuments();
    const contexts = await pane()
      .$$(".recent-files .recent-context")
      .map((element) => element.getText());
    expect(contexts.some((context) => context.endsWith("alpha"))).toBe(true);
    expect(contexts.some((context) => context.endsWith("beta"))).toBe(true);
  });

  it("opens sixteen real notes, reveals the selected menu entry and saves a copy at capacity", async () => {
    const fs = await import("node:fs/promises");
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Capacity fixture\n",
    });
    const root = dirname(fixture.path);
    const files = Array.from({ length: 16 }, (_, index) =>
      join(root, `note-${String(index).padStart(2, "0")}-文.md`),
    );
    const original = "# Note 0\r\n\r\nKeep native bytes.\n";
    await Promise.all(
      files.map((path, index) =>
        fs.writeFile(path, index ? `# Note ${index}\n` : original),
      ),
    );
    const handed = spawnSync(desktopBinary("debug"), files, {
      encoding: "utf8",
      timeout: 15000,
    });
    expect(handed.status).toBe(0);
    await readDocument("Note 15");
    await browser.waitUntil(
      async () => (await browser.$$(".tabs .tab")).length === 16,
      { timeout: 60000 },
    );
    await expect(browser.$('button[aria-label="New tab"]')).toBeDisabled();
    await expect(
      browser.$(".workspace-actions").$("button=New note"),
    ).toBeDisabled();
    await browser.$('[aria-label="Tab actions"]').click();
    await browser.waitUntil(() =>
      browser.tauri.execute(() => {
        const menu = document.querySelector(".tab-menu > div"),
          selected = menu.querySelector('button[aria-pressed="true"]');
        const outer = menu.getBoundingClientRect(),
          item = selected.getBoundingClientRect();
        return item.top >= outer.top - 1 && item.bottom <= outer.bottom + 1;
      }),
    );
    await browser.$(".tab-menu").$("button=note-00-文.md").click();
    await readDocument("Note 0");
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await invoke("e2e_fixture", { action: "save-as", text: null });
    await pane().$(".toolbar").$("button=Save As").click();
    await browser.waitUntil(
      async () =>
        (await invoke("e2e_fixture", { action: "read-copy" })).text ===
        original,
      { timeout: 60000 },
    );
    await ready();
    expect(await browser.$$(".tabs .tab").length).toBe(16);
    for (let index = 0; index < files.length; index++)
      expect(await fs.readFile(files[index], "utf8")).toBe(
        index ? `# Note ${index}\n` : original,
      );
    await browser.saveScreenshot("artifacts/native-sixteen-tabs.png");
  });

  it("reuses empty tabs at capacity and preserves active tabs across background close and cancellation", async () => {
    const fs = await import("node:fs/promises");
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# First\n",
    });
    const second = join(dirname(fixture.path), "second 文.txt");
    const third = join(dirname(fixture.path), "third.txt");
    await fs.writeFile(second, "# Second\n");
    await fs.writeFile(third, "# Third\n");
    handoff(fixture.path);
    await ready();
    await readDocument("First");
    await ready();
    await browser.$('button[aria-label="New tab"]').click();
    await browser.$('button[aria-label="New tab"]').click();
    handoff(second);
    await ready();
    await readDocument("Second");
    await ready();
    expect(await browser.$$(".tabs .tab").length).toBe(3);
    for (let slot = 3; slot < 16; slot++)
      await browser.$('button[aria-label="New tab"]').click();
    await expect(browser.$('button[aria-label="New tab"]')).toBeDisabled();
    await expect(
      browser.$(".workspace-actions").$("button=New note"),
    ).toBeEnabled();
    handoff(third);
    await ready();
    await readDocument("Third");
    await ready();
    expect(await browser.$$(".tabs .tab").length).toBe(16);
    await browser.$('.tabs button[aria-label="Close New tab"]').click();
    await ready();
    await readDocument("Third");
    await ready();
    await expect(selected()).toHaveText("third.txt");
    await browser.$('.tabs [role="tab"]').click();
    await ready();
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await ready();
    await browser.tauri.execute(() => {
      document.querySelector(".document-tab:not([hidden]) .cm-content").focus();
      document.execCommand("insertText", false, "unsaved ");
    });
    await expect(selected()).toHaveText(expect.stringContaining("*"));
    const firstTab = await selected().getAttribute("id");
    await browser.$$('.tabs [role="tab"]')[1].click();
    await ready();
    await readDocument("Second");
    await ready();
    await browser
      .$(`.tabs .tab:has(#${firstTab}) button[aria-label^="Close "]`)
      .click();
    await expect(
      browser.$('dialog[open] [id$="-discard-title"]'),
    ).toBeDisplayed();
    for (const close of await browser.$$('.tabs button[aria-label^="Close "]'))
      await expect(close).toBeDisabled();
    await browser.$("dialog[open]").$("button=Keep editing").click();
    await ready();
    await readDocument("Second");
    await ready();
    await expect(selected()).toHaveText("second 文.txt");
    await browser.$(`#${firstTab}`).click();
    await expect(pane().$(".cm-content")).toHaveText(
      expect.stringContaining("unsaved"),
    );
    expect(await fs.readFile(fixture.path, "utf8")).toBe("# First\n");
  });

  it("browses expandable folders and opens live filename and content search results with the keyboard", async () => {
    const fs = await import("node:fs/promises");
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Original tree fixture\n",
    });
    const root = dirname(fixture.path);
    await fs.mkdir(join(root, "nested"));
    await fs.writeFile(
      join(root, "nested", "Scratch.md"),
      "# Scratch\n\nA needle for content search.\n",
    );
    await fs.writeFile(join(root, "Other.md"), "# Other\n");
    await Promise.all(
      Array.from({ length: 20 }, (_, i) =>
        fs.writeFile(
          join(root, `Selection ${String(i).padStart(2, "0")}.md`),
          "# Selection\n",
        ),
      ),
    );
    await pane().$("button=Open folder").click();
    await browser
      .$(`button[aria-label="Remove root ${root.split("/").at(-1)}"]`)
      .waitForExist({ timeout: 60000 });
    await browser
      .$(".library-views")
      .$("button=Files")
      .waitForEnabled({ timeout: 60000 });
    await browser.$(".library-views").$("button=Files").click();
    const folder = browser.$('button[aria-label="Folder nested"]');
    await folder.waitForExist({ timeout: 60000 });
    await folder.scrollIntoView({ block: "nearest" });
    await folder.click();
    await expect(folder).toHaveAttribute("aria-expanded", "true");
    await browser.keys("ArrowLeft");
    await expect(folder).toHaveAttribute("aria-expanded", "false");
    await browser.keys("ArrowRight");
    const file = browser.$('.root-list button[aria-label="Open Scratch.md"]');
    await file.waitForEnabled({ timeout: 60000 });
    await file.click();
    await readDocument("Scratch");
    await browser.$(".library-views").$("button=File details").click();
    await expect(browser.$(".library-controls")).toBeDisplayed();
    await browser.$(".library-views").$("button=Files").click();
    await expect(
      browser.$('button[aria-label="Folder nested"]'),
    ).toHaveAttribute("aria-expanded", "true");
    await browser.$(".workspace-actions .quick-open-button").click();
    const search = browser.$('dialog[aria-label="Quick open and search"]');
    const input = search.$('[aria-label="Workspace query"]');
    try {
      await input.setValue("Selection");
      await browser.waitUntil(
        async () => (await search.$$(".search-result-open")).length === 20,
        { timeout: 60000 },
      );
      await input.click();
      await browser.keys(Array(15).fill("ArrowDown"));
      await browser.waitUntil(() =>
        browser.tauri.execute(() => {
          const list = document.querySelector(".workspace-search-results"),
            selected = list.querySelector("li.selected");
          const outer = list.getBoundingClientRect(),
            item = selected.getBoundingClientRect();
          return item.top >= outer.top - 1 && item.bottom <= outer.bottom + 1;
        }),
      );
      await input.setValue("Other");
      await input.setValue("Scratch");
      await search
        .$('button[aria-label="Open Scratch.md"]')
        .waitForDisplayed({ timeout: 60000 });
      await expect(search.$(".workspace-search-results")).not.toHaveText(
        expect.stringContaining("Other.md"),
      );
      await search.$("button=File contents").click();
      await input.setValue("needle for content");
      await search.$("button=Search").click();
      await search
        .$('button[aria-label="Open Scratch.md"]')
        .waitForDisplayed({ timeout: 60000 });
      await search.$('button[aria-label="Preview result Scratch.md"]').click();
      await expect(browser.$(".quick-preview article h1")).toHaveText(
        "Scratch",
      );
      await browser.$("button=Close preview").click();
      await browser.saveScreenshot("artifacts/native-folder-search.png");
      await input.click();
      await browser.keys("Enter");
      await expect(search).not.toBeDisplayed();
      await readDocument("Scratch");
      expect((await invoke("e2e_fixture", { action: "read" })).text).toBe(
        "# Original tree fixture\n",
      );
    } finally {
      if (await search.getProperty("open"))
        await search.$("button=Close search").click();
      const authorized = (await invoke("library_state")).roots.find(
        (folder) => folder.name === root.split("/").at(-1),
      );
      if (authorized)
        await invoke("library_remove_root", { id: authorized.id });
    }
  });

  it("follows wiki links and shows saved backlinks without replacing unsaved Source", async () => {
    const fs = await import("node:fs/promises");
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Untouched\n",
    });
    const root = dirname(fixture.path);
    await fs.mkdir(join(root, "one"));
    await fs.mkdir(join(root, "two"));
    await fs.writeFile(
      join(root, "Start.md"),
      "# Start\n\n[[Other#Details|Open other]] and [[Duplicate]]\n",
    );
    await fs.writeFile(
      join(root, "Other.md"),
      "# Other\n\n## Details\n\nA linked note.\n",
    );
    await fs.writeFile(join(root, "Back.md"), "[Other](Other.md)\n");
    for (const folder of ["one", "two"])
      await fs.writeFile(join(root, folder, "Duplicate.md"), "# Duplicate\n");
    const authorized = await invoke("library_choose_root");
    handoff(join(root, "Start.md"));
    await readDocument("Start");
    const startTab = await selected().getAttribute("id");
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await browser.tauri.execute(() => {
      window.linkEditor = document.querySelector(
        ".document-tab:not([hidden]) .cm-editor",
      );
      window.linkEditor.querySelector(".cm-content").focus();
      document.execCommand("insertText", false, "[[UnsavedOnly]] ");
    });
    await pane().$("button=Split").click();
    const links = () => pane().$('dialog[aria-label="Note links"][open]');
    try {
      await pane()
        .$('a[data-note-link="Other#Details"]')
        .waitForDisplayed({ timeout: 60000 });
      await pane().$('a[data-note-link="Other#Details"]').click();
      await readDocument("Other");
      await pane().$(".toolbar").$("button=Links").click();
      await links()
        .$('section[aria-label="Backlinks"]')
        .waitForDisplayed({ timeout: 60000 });
      await expect(links().$('section[aria-label="Backlinks"]')).toHaveText(
        expect.stringContaining("Start.md"),
      );
      await expect(links().$('section[aria-label="Backlinks"]')).toHaveText(
        expect.stringContaining("Back.md"),
      );
      await links().$("button=Close links").click();
      await browser.$(`#${startTab}`).click();
      expect(
        await browser.tauri.execute(
          () =>
            window.linkEditor ===
            document.querySelector(".document-tab:not([hidden]) .cm-editor"),
        ),
      ).toBe(true);
      await expect(pane().$(".cm-content")).toHaveText(
        expect.stringContaining("UnsavedOnly"),
      );
      await pane().$('a[data-note-link="Duplicate"]').click();
      await links()
        .$('section[aria-label="Matching notes"]')
        .waitForDisplayed({ timeout: 60000 });
      await expect(links()).toHaveText(
        expect.stringContaining("one/Duplicate.md"),
      );
      await expect(links()).toHaveText(
        expect.stringContaining("two/Duplicate.md"),
      );
      expect(await browser.$$(".tabs .tab").length).toBe(2);
      await browser.saveScreenshot("artifacts/native-note-links.png");
    } catch (error) {
      await writeFile(
        "artifacts/native-note-links-before-cleanup.json",
        JSON.stringify(
          await browser.tauri.execute(() => ({
            text: document.body.innerText.slice(0, 12000),
            errors: window.marktubTestErrors,
          })),
          null,
          2,
        ),
      );
      await browser.saveScreenshot(
        "artifacts/native-note-links-before-cleanup.png",
      );
      throw error;
    } finally {
      if (await links().isExisting())
        await links().$("button=Close links").click();
      await browser.tauri.execute(() => {
        delete window.linkEditor;
      });
      await invoke("library_remove_root", { id: authorized.id });
    }
  });

  it("creates recoverable new notes and templates and saves only to a native-selected destination", async () => {
    await invoke("e2e_fixture", { action: "seed", text: "# Existing file\n" });
    await browser.$('button[aria-label="New tab"]').click();
    await browser.$('button[aria-label="New tab"]').click();
    const empty = await selected().getAttribute("id");
    await browser.$(".workspace-actions").$("button=New note").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await ready();
    expect(await selected().getAttribute("id")).toBe(empty);
    await expect(selected()).toHaveText(
      expect.stringContaining("Untitled.md *"),
    );
    await browser.tauri.execute(() => {
      document.querySelector(".document-tab:not([hidden]) .cm-content").focus();
      document.execCommand("insertText", false, "# A new thought\n");
    });
    await pane().$('button[aria-label="Undo"]').click();
    await expect(pane().$(".cm-content")).toHaveText("");
    await browser.tauri.execute(() => {
      document.querySelector(".document-tab:not([hidden]) .cm-content").focus();
      document.execCommand("insertText", false, "# A new thought\n");
    });
    await invoke("e2e_fixture", { action: "save-as", text: null });
    await pane().$(".toolbar").$("button=Save").click();
    await browser.waitUntil(
      async () =>
        (await invoke("e2e_fixture", { action: "read-copy" })).text ===
        "# A new thought\n",
    );
    expect((await invoke("e2e_fixture", { action: "read" })).text).toBe(
      "# Existing file\n",
    );
    await closeDocuments();
    await pane().$("button=From template").click();
    const chooser = browser.$('dialog[aria-label="New note from template"]');
    await chooser.$("input[maxlength='160']").setValue("Team planning");
    await chooser.$('input[value="meeting"]').click();
    await chooser.$("button=Create note").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await expect(pane().$(".cm-content")).toHaveText(
      expect.stringContaining("## Decisions"),
    );
    await expect(selected()).toHaveText(
      expect.stringContaining("Team planning.md *"),
    );
    await expect(chooser).not.toBeDisplayed();
    await browser.waitUntil(() =>
      browser.tauri.execute(() =>
        document.activeElement?.classList.contains("cm-content"),
      ),
    );
    await browser.saveScreenshot("artifacts/native-note-template.png");
  });

  it("reorders tabs by pointer and keyboard without replacing Source or losing exact-byte undo", async () => {
    const original = "\uFEFF# Moving tabs\r\nkeep\nlast\rend";
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: original,
    });
    await openHomeFile();
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed();
    await browser.tauri.execute(() => {
      window.uxEditor = document.querySelector(
        ".document-tab:not([hidden]) .cm-editor",
      );
      window.uxEditor.querySelector(".cm-content").focus();
      document.execCommand("insertText", false, "moving ");
    });
    await expect(selected()).toHaveText(expect.stringContaining("*"));
    const first = await selected().getAttribute("id");
    const second = join(dirname(fixture.path), "second-tab.txt");
    await (
      await import("node:fs/promises")
    ).writeFile(second, "# Second tab\n");
    handoff(second);
    await readDocument("Second tab");
    const other = await selected().getAttribute("id");
    const source = browser.$(`#${first}`),
      destination = browser.$(`#${other}`);
    const from = await source.getLocation(),
      to = await destination.getLocation(),
      size = await destination.getSize();
    await browser.performActions([
      {
        type: "pointer",
        id: "tab-drag",
        parameters: { pointerType: "mouse" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            x: Math.round(from.x + 35),
            y: Math.round(from.y + 15),
          },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 300,
            x: Math.round(to.x + size.width - 8),
            y: Math.round(to.y + 15),
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ]);
    await browser.releaseActions();
    await browser.waitUntil(
      async () =>
        (
          await browser
            .$$('.tabs [role="tab"]')
            .map((tab) => tab.getAttribute("id"))
        ).join(",") === `${other},${first}`,
    );
    await expect(selected()).toHaveAttribute("id", first);
    expect(
      await browser.tauri.execute(
        () =>
          window.uxEditor ===
          document.querySelector(".document-tab:not([hidden]) .cm-editor"),
      ),
    ).toBe(true);
    await browser.$('[aria-label="Tab actions"]').click();
    await browser.$(".tab-menu").$("button=Move tab left").click();
    await browser.waitUntil(
      async () =>
        (await browser.$('.tabs [role="tab"]').getAttribute("id")) === first,
    );
    await browser.tauri.execute(() =>
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "ArrowRight",
          altKey: true,
          shiftKey: true,
          bubbles: true,
        }),
      ),
    );
    await browser.waitUntil(
      async () =>
        (await browser.$('.tabs [role="tab"]').getAttribute("id")) === other,
    );
    await expect(pane().$(".cm-content")).toHaveText(
      expect.stringContaining("moving "),
    );
    await pane().$('button[aria-label="Undo"]').click();
    await pane().$("header").$("button=Save").waitForEnabled();
    await pane().$("header").$("button=Save").click();
    await expect(pane().$(".statusbar")).toHaveText(
      expect.stringContaining("Saved"),
    );
    expect((await invoke("e2e_fixture", { action: "read" })).text).toBe(
      original,
    );
    await browser.saveScreenshot("artifacts/native-ux-tabs.png");
    await browser.tauri.execute(() => {
      delete window.uxEditor;
    });
  });

  it("resizes the library and previews, reverts and applies settings without losing scope drafts", async () => {
    const before = await invoke("get_settings", { id: null, scope: "global" });
    const session = await invoke("get_settings", {
      id: null,
      scope: "session",
    });
    const prefs = () => browser.$('dialog[aria-label="Preferences"]');
    try {
      await invoke("update_settings", { id: null, scope: "session", text: "" });
      await browser.$(".workspace-actions").$("button=Library").click();
      const divider = browser.$('[aria-label="Resize library"]');
      await divider.waitForDisplayed();
      const location = await divider.getLocation(),
        size = await divider.getSize();
      const initial = Number(await divider.getAttribute("aria-valuenow"));
      await browser.performActions([
        {
          type: "pointer",
          id: "library-drag",
          parameters: { pointerType: "mouse" },
          actions: [
            {
              type: "pointerMove",
              duration: 0,
              x: Math.round(location.x + size.width / 2),
              y: Math.round(location.y + 80),
            },
            { type: "pointerDown", button: 0 },
            {
              type: "pointerMove",
              duration: 250,
              x: Math.round(location.x + size.width / 2 + 32),
              y: Math.round(location.y + 80),
            },
            { type: "pointerUp", button: 0 },
          ],
        },
      ]);
      await browser.releaseActions();
      await browser.waitUntil(
        async () =>
          (await invoke("get_settings", { id: null, scope: "global" }))
            .resolved["library.width"].value ===
          initial + 32,
      );
      await divider.click();
      await browser.keys("ArrowRight");
      await browser.waitUntil(
        async () =>
          (await invoke("get_settings", { id: null, scope: "global" }))
            .resolved["library.width"].value ===
          initial + 48,
      );
      await browser.$('button[aria-label="Settings"]').click();
      await prefs().waitForDisplayed();
      await browser.waitUntil(
        async () => !(await prefs().$("fieldset").getProperty("disabled")),
      );
      await expect(prefs().$("button=Apply settings")).toBeDisabled();
      await prefs().$("button=Workspace").click();
      await prefs().$('input[id="pref-library.width"]').setValue("420");
      await browser.waitUntil(
        async () => Number(await divider.getAttribute("aria-valuenow")) === 420,
      );
      expect(
        (await invoke("get_settings", { id: null, scope: "global" })).resolved[
          "library.width"
        ].value,
      ).toBe(initial + 48);
      await expect(
        prefs().$('select[aria-label="Settings scope"]'),
      ).toBeDisabled();
      await expect(prefs().$('input[id="pref-library.width"]')).toHaveValue(
        "420",
      );
      await prefs().$("button=Revert changes").click();
      await browser.waitUntil(
        async () =>
          Number(await divider.getAttribute("aria-valuenow")) === initial + 48,
      );
      await prefs().$('input[id="pref-library.width"]').setValue("398");
      await prefs().$("button=Apply settings").waitForEnabled();
      await prefs().$("button=Apply settings").click();
      await prefs().waitForDisplayed({ reverse: true });
      expect(
        (await invoke("get_settings", { id: null, scope: "global" })).resolved[
          "library.width"
        ].value,
      ).toBe(398);
      await browser.$('button[aria-label="Settings"]').click();
      await browser.waitUntil(
        async () => !(await prefs().$("fieldset").getProperty("disabled")),
      );
      await prefs().$("button=Appearance").click();
      await prefs().$("button*=Dark").click();
      await browser.waitUntil(
        async () =>
          (await browser.tauri.execute(() =>
            document.documentElement.style.getPropertyValue("--paper"),
          )) === "#202127",
      );
      await prefs().$("button=Revert changes").click();
      await browser.waitUntil(
        async () => (await prefs().getAttribute("aria-busy")) === "false",
      );
      await browser.saveScreenshot("artifacts/native-ux-settings.png");
      await prefs().$("button=Cancel").click();
    } finally {
      if (await prefs().isDisplayed()) await prefs().$("button=Cancel").click();
      await invoke("update_settings", {
        id: null,
        scope: "global",
        text: before.text,
      });
      await invoke("update_settings", {
        id: null,
        scope: "session",
        text: session.text,
      });
    }
  });

  it("opens the complete built-in feature guide and authorizes only its bundled image", async () => {
    await browser.$('button[aria-label="Open feature guide"]').click();
    await readDocument("A place for unfinished thoughts");
    const image = pane().$('button[data-image="marktub-example.png"]');
    await image.waitForExist({ timeout: 60000 });
    await image.scrollIntoView();
    await image.click();
    await browser.waitUntil(async () =>
      browser.tauri.execute(() => {
        const image = document.querySelector(
          ".document-tab:not([hidden]) article img",
        );
        return (
          image?.complete &&
          image.naturalWidth === 256 &&
          image.src.startsWith("data:image/png;base64,")
        );
      }),
    );
    const headings = await pane()
      .$$("article h2")
      .map((heading) => heading.getText());
    for (const title of [
      "Read and write",
      "Notes and navigation",
      "Math and equation references",
      "Diagrams",
      "Local images",
      "Git and Review",
      "Sharing and export",
      "Optional local tools",
    ])
      expect(headings).toContain(title);
    expect(
      await browser.tauri.execute(() => {
        const header = document
          .querySelector(".workspace-actions")
          .getBoundingClientRect();
        return (
          header.top >= 0 &&
          header.bottom < innerHeight &&
          document.querySelector(".workbench").scrollTop === 0
        );
      }),
    ).toBe(true);
    await browser.saveScreenshot("artifacts/native-ux-guide.png");
    await pane().$("button=Split").waitForEnabled();
    await pane().$("button=Split").click();
    await pane().$(".cm-content").waitForDisplayed();
    await expect(pane().$(".toolbar .save-button")).toBeDisplayed();
    const effective = (
      await invoke("get_settings", { id: null, scope: "global" })
    ).resolved;
    await expect(pane().$('select[aria-label="Reading width"]')).toHaveValue(
      String(effective["reader.width"].value),
    );
    await expect(
      pane().$('select[aria-label="Reading text size"]'),
    ).toHaveValue(String(effective["reader.font_size"].value));
  });

  it("keeps Source view and exact-byte undo across More, Escape and preference dialog focus", async () => {
    const original = "\uFEFF# Polish\r\nالعربية 日本語 🦀\nlast\rend";
    await invoke("e2e_fixture", { action: "seed", text: original });
    await openHomeFile();
    await ready();
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await ready();
    await browser.tauri.execute(() => {
      window.workflowEditor = document.querySelector(
        ".document-tab:not([hidden]) .cm-editor",
      );
      window.workflowEditor.querySelector(".cm-content").focus();
      document.execCommand("insertText", false, "polish ");
    });
    await expect(selected()).toHaveText(expect.stringContaining("*"));
    const summary = () => pane().$(".document-actions summary");
    await summary().click();
    await browser.tauri.execute(() =>
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Escape",
          isComposing: true,
          bubbles: true,
        }),
      ),
    );
    expect(await pane().$(".document-actions").getProperty("open")).toBe(true);
    await browser.tauri.execute(() =>
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "p",
          code: "KeyP",
          metaKey: navigator.platform.startsWith("Mac"),
          ctrlKey: !navigator.platform.startsWith("Mac"),
          shiftKey: true,
          bubbles: true,
        }),
      ),
    );
    await browser
      .$('dialog[aria-label="Command palette"][open]')
      .waitForDisplayed();
    await browser.keys("Escape");
    await browser
      .$('dialog[aria-label="Command palette"][open]')
      .waitForExist({ reverse: true });
    expect(await pane().$(".document-actions").getProperty("open")).toBe(false);
    await summary().click();
    await expect(pane().$(".document-actions")).toHaveAttribute("open");
    await browser.keys("Escape");
    expect(await pane().$(".document-actions").getProperty("open")).toBe(false);
    expect(await summary().isFocused()).toBe(true);
    await documentAction("Settings");
    await browser
      .$('dialog[aria-label="Preferences"]')
      .$("button=Cancel")
      .click();
    expect(await summary().isFocused()).toBe(true);
    await summary().click();
    await pane().$(".cm-content").click();
    expect(await pane().$(".document-actions").getProperty("open")).toBe(false);
    expect(
      await browser.tauri.execute(
        () =>
          window.workflowEditor ===
          document.querySelector(".document-tab:not([hidden]) .cm-editor"),
      ),
    ).toBe(true);
    await pane().$('button[aria-label="Undo"]').click();
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane()
      .$("header")
      .$("button=Save")
      .waitForEnabled({ timeout: 60000 });
    await pane().$("header").$("button=Save").click();
    await expect(pane().$(".statusbar")).toHaveText(
      expect.stringContaining("Saved"),
    );
    expect((await invoke("e2e_fixture", { action: "read" })).text).toBe(
      original,
    );
    await browser.tauri.execute(() => {
      delete window.workflowEditor;
    });
  });

  it("keeps narrow panes and secondary actions reachable in the actual WebView", async () => {
    await browser.$(".workspace-actions").$("button=New note").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await ready();
    await browser.$('.tabs button[aria-label^="Close "]').click();
    await pane().$("button=Keep recovery").click();
    await ready();
    const fixture = await invoke("e2e_fixture", {
      action: "seed",
      text: "# Layout\n\nA document leaves room to write.\n",
    });
    await openHomeFile();
    await ready();
    await pane().$("button=Source").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForDisplayed({ timeout: 60000 });
    await ready();
    const recoveries = pane().$(".recovery-list");
    await recoveries.waitForDisplayed({ timeout: 60000 });
    expect(await recoveries.getProperty("open")).toBe(false);
    await recoveries.$("summary").click();
    await expect(recoveries.$("button=Restore")).toBeDisplayed();
    await recoveries.$("summary").click();
    const observations = [];
    try {
      for (const width of [960, 640]) {
        await browser.tauri.execute((_api, width) => {
          const host = document.querySelector(".document-host");
          host.style.maxWidth = `${width}px`;
          host.style.maxHeight = "555px";
        }, width);
        await pane().$(".document-actions summary").click();
        const geometry = await browser.tauri.execute(() => {
          const pane = document.querySelector(".document-tab:not([hidden])");
          const rect = pane.getBoundingClientRect();
          const toolbar = pane
            .querySelector(".toolbar")
            .getBoundingClientRect();
          const popup = pane
            .querySelector(".document-actions-list")
            .getBoundingClientRect();
          const editor = pane
            .querySelector(".cm-editor")
            .getBoundingClientRect();
          return {
            width: rect.width,
            height: rect.height,
            recoveryCollapsed: !pane.querySelector(".recovery-list").open,
            toolbarHeight: toolbar.height,
            popupWithinPane:
              popup.left >= rect.left && popup.right <= rect.right,
            bodyOverflow: document.documentElement.scrollWidth > innerWidth,
            editorHeight: editor.height,
            toolsWithinPane: [
              ...pane.querySelectorAll(".author-controls button"),
            ].every((button) => {
              const b = button.getBoundingClientRect();
              return (
                b.width >= 30 &&
                b.height >= 30 &&
                b.left >= rect.left &&
                b.right <= rect.right &&
                !!button.getAttribute("aria-label")
              );
            }),
            toolsOverflow:
              pane.querySelector(".author-controls").scrollWidth >
              pane.querySelector(".author-controls").clientWidth,
          };
        });
        expect(geometry.recoveryCollapsed).toBe(true);
        expect(geometry.popupWithinPane).toBe(true);
        expect(geometry.toolsWithinPane).toBe(true);
        expect(geometry.toolsOverflow).toBe(false);
        expect(geometry.bodyOverflow).toBe(false);
        expect(geometry.editorHeight).toBeGreaterThan(200);
        observations.push(geometry);
        await browser.saveScreenshot(`artifacts/workflow-source-${width}.png`);
        await browser.keys("Escape");
      }
    } finally {
      await browser.tauri.execute(() => {
        const host = document.querySelector(".document-host");
        host.style.maxWidth = "";
        host.style.maxHeight = "";
      });
    }
    if (!(await browser.$(".library-host").isDisplayed()))
      await browser.$(".workspace-actions").$("button=Library").click();
    await browser.$("button=Add folder").click();
    await browser
      .$(
        `button[aria-label="Remove root ${dirname(fixture.path).split("/").at(-1)}"]`,
      )
      .waitForExist();
    await pane().$(".document-actions summary").click();
    await expect(
      pane().$(".document-actions").$("button=Settings"),
    ).toBeDisplayed();
    await browser.saveScreenshot("artifacts/workflow-source-library.png");
    await browser.keys("Escape");
    await writeFile(
      "artifacts/workflow-layout.json",
      JSON.stringify(
        {
          method: `actual ${nativeWebView}; programmatically constrained pane widths and 555px height with retained recovery, not physical monitor/OS resize certification`,
          observations,
        },
        null,
        2,
      ),
    );
  });
});

describe("Math context policy", () => {
  it("renders finalized equation context across unmounted sections and standalone export", async () => {
    const call = (command, args = {}) =>
      browser.tauri.execute(
        ({ core }, command, args) => core.invoke(command, args),
        command,
        args,
      );
    const pane = () => browser.$(".document-tab:not([hidden])");
    const source =
      "# Local math policy\n\n$$\\begin{equation}x\\label{first}\\end{equation}$$\n\n$\\gdef\\local{q}\\local$\n\n$$x=y\\tag{7}$$\n\n" +
      "Padding for independent section navigation.\n\n".repeat(400) +
      "# Later reference\n\n$$\\eqref{first}$$\n\n$$(1)$$\n\n";
    await call("e2e_fixture", { action: "seed", text: source });
    const close = await browser.$$('.tabs button[aria-label^="Close "]');
    for (const button of close) {
      await button.waitForEnabled({ timeout: 60000 });
      await button.click();
      if (await browser.$('dialog[open] [id$="-discard-title"]').isDisplayed())
        await browser.$("button=Discard edits").click();
    }
    await pane().$("button.open-button").waitForEnabled({ timeout: 60000 });
    await pane().$("button.open-button").click();
    await expect(pane().$("article h1")).toHaveText("Local math policy");
    await pane().$("button=Export HTML").waitForEnabled({ timeout: 60000 });
    await pane().$("[data-rich] svg").waitForExist({ timeout: 30000 });
    await expect(pane().$(".render-error")).not.toExist();
    await expect(pane().$('[data-rich] svg[role="img"]')).toExist();
    await call("e2e_fixture", { action: "export" });
    await pane().$("button=Export HTML").click();
    await pane().$("button=Export HTML").waitForEnabled({ timeout: 60000 });
    await expect(pane().$(".reader-notice")).toHaveText(
      expect.stringContaining("export completed"),
    );
    const info = await call("e2e_fixture", { action: "read-export" });
    const html = await (
      await import("node:fs/promises")
    ).readFile(info.path, "utf8");
    expect(html).not.toContain("Document-wide math context is unsupported");
    expect(html).not.toContain("Unresolved document math reference");
    expect((html.match(/<svg\b/g) ?? []).length).toBeGreaterThanOrEqual(5);
    expect(html).toContain("\\eqref{first}");
    expect(html).toContain("\\label{first}");
    expect(html).not.toContain("<script");
    await pane().$("button=Source").click();
    await pane().$(".cm-content").waitForExist({ timeout: 30000 });
    await pane().$("button=Visual").click();
    await pane()
      .$('.cm-author-block svg[role="img"]')
      .waitForExist({ timeout: 30000 });
    await pane().$(".cm-author-block").$("button=Edit math").click();
    const controls = pane().$("dialog.author-dialog[open]");
    await controls.$("button=Preview locally").click();
    await controls.$('svg[role="img"]').waitForExist({ timeout: 30000 });
    await expect(controls.$('[role="alert"]')).not.toExist();
    await controls.$("button=Close").click();
    await pane().$("button=Read").click();
    await pane().$("button=Export HTML").waitForEnabled({ timeout: 60000 });
    await pane().$("button=Outline & find").click();
    await pane().$(".outline nav").$("button=Later reference").click();
    await expect(pane().$("article h1")).toHaveText("Later reference");
    await pane().$("[data-rich] svg").waitForExist({ timeout: 30000 });
    await browser.waitUntil(
      async () => (await pane().$$("[data-rich] svg")).length >= 2,
    );
    const formulaPaths = await browser.tauri.execute(() =>
      [
        ...document.querySelectorAll(
          ".document-tab:not([hidden]) article [data-rich] svg",
        ),
      ]
        .slice(0, 2)
        .map((svg) =>
          [...svg.querySelectorAll("path")].map((path) =>
            path.getAttribute("d"),
          ),
        ),
    );
    expect(formulaPaths[0].length).toBeGreaterThan(0);
    expect(formulaPaths[0]).toEqual(formulaPaths[1]);
    await expect(pane().$(".render-error")).not.toExist();
    expect((await call("e2e_fixture", { action: "read" })).text).toBe(source);
    await writeFile("artifacts/math-context-export.html", html);
  });
});
