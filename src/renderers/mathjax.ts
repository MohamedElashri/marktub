import { mathjax } from "@mathjax/src/js/mathjax.js";
import { TeX } from "@mathjax/src/js/input/tex.js";
import { SVG } from "@mathjax/src/js/output/svg.js";
import { liteAdaptor } from "@mathjax/src/js/adaptors/liteAdaptor.js";
import { RegisterHTMLHandler } from "@mathjax/src/js/handlers/html.js";
import { MathJaxTexFont } from "@mathjax/mathjax-tex-font/js/svg.js";
import "@mathjax/src/js/input/tex/base/BaseConfiguration.js";
import "@mathjax/src/js/input/tex/ams/AmsConfiguration.js";
import "@mathjax/src/js/input/tex/newcommand/NewcommandConfiguration.js";
import "@mathjax/src/js/input/tex/mathtools/MathtoolsConfiguration.js";
import "@mathjax/src/js/input/tex/begingroup/BegingroupConfiguration.js";
import type { MathExpression } from "../reader/model";
import { MathPolicyError } from "./math-policy";
import { MATH_EXPRESSIONS, MATH_SOURCE } from "../reader/math-context";
import type { MmlNode } from "@mathjax/src/js/core/MmlTree/MmlNode.js";
import { SerializedMmlVisitor } from "@mathjax/src/js/core/MmlTree/SerializedMmlVisitor.js";
const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);
// The pinned MathJax API accepts a FontData instance. Reuse glyph metrics, not
// parser/macro/label/document state, instead of rebuilding the font per formula.
let font: InstanceType<typeof MathJaxTexFont> | undefined;
export function mathjaxDocument(expressions: MathExpression[]) {
  if (
    expressions.length > MATH_EXPRESSIONS ||
    expressions.reduce((n, e) => n + e.source.length, 0) > MATH_SOURCE
  )
    throw new MathPolicyError(
      "Document math context exceeds its admission budget.",
    );
  for (const expression of expressions)
    if (
      expression.source.length > 8192 ||
      /\\(?:require|autoload|includegraphics|href|url|html\w*|style|class|cssId|catcode|csname|xdef|globaldefs)\b/.test(
        expression.source,
      )
    )
      throw new MathPolicyError(
        "Document math resource, HTML or unsupported expansion commands are disabled. Source preserved.",
      );
  const input = new TeX({
    packages: ["base", "ams", ["newcommand", 3], "mathtools", "begingroup"],
    tags: "ams",
    maxBuffer: 8192,
    maxMacros: 500,
    formatError: (_jax: unknown, error: Error) => {
      throw error;
    },
  });
  const output = new SVG({
    fontCache: "none",
    fontData: font ?? MathJaxTexFont,
  });
  font ??= output.font as InstanceType<typeof MathJaxTexFont>;
  const doc = mathjax.document("", { InputJax: input, OutputJax: output });
  try {
    const items = expressions.map(({ source, display }) => {
      const item = new doc.options.MathItem(source, input, display);
      item.setMetrics(18, 9, 720, 100000, 1);
      doc.math.push(item);
      return item;
    });
    // Compile all items in Markdown order, including MathJax's second pass for
    // forward references, before producing any viewport-dependent SVG.
    doc.compile();
    for (const [index, item] of items.entries()) {
      if (item.inputData.error)
        throw new MathPolicyError(
          `Document math formula ${index + 1}: ${String(item.inputData.error.message)}. Source preserved.`,
        );
      let unresolved = false;
      item.root.walkTree((node: MmlNode) => {
        if (
          node.attributes?.get("class") === "MathJax_ref" &&
          String(node.attributes.get("href")).endsWith("#")
        )
          unresolved = true;
      });
      if (unresolved)
        throw new MathPolicyError(
          `Unresolved document math reference in formula ${index + 1}. Source preserved.`,
        );
    }
    doc.typeset();
    let bytes = 0;
    return items.map((item) => {
      const html =
        adaptor.outerHTML(item.typesetRoot) +
        `<span class="sr-only math-alternative">${new SerializedMmlVisitor().visitTree(item.root)}</span>`;
      bytes += html.length * 2;
      if (html.length > 1024 * 1024 || bytes > 8 * 1024 * 1024)
        throw new MathPolicyError(
          "Document math exceeds its 1 MiB per formula / 8 MiB output budget. Source preserved.",
        );
      return html;
    });
  } finally {
    doc.clear();
  }
}
export function mathjaxSvg(source: string, display: boolean) {
  // Fresh input/output/document per expression: macro/label state never depends
  // on viewport order. No require/autoload/HTML/URL packages or dynamic fonts.
  const input = new TeX({
    packages: ["base", "ams", "newcommand", "mathtools"],
    maxBuffer: 8192,
    maxMacros: 500,
    formatError: (_jax: unknown, error: Error) => {
      throw error;
    },
  });
  const output = new SVG({
    fontCache: "none",
    fontData: font ?? MathJaxTexFont,
  });
  font ??= output.font as InstanceType<typeof MathJaxTexFont>;
  const document = mathjax.document("", { InputJax: input, OutputJax: output });
  return adaptor.outerHTML(
    document.convert(source, { display, em: 18, ex: 9, containerWidth: 720 }),
  );
}
