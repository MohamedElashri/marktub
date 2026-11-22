import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView, drawSelection, keymap } from "@codemirror/view";
import { writingDirections } from "../../src/editor/direction.ts";
import sample from "./sample.txt?raw";
import "./editor.css";

const params = new URLSearchParams(location.search);
const useOverlay = params.get("mode") === "overlay";
const benchmark = params.has("bench");
if (benchmark) document.body.dataset.bench = "";
const documentText = sample.trimEnd().concat("\n").repeat(400).trimEnd();
const pango = window.webkit.messageHandlers.pango;
const report = window.webkit.messageHandlers.report;
if (useOverlay && (!CSS.highlights || typeof Highlight === "undefined")) {
  const error = "The overlay requires CSS Custom Highlights.";
  report.postMessage({ error });
  throw new Error(error);
}
const visibleMarks = /[\u0610-\u061a\u064b-\u065f\u0670\u06d6-\u06ed]/u;
const stats = {
  rounds: 0,
  replies: 0,
  maxWidthDrift: 0,
  paintedLines: 0,
  wrappedLines: 0,
};

const view = new EditorView({
  parent: document.getElementById("editor"),
  doc: documentText,
  extensions: [
    markdown(),
    writingDirections,
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap]),
    drawSelection(),
    EditorView.lineWrapping,
    EditorView.updateListener.of((update) => {
      if (
        useOverlay &&
        (update.docChanged || update.viewportChanged || update.geometryChanged)
      ) {
        if (update.docChanged) {
          CSS.highlights.delete("pango-ink");
          layer.replaceChildren();
        }
        schedulePaint();
      }
    }),
  ],
});

const layer = document.createElement("div");
layer.className = "pango-layer";
layer.setAttribute("aria-hidden", "true");
view.dom.append(layer);
const cache = new Map();
let generation = 0;
let frame;

function schedulePaint() {
  const token = ++generation;
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => void paint(token));
}

function visibleLines() {
  const viewport = view.scrollDOM.getBoundingClientRect();
  const origin = view.dom.getBoundingClientRect();
  const sites = [];
  let wrapped = 0;
  for (const line of view.contentDOM.querySelectorAll(".cm-line")) {
    const text = line.textContent;
    if (!visibleMarks.test(text)) continue;
    const box = line.getBoundingClientRect();
    if (box.bottom <= viewport.top || box.top >= viewport.bottom) continue;
    const style = getComputedStyle(line);
    const lineHeight = parseFloat(style.lineHeight);
    if (box.height > lineHeight + 1) {
      wrapped += 1;
      continue;
    }
    const range = document.createRange();
    range.selectNodeContents(line);
    const ink = range.getBoundingClientRect();
    const rtl = style.direction === "rtl";
    sites.push({
      range,
      text,
      top: box.top - origin.top,
      anchor: (rtl ? ink.right : ink.left) - origin.left,
      width: ink.width,
      height: lineHeight,
      rtl,
    });
  }
  return { sites, wrapped };
}

async function paint(token) {
  const { sites, wrapped } = visibleLines();
  const missing = [
    ...new Set(
      sites.map((site) => site.text).filter((text) => !cache.has(text)),
    ),
  ];
  if (missing.length) {
    try {
      const rows = await pango.postMessage(missing);
      stats.replies += 1;
      missing.forEach((text, index) => cache.set(text, rows[index]));
      for (const key of cache.keys()) {
        if (cache.size <= 256) break;
        cache.delete(key);
      }
    } catch (error) {
      if (token === generation) report.postMessage({ error: String(error) });
      return;
    }
  }
  if (token !== generation) return;

  const overlays = [];
  const highlight = new Highlight();
  for (const site of sites) {
    const outline = cache.get(site.text);
    if (!outline) continue;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const path = document.createElementNS(svg.namespaceURI, "path");
    path.setAttribute("d", outline.path);
    svg.append(path);
    svg.setAttribute("width", String(outline.width));
    svg.setAttribute("height", String(site.height));
    svg.style.left = `${site.anchor - (site.rtl ? outline.width : 0)}px`;
    svg.style.top = `${site.top}px`;
    highlight.add(site.range);
    overlays.push(svg);

    stats.maxWidthDrift = Math.max(
      stats.maxWidthDrift,
      Math.abs(site.width - outline.width),
    );
  }
  CSS.highlights.set("pango-ink", highlight);
  layer.replaceChildren(...overlays);
  stats.rounds += 1;
  stats.paintedLines = overlays.length;
  stats.wrappedLines = wrapped;
}

view.scrollDOM.addEventListener(
  "scroll",
  () => {
    if (useOverlay) schedulePaint();
  },
  { passive: true },
);
const observer = new ResizeObserver(() => {
  if (useOverlay) schedulePaint();
});
observer.observe(view.dom);

window.prototypeState = (inserted = "") => ({
  ...stats,
  prefix: view.state.doc.sliceString(0, 100),
  cursor: view.state.selection.main.head,
  documentMatches: view.state.doc.toString() === inserted + documentText,
});
window.prototypePaste = (text) => {
  view.focus();
  return document.execCommand("insertText", false, text);
};
window.prototypeEvict = (text) => cache.delete(text);

await document.fonts.load('26px "Noto Naskh Arabic"', "بٌّ");
await document.fonts.load("26px Cousine", "2026");
await document.fonts.ready;
view.focus();
if (useOverlay) {
  if (!CSS.highlights || typeof Highlight === "undefined") {
    report.postMessage({
      error: "The overlay requires CSS Custom Highlights.",
    });
  } else {
    await paint(++generation);
  }
}
report.postMessage({ ready: true });
