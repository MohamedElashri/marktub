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
const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);
// The pinned MathJax API accepts a FontData instance. Reuse glyph metrics, not
// parser/macro/label/document state, instead of rebuilding the font per formula.
let font: InstanceType<typeof MathJaxTexFont> | undefined;
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
