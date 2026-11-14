import { grammar, emptyContext } from "../reader/grammar";
type Token = ReturnType<ReturnType<typeof grammar>["parse"]>[number];
export function resources(text: string) {
  const images = new Set<string>(),
    diagrams: string[] = [];
  function walk(tokens: Token[]) {
    for (const token of tokens) {
      if (token.type === "image") {
        const source = token.attrGet("src");
        if (typeof source === "string" && source) images.add(source);
      }
      if (token.type === "fence" && token.info.trim() === "mermaid")
        diagrams.push(token.content);
      if (token.children) walk(token.children);
    }
  }
  walk(grammar().parse(text, { ...emptyContext(), beginning: true }));
  if (images.size > 16 || diagrams.length > 10)
    throw Error(
      "PDF assets admit 16 image references and 10 diagrams. Simplify the capture before exporting.",
    );
  return { images: [...images], diagrams, error: "" };
}
export async function diagramPng(source: string) {
  const { renderRich } = await import("../renderers/rich");
  const html = await renderRich({ kind: "mermaid", source, display: true });
  const doc = new DOMParser().parseFromString(html, "image/svg+xml");
  const svg = doc.querySelector("svg");
  if (!svg || doc.querySelector("parsererror"))
    throw Error("Diagram SVG unavailable.");
  const view = (svg.getAttribute("viewBox") ?? "0 0 800 600")
    .split(/[ ,]+/)
    .map(Number);
  let width = view[2],
    height = view[3];
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  )
    throw Error("Diagram dimensions are invalid.");
  const scale = Math.min(2, 2048 / width, 2048 / height);
  width = Math.max(1, Math.ceil(width * scale));
  height = Math.max(1, Math.ceil(height * scale));
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  const image = new Image();
  image.src =
    "data:image/svg+xml;charset=utf-8," +
    encodeURIComponent(new XMLSerializer().serializeToString(svg));
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) throw Error("Raster export unavailable.");
  context.fillStyle = "#fafbf9";
  context.fillRect(0, 0, width, height);
  context.drawImage(image, 0, 0, width, height);
  return canvas.toDataURL("image/png").split(",")[1];
}
