import { grammar, emptyContext } from "../reader/grammar";
import { mathPlanForText } from "../reader/math-context";
import { DocumentMath } from "../renderers/math-document";
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
  return svgPng(html);
}
async function svgPng(html: string, math = false) {
  const template = document.createElement("template");
  template.innerHTML = html;
  const svg = template.content.querySelector("svg")?.cloneNode(true) as
    SVGElement | undefined;
  if (!svg) throw Error("Rendered SVG unavailable.");
  svg.removeAttribute("xmlns");
  svg.removeAttribute("xmlns:xlink");
  const view = (svg.getAttribute("viewBox") ?? "0 0 800 600")
    .split(/[ ,]+/)
    .map(Number);
  let width = view[2],
    height = view[3];
  if (math) {
    width = (svg.getAttribute("width") ?? "").endsWith("ex")
      ? parseFloat(svg.getAttribute("width")!) * 9
      : width * 0.018;
    height = (svg.getAttribute("height") ?? "").endsWith("ex")
      ? parseFloat(svg.getAttribute("height")!) * 9
      : height * 0.018;
  }
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
export async function documentMathPngs(
  text: string,
  admit: (source: string, display: boolean, png: string) => Promise<unknown>,
) {
  const plan = mathPlanForText(text);
  if (!plan.needsContext) return;
  const math = new DocumentMath(plan);
  const { sanitizeRich } = await import("../reader/safe");
  try {
    const images = new Map<
      string,
      { source: string; display: boolean; html: string }
    >();
    const ambiguous = new Set<string>();
    for (const [index, expression] of plan.expressions.entries()) {
      const html = await math.render(
        index,
        expression.source,
        expression.display,
      );
      const key = JSON.stringify([
        expression.source.trim(),
        expression.display,
      ]);
      if (images.has(key) && images.get(key)!.html !== html) ambiguous.add(key);
      else images.set(key, { ...expression, html });
    }
    if (plan.issue) throw Error(plan.issue);
    for (const [key, expression] of images) {
      if (ambiguous.has(key)) continue;
      const clean = sanitizeRich(expression.html, "pdf-math");
      await admit(
        expression.source,
        expression.display,
        await svgPng(clean, true),
      );
    }
    if (ambiguous.size)
      throw Error(
        "Repeated formulas with different document context remain source in PDF; HTML keeps their distinct rendering.",
      );
  } finally {
    math.destroy();
  }
}
