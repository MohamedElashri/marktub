// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { exportStandalone } from "../../src/reader/standalone";
import type { ReaderClient } from "../../src/reader/client";
import type { Values } from "../../src/preferences";
import { exportFontCSS } from "../../src/fonts/export";
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

const settings: Values = {
  theme: { type: "enum", value: "riwaq_night" },
  "reader.font": { type: "text", value: "Noto Naskh Arabic" },
  "reader.heading_font": { type: "text", value: "Amiri" },
  "editor.font": { type: "text", value: "Cousine" },
};

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
    "reader.font": { type: "text", value: "Thmanyah Serif Text" },
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
