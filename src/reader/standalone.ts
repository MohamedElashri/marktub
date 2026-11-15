import { invoke } from "@tauri-apps/api/core";
import { ReaderClient } from "./client";
import { sanitizeReader } from "./safe";
import { renderRich } from "../renderers/rich";
const LIMIT = 48 * 1024 * 1024;
export async function exportStandalone(
  client: ReaderClient,
  title: string,
  revision: string,
) {
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
  const doc = document.implementation.createHTMLDocument(title);
  doc.documentElement.lang = "en";
  const csp = doc.createElement("meta");
  csp.httpEquiv = "Content-Security-Policy";
  csp.content =
    "default-src 'none'; style-src 'unsafe-inline'; font-src data:; img-src data:; base-uri 'none'; form-action 'none'";
  doc.head.append(csp);
  const meta = doc.createElement("meta");
  meta.setAttribute("charset", "utf-8");
  doc.head.prepend(meta);
  const style = doc.createElement("style");
  style.textContent = `body{max-width:760px;margin:40px auto;padding:0 24px;font:18px/1.7 Palatino,Georgia,serif;color:#24352f;background:#fafbf9}pre{white-space:pre-wrap;background:#f0f4f1;padding:18px}table{border-collapse:collapse}td,th{border:1px solid #dce3df;padding:8px}svg{max-width:100%;height:auto}.export-revision{font:13px system-ui;color:#66736d} ${fonts}`;
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
