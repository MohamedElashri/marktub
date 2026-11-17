import { invoke } from "@tauri-apps/api/core";
import { ReaderClient } from "./client";
import { sanitizeReader } from "./safe";
import { renderRich } from "../renderers/rich";
import { appearance, type Values } from "../preferences";
import typography from "./typography.css?raw";
const LIMIT = 48 * 1024 * 1024;
export async function exportStandalone(
  client: ReaderClient,
  title: string,
  revision: string,
  presentation: Values = {},
  dark = false,
) {
  const settings = structuredClone(presentation);
  // CSS strings may contain a literal HTML end tag. Escape '<' before the
  // style element is serialized into standalone HTML's raw-text context.
  const css = appearance(settings, dark).replaceAll("<", "\\3c ");
  if (!client.complete) throw Error("Document context is still provisional.");
  const parts: string[] = [];
  let cost = 0,
    hasKatex = false;
  for (let i = 0; i < client.sections.length; i++) {
    const page = await client.page(i);
    const host = document.createElement("div");
    host.innerHTML = sanitizeReader(page.html);
    for (const target of host.querySelectorAll<HTMLElement>("[data-rich]")) {
      const rich = page.rich[Number(target.dataset.rich)];
      if (!rich) continue;
      try {
        target.innerHTML = await renderRich(
          rich,
          "auto",
          () => !client.dead,
          client.math,
        );
        hasKatex ||= !!target.querySelector(".katex");
      } catch (error) {
        target.textContent = `${String(error)}\n${rich.source}`;
      }
    }
    for (const image of host.querySelectorAll<HTMLElement>(
      "button[data-image]",
    )) {
      const source = client.approvedImages.get(image.dataset.image ?? "");
      if (source) {
        const img = document.createElement("img");
        img.src = source;
        img.alt = image.textContent ?? "Image";
        img.style.maxWidth = "100%";
        if (image.dataset.width) img.width = Number(image.dataset.width);
        if (image.dataset.height) img.height = Number(image.dataset.height);
        image.replaceWith(img);
      } else {
        const text = document.createElement("p");
        text.textContent = `Image not embedded (permission required): ${image.textContent}`;
        image.replaceWith(text);
      }
    }
    // Export anchors are local-only. Cross-file and remote links remain labeled text.
    for (const anchor of host.querySelectorAll("a[href]"))
      if (!anchor.getAttribute("href")!.startsWith("#"))
        anchor.removeAttribute("href");
    cost += host.innerHTML.length * 2;
    if (cost > LIMIT)
      throw Error(
        "HTML export exceeded its 48 MiB assembly budget. No destination was written.",
      );
    parts.push(host.innerHTML);
  }
  let fonts = "";
  if (hasKatex) {
    const css = (await import("katex/dist/katex.min.css?raw")).default;
    const assets = (await import("virtual:marktub-export-fonts")).default;
    fonts = css.replace(/src:[^;}]+/g, (declaration) => {
      const name = /fonts\/([^)'"\s]+\.woff2)/.exec(declaration)?.[1];
      return name ? `src:url(__FONT_${name}__) format('woff2')` : "";
    });
    for (const [name, data] of Object.entries(assets))
      fonts = fonts.replaceAll(`__FONT_${name}__`, data);
    if (fonts.includes("__FONT_"))
      throw Error("Export font mapping incomplete.");
  }
  fonts += await (await import("../fonts/export")).exportFontCSS(settings);
  const doc = document.implementation.createHTMLDocument(title);
  doc.body.className = "prose";
  const csp = doc.createElement("meta");
  csp.httpEquiv = "Content-Security-Policy";
  csp.content =
    "default-src 'none'; style-src 'unsafe-inline'; font-src data:; img-src data:; base-uri 'none'; form-action 'none'";
  doc.head.append(csp);
  const meta = doc.createElement("meta");
  meta.setAttribute("charset", "utf-8");
  doc.head.prepend(meta);
  const style = doc.createElement("style");
  style.textContent = `body{${css};max-width:var(--reader-width);margin:40px auto;padding:0 24px;font-family:var(--reader-font);font-size:var(--reader-size);line-height:var(--reader-line-height);color:var(--ink);background:var(--paper)}a{color:var(--accent)}pre{white-space:pre-wrap;background:var(--code);padding:18px}pre,code{font-family:var(--source-font)}table{border-collapse:collapse}td,th{border:1px solid var(--line);padding:8px}svg{max-width:100%;height:auto}.export-revision{font:13px var(--ui-font);color:var(--quiet)}.hljs-keyword,.hljs-built_in,.hljs-literal{color:var(--keyword)}.hljs-string{color:var(--string)}.hljs-number{color:var(--number)} ${typography} ${fonts}`;
  doc.head.append(style);
  style.textContent +=
    ".sr-only{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}";
  const label = doc.createElement("p");
  label.className = "export-revision";
  label.textContent = revision;
  doc.body.append(label);
  for (const html of parts) {
    const section = doc.createElement("section");
    section.innerHTML = html;
    doc.body.append(section);
  }
  const credits = doc.createElement("details");
  const summary = doc.createElement("summary");
  summary.textContent = "Renderer and embedded-font licenses";
  const licenses = doc.createElement("pre");
  licenses.textContent = (await import("virtual:marktub-notices")).default;
  credits.append(summary, licenses);
  doc.body.append(credits);
  const content = "<!doctype html>\n" + doc.documentElement.outerHTML;
  if (content.length * 2 > 64 * 1024 * 1024)
    throw Error("Self-contained export exceeds the 64 MiB output budget.");
  const saved = await invoke<boolean>("export_html", { content });
  if (!saved) throw Error("Export canceled.");
}
