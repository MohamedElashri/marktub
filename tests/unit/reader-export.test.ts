// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { exportStandalone } from "../../src/reader/standalone";
import type { ReaderClient } from "../../src/reader/client";
import type { Values } from "../../src/preferences";
import { exportFontCSS } from "../../src/fonts/export";
import * as bundledFonts from "../../src/fonts/export";
import { renderRich } from "../../src/renderers/rich";
import type { Page } from "../../src/reader/model";
const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn<
    (command: string, args: { content: string }) => Promise<boolean>
  >(async () => true),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../../src/renderers/rich", () => ({ renderRich: vi.fn() }));
vi.mock("virtual:marktub-notices", () => ({
  default: "Retained font notices",
}));
beforeEach(() => {
  invoke.mockClear();
  vi.mocked(renderRich).mockReset();
});
afterEach(() => vi.restoreAllMocks());

const settings: Values = {
  theme: { type: "enum", value: "riwaq_night" },
  "reader.font": { type: "text", value: "noto naskh arabic" },
  "reader.heading_font": { type: "text", value: "amiri" },
  "editor.font": { type: "text", value: "cousine" },
};

it("exports the same steady Arabic fallback face used by the app", async () => {
  const css = await exportFontCSS({
    "reader.font": { type: "text", value: "Noto Sans Arabic" },
  });
  const faces = [
    ...css.matchAll(
      /@font-face\{font-family:"([^"]+)"[^}]+src:url\(([^)]+)\)/g,
    ),
  ];
  expect(faces.map((face) => face[1])).toEqual([
    "Noto Sans Arabic",
    "Marktub Arabic",
    "Marktub Arabic Source",
  ]);
  expect(faces[0][2]).toBe(faces[1][2]);
  expect(faces[2][2]).not.toBe(faces[1][2]);
  expect(css).toContain("unicode-range:U+0600-06FF");
});

it("exports captured Riwaq appearance and approved fonts without external resources or active content", async () => {
  const client = {
    complete: true,
    sections: [{}],
    approvedImages: new Map(),
    page: async () => ({
      html: '<h1>العربية</h1><p>English مع العربية <a href="https://example.org">label</a></p><script>bad()</script>',
      rich: [],
    }),
  } as unknown as ReaderClient;
  const reactive = new Proxy(
    Object.fromEntries(
      Object.entries(settings).map(([key, record]) => [
        key,
        new Proxy(record, {}),
      ]),
    ),
    {},
  );
  await exportStandalone(client, "<A title>", "Saved revision", reactive);
  expect(invoke).toHaveBeenLastCalledWith("export_html", expect.any(Object));
  const content = invoke.mock.lastCall![1].content as string;
  const doc = new DOMParser().parseFromString(content, "text/html");
  expect(doc.title).toBe("<A title>");
  expect(doc.querySelector("script,link,iframe,a[href]")).toBeNull();
  expect(
    doc
      .querySelector('meta[http-equiv="Content-Security-Policy"]')
      ?.getAttribute("content"),
  ).toContain("font-src data:");
  const css = doc.querySelector("style")!.textContent!;
  expect(css).toContain("--paper:#1a1a2e");
  expect(css).toContain('--reader-heading-font:"Amiri"');
  expect(css).toContain('font-family:"Cousine"');
  expect(css).toContain("data:font/ttf;base64,");
  expect(css).not.toMatch(/url\((?!data:)/);
  expect(css).not.toContain('font-family:"Noto Sans Arabic"');
  expect(doc.querySelector("details")?.textContent).toContain(
    "Retained font notices",
  );
});

it("embeds open fallback faces for installed Thmanyah without copying restricted font files", async () => {
  const css = await exportFontCSS({
    "reader.font": { type: "text", value: "thmanyah serif text" },
    "reader.heading_font": { type: "text", value: "Thmanyah Serif Display" },
  });
  expect(css).toContain('font-family:"Noto Naskh Arabic"');
  expect(css).toContain('font-family:"Amiri"');
  expect(css).not.toContain("Thmanyah");
  expect(css).not.toContain('font-family:"Cousine"');
});

it("cannot end the exported style element through a literal font name", async () => {
  const client = {
    complete: true,
    sections: [],
    approvedImages: new Map(),
  } as unknown as ReaderClient;
  await exportStandalone(client, "Safe title", "Saved", {
    "reader.font": {
      type: "text",
      value: "</style><script>bad()</script><style>",
    },
  });
  const content = invoke.mock.lastCall![1].content;
  const doc = new DOMParser().parseFromString(content, "text/html");
  expect(doc.querySelector("script")).toBeNull();
  expect(doc.querySelectorAll("style")).toHaveLength(1);
  expect(doc.querySelector("style")!.textContent).toContain("\\3c /style>");
});

it("keeps font assets consistent with the appearance captured before page assembly", async () => {
  const changing: Values = {
    theme: { type: "enum", value: "riwaq_night" },
    "reader.heading_font": { type: "text", value: "Amiri" },
  };
  const client = {
    complete: true,
    sections: [{}],
    approvedImages: new Map(),
    page: async () => {
      changing.theme.value = "riwaq_day";
      changing["reader.heading_font"].value = "Cousine";
      return { html: "<h1>عنوان</h1>", rich: [] };
    },
  } as unknown as ReaderClient;
  await exportStandalone(client, "Title", "Saved", changing);
  const content = invoke.mock.lastCall![1].content;
  expect(content).toContain("--paper:#1a1a2e");
  expect(content).toContain('font-family:"Amiri"');
  expect(content).not.toContain('font-family:"Cousine"');
});

function exportClient() {
  return {
    dead: false,
    complete: true,
    sections: [{}, {}],
    approvedImages: new Map(),
    page: vi.fn<ReaderClient["page"]>(async () => ({
      html: "<p>Kept source</p>",
      rich: [],
      provisional: false,
    })),
  };
}

it("never opens a save dialog when the reader closes during a page read", async () => {
  const client = exportClient();
  let finish!: (value: Page) => void;
  client.page.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const pending = exportStandalone(
    client as unknown as ReaderClient,
    "Title",
    "Saved",
  );
  const stopped = expect(pending).rejects.toMatchObject({ name: "AbortError" });
  client.dead = true;
  client.sections.length = 0;
  finish({ html: "<p>Partial content</p>", rich: [], provisional: false });
  await stopped;
  expect(invoke).not.toHaveBeenCalled();
});

it("does not turn canceled rich rendering into a partial saved document", async () => {
  const client = exportClient();
  client.page.mockResolvedValueOnce({
    html: '<span data-rich="0"></span>',
    rich: [{ kind: "math", source: "x", display: true }],
    provisional: false,
  });
  let cancel!: (error: Error) => void;
  vi.mocked(renderRich).mockImplementationOnce(
    () =>
      new Promise((_resolve, reject) => {
        cancel = reject;
      }),
  );
  const pending = exportStandalone(
    client as unknown as ReaderClient,
    "Title",
    "Saved",
  );
  const stopped = expect(pending).rejects.toMatchObject({ name: "AbortError" });
  await vi.waitFor(() => expect(renderRich).toHaveBeenCalled());
  client.dead = true;
  client.sections.length = 0;
  cancel(new DOMException("Reader closed", "AbortError"));
  await stopped;
  expect(invoke).not.toHaveBeenCalled();
});

it("checks the reader again after asynchronously loading export fonts", async () => {
  const client = exportClient();
  client.sections.length = 0;
  let finish!: (css: string) => void;
  const fonts = vi.spyOn(bundledFonts, "exportFontCSS").mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const pending = exportStandalone(
    client as unknown as ReaderClient,
    "Title",
    "Saved",
  );
  const stopped = expect(pending).rejects.toMatchObject({ name: "AbortError" });
  await vi.waitFor(() => expect(fonts).toHaveBeenCalled());
  client.dead = true;
  finish("");
  await stopped;
  expect(invoke).not.toHaveBeenCalled();
});

it("refuses to export an already closed reader", async () => {
  const client = exportClient();
  client.dead = true;
  client.sections.length = 0;
  await expect(
    exportStandalone(client as unknown as ReaderClient, "Title", "Saved"),
  ).rejects.toMatchObject({ name: "AbortError" });
  expect(invoke).not.toHaveBeenCalled();
});

it("retains the entire document when a live reader contains an unsupported formula", async () => {
  const client = exportClient();
  client.page.mockResolvedValueOnce({
    html: '<span data-rich="0"></span>',
    rich: [{ kind: "math", source: "unsupported-source", display: true }],
    provisional: false,
  });
  client.page.mockResolvedValueOnce({
    html: "<p>Final paragraph</p>",
    rich: [],
    provisional: false,
  });
  vi.mocked(renderRich).mockRejectedValueOnce(Error("Unsupported formula"));
  await exportStandalone(client as unknown as ReaderClient, "Title", "Saved");
  const content = invoke.mock.lastCall![1].content;
  expect(content).toContain("unsupported-source");
  expect(content).toContain("Final paragraph");
});
