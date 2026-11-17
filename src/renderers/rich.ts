import { sanitizeRich, sanitizeReader } from "../reader/safe";
import { escape } from "../reader/grammar";
import { MathPolicyError, requireLocalMath } from "./math-policy";
import type { Rich } from "../reader/model";
import type { DocumentMath } from "./math-document";
let queue: Promise<void> = Promise.resolve();
let uid = 0;
const cache = new Map<string, string>();
let cacheBytes = 0;
let cacheEpoch = 0;
export function clearRichCache() {
  cacheEpoch++;
  cache.clear();
  cacheBytes = 0;
}
export function admissibleDiagram(source: string) {
  if (source.length > 12000 || (source.match(/--|==|->|:/g)?.length ?? 0) > 100)
    throw Error(
      "Diagram exceeds the 12,000 character / 100 edge admission budget.",
    );
  if (
    /%%\{|^\s*---|\b(?:click|href|style|classDef|linkStyle|icon|image)\b|<|https?:|file:|javascript:/im.test(
      source,
    )
  )
    throw Error("Diagram directives, links, styling and HTML are disabled.");
  if (
    !/^\s*(?:graph|flowchart|sequenceDiagram|classDiagram|stateDiagram(?:-v2)?|erDiagram|pie|gantt|journey|mindmap|timeline)\b/.test(
      source,
    )
  )
    throw Error("This diagram family is source-only.");
}
export async function renderRich(
  rich: Rich,
  engine: "auto" | "mathjax" = "auto",
  current: () => boolean = () => true,
  math?: Pick<DocumentMath, "render">,
): Promise<string> {
  const checkCurrent = () => {
    if (!current())
      throw new DOMException("Rich view closed or changed.", "AbortError");
  };
  checkCurrent();
  if (rich.source.length > 8192 && rich.kind === "math")
    throw new MathPolicyError("Math source exceeds 8,192 characters.");
  if (
    /\\(?:require|autoload|includegraphics|href|url|html\w*|style|class|cssId)\b/.test(
      rich.source,
    )
  )
    throw new MathPolicyError("Math resource/HTML commands are disabled.");
  const contextual = rich.kind === "math" && rich.mathIndex !== undefined;
  if (rich.kind === "math" && !contextual) requireLocalMath(rich.source);
  const key = JSON.stringify([rich, engine, "v1"]);
  const cached = contextual ? undefined : cache.get(key);
  if (cached) return sanitizeRich(cached, `cached-${++uid}`);
  const epoch = cacheEpoch;
  let result: string;
  let svgAlternative = rich.kind === "mermaid";
  if (rich.kind === "math") {
    if (contextual) {
      if (!math)
        throw new MathPolicyError(
          "Document math is provisional until context scanning finishes. Source preserved.",
        );
      result = sanitizeRich(
        await math.render(rich.mathIndex!, rich.source, rich.display, current),
        `math-${++uid}`,
      );
      svgAlternative = true;
    } else if (
      engine === "mathjax" ||
      /\\(?:prescript|mathclap|DeclarePairedDelimiter)\b/.test(rich.source)
    ) {
      const { mathjaxSvg } = await import("./mathjax");
      checkCurrent();
      svgAlternative = true;
      result = sanitizeRich(
        mathjaxSvg(rich.source, rich.display),
        `math-${++uid}`,
      );
    } else {
      const katex = await import("katex");
      await import("katex/dist/katex.min.css");
      checkCurrent();
      result = sanitizeRich(
        katex.default.renderToString(rich.source, {
          displayMode: rich.display,
          throwOnError: true,
          trust: false,
          strict: "error",
          maxExpand: 500,
          maxSize: 20,
          macros: {},
          output: "htmlAndMathml",
        }),
        `math-${++uid}`,
      );
    }
  } else {
    admissibleDiagram(rich.source);
    const render = async () => {
      checkCurrent();
      // A preceding queued job may have populated this key while we waited.
      const ready = cache.get(key);
      if (ready) return sanitizeRich(ready, `queued-${++uid}`);
      const { default: mermaid } = await import("mermaid");
      checkCurrent();
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        htmlLabels: false,
        suppressErrorRendering: true,
        maxTextSize: 12000,
        maxEdges: 100,
        flowchart: { htmlLabels: false },
        theme: "neutral",
        fontFamily: "Arial, sans-serif",
        themeVariables: { fontFamily: "Arial, sans-serif", fontSize: "16px" },
      });
      const { svg } = await mermaid.render(`diagram-${++uid}`, rich.source);
      const clean = sanitizeRich(svg, `diagram-${uid}`);
      const template = document.createElement("template");
      template.innerHTML = clean;
      for (const shape of template.content.querySelectorAll(
        "rect,polygon,circle,ellipse",
      )) {
        shape.setAttribute("fill", "#f3f4f7");
        shape.setAttribute("stroke", "#3d55b4");
      }
      for (const svg of template.content.querySelectorAll("svg")) {
        svg.setAttribute("font-family", "Arial, sans-serif");
        svg.setAttribute("font-size", "16");
        svg.setAttribute(
          "style",
          (svg.getAttribute("style") ?? "") + ";background-color:#ffffff;",
        );
      }
      for (const text of template.content.querySelectorAll("text,tspan")) {
        text.setAttribute("fill", "#24262d");
        text.setAttribute("font-family", "Arial, sans-serif");
        text.setAttribute("font-size", "16");
      }
      for (const path of template.content.querySelectorAll("path")) {
        if (!path.hasAttribute("stroke"))
          path.setAttribute("stroke", "#3d55b4");
        if (!path.hasAttribute("fill"))
          path.setAttribute(
            "fill",
            path.closest("marker") ? "#3d55b4" : "none",
          );
      }
      return template.innerHTML;
    };
    const next = queue.then(render, render);
    // Do not keep the last completed SVG alive through the queue's value.
    queue = next.then(
      () => {},
      () => {},
    );
    result = await next;
  }
  checkCurrent();
  if (result.length > 1024 * 1024)
    throw Error("Rendered block exceeds the 1 MiB output budget.");
  // MathJax/diagram SVG has no native MathML alternative. Keep its original
  // local notation available to assistive technology, including cached output.
  if (svgAlternative) {
    const accessible = document.createElement("template");
    accessible.innerHTML = result;
    for (const svg of accessible.content.querySelectorAll("svg")) {
      svg.setAttribute("role", "img");
      svg.removeAttribute("aria-hidden");
      svg.removeAttribute("aria-labelledby");
      svg.setAttribute(
        "aria-label",
        `${rich.kind === "math" ? "TeX math" : "Diagram source"}: ${rich.source}`,
      );
      if (accessible.content.querySelector(".math-alternative math"))
        svg.setAttribute("aria-hidden", "true");
    }
    result = accessible.innerHTML;
  }
  if (contextual || epoch !== cacheEpoch) return result;
  cacheBytes -= (cache.get(key)?.length ?? 0) * 2;
  cache.set(key, result);
  cacheBytes += result.length * 2;
  while (cacheBytes > 8 * 1024 * 1024) {
    const first = cache.keys().next().value!;
    cacheBytes -= cache.get(first)!.length * 2;
    cache.delete(first);
  }
  return result;
}
export async function hydrate(
  host: HTMLElement,
  rich: Rich[],
  current: () => boolean,
  math?: DocumentMath,
) {
  const targets = Array.from(host.querySelectorAll<HTMLElement>("[data-rich]"));
  // Bound actual retained output, rather than rejecting a fifth small formula.
  const outputLimit = 8 * 1024 * 1024;
  let outputBytes = 0;
  let exhausted = false;
  for (const target of targets) {
    if (!current() || !target.isConnected) return;
    const item = rich[Number(target.dataset.rich)];
    if (!item) continue;
    const fallback = () => {
      target.innerHTML = `<details class="render-error"><summary>Rich output exceeds the 8 MiB section limit. Source is available.</summary><pre>${escape(item.source)}</pre></details>`;
    };
    if (exhausted) {
      fallback();
      continue;
    }
    try {
      const started = performance.now();
      const cached = cache.has(JSON.stringify([item, "auto", "v1"]));
      const html = await renderRich(
        item,
        "auto",
        () => current() && target.isConnected,
        math,
      );
      if (current() && target.isConnected) {
        if (outputBytes + html.length * 2 > outputLimit) {
          exhausted = true;
          fallback();
          continue;
        }
        outputBytes += html.length * 2;
        target.innerHTML = html;
        target.dataset.renderMs = String(performance.now() - started);
        target.dataset.richKind = item.kind;
        target.dataset.cacheHit = String(cached);
      }
    } catch (error) {
      if (current() && target.isConnected) {
        target.innerHTML = `<details class="render-error"><summary>${escape(String(error).slice(0, 220))}</summary><pre>${escape(item.source)}</pre></details>`;
        if (item.kind === "math" && !(error instanceof MathPolicyError)) {
          const retry = document.createElement("button");
          retry.textContent = "Try MathJax";
          retry.onclick = async () => {
            try {
              const html = await renderRich(
                item,
                "mathjax",
                () => current() && target.isConnected,
              );
              if (current() && target.isConnected) target.innerHTML = html;
            } catch (e) {
              retry.textContent = String(e).slice(0, 160);
            }
          };
          target.append(retry);
        }
      }
    }
  }
  const code = Array.from(
    host.querySelectorAll<HTMLElement>("pre code"),
  ).filter((c) => c.textContent && c.textContent.length < 16000);
  if (code.length) {
    const { highlight } = await import("./highlight");
    for (const block of code) {
      if (!current()) return;
      block.innerHTML = sanitizeReader(
        highlight(
          block.textContent ?? "",
          block.className.replace("language-", ""),
        ),
      );
    }
  }
}
