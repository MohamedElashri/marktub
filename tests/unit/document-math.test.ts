// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { DocumentMath } from "../../src/renderers/math-document";
import { MathWork } from "../../src/renderers/math-work";
import { AuthorMath } from "../../src/editor/math";
import { sanitizeRich } from "../../src/reader/safe";
import {
  authorMathSpans,
  mathPlanForText,
} from "../../src/reader/math-context";
import { mathjaxDocument } from "../../src/renderers/mathjax";
import {
  appendMath,
  emptyMathPlan,
  mathExpressions,
} from "../../src/reader/math-context";
import { buildIndex, renderText } from "../../src/reader/indexer";
import { grammar, emptyContext } from "../../src/reader/grammar";
import { input } from "../support/reader-parity";
import type { MathExpression } from "../../src/reader/model";
const expression = (source: string, display = true): MathExpression => ({
  source,
  display,
});

it("compiles forward references, ordered equation tags and global definitions without viewport state", () => {
  const formulas = [
    expression(String.raw`\eqref{later}`),
    expression(String.raw`\gdef\abc{q}`),
    expression(String.raw`\begin{equation}\abc=x\label{later}\end{equation}`),
    expression(String.raw`\begin{equation}y=z\label{second}\end{equation}`),
    expression(String.raw`\ref{second}`),
  ];
  const output = mathjaxDocument(formulas);
  expect(output[0]).toContain('data-c="31"');
  expect(output[2]).toContain('data-c="1D45E"');
  expect(output[2]).toContain('id="mjx-eqn:later"');
  expect(output[4]).toContain('data-c="32"');
  const accessible = document.createElement("div");
  accessible.innerHTML = sanitizeRich(output[4], "document-ref");
  expect(
    accessible.querySelector(".math-alternative math")?.textContent?.trim(),
  ).toBe("2");
  expect(mathjaxDocument(formulas)).toEqual(output);
  expect(() => mathjaxDocument([expression(String.raw`\abc`)])).toThrow(
    "Undefined control sequence",
  );
});
it("fails visibly on unknown/duplicate labels, recursion and resource-bearing definitions", () => {
  for (const formulas of [
    [expression(String.raw`\eqref{missing}`)],
    [
      expression(String.raw`\begin{equation}a\label{x}\end{equation}`),
      expression(String.raw`\begin{equation}b\label{x}\end{equation}`),
    ],
    [expression(String.raw`\gdef\forever{\forever}\forever`)],
    [expression(String.raw`\gdef\bad{\href{https://invalid}{x}}`)],
  ])
    expect(() => mathjaxDocument(formulas)).toThrow();
});
it("bounds global context without breaking the indexed source or retaining an oversized formula set", () => {
  const plan = emptyMathPlan();
  appendMath(
    plan,
    Array.from({ length: 129 }, () => expression("x")),
  );
  appendMath(plan, [expression(String.raw`\eqref{x}`)]);
  expect(plan.needsContext).toBe(true);
  expect(plan.issue).toContain("128 formulas");
  expect(plan.expressions).toEqual([]);
  expect(() =>
    mathjaxDocument(Array.from({ length: 129 }, () => expression("x"))),
  ).toThrow("admission budget");
});
it("runs one owned worker, refuses stale/provisional inputs and releases it on close", async () => {
  const instances: FakeWorker[] = [];
  class FakeWorker {
    onmessage: ((message: { data: unknown }) => void) | null = null;
    onerror: (() => void) | null = null;
    terminated = false;
    constructor() {
      instances.push(this);
    }
    postMessage() {}
    terminate() {
      this.terminated = true;
    }
  }
  vi.stubGlobal("Worker", FakeWorker);
  try {
    const plan = emptyMathPlan();
    appendMath(plan, [expression(String.raw`\eqref{x}`)]);
    const session = new DocumentMath(plan);
    const first = session.render(0, plan.expressions[0].source, true);
    const second = session.render(0, plan.expressions[0].source, true);
    expect(instances.length).toBe(1);
    instances[0].onmessage!({ data: { results: ["<svg>resolved</svg>"] } });
    expect(await first).toContain("resolved");
    expect(await second).toContain("resolved");
    expect(instances[0].terminated).toBe(true);
    await expect(session.render(0, "stale", true)).rejects.toThrow(
      "finalized document context",
    );
    session.destroy();
    await expect(
      session.render(0, plan.expressions[0].source, true),
    ).rejects.toMatchObject({ name: "AbortError" });
    const closing = new DocumentMath(plan);
    const pending = expect(
      closing.render(0, plan.expressions[0].source, true),
    ).rejects.toMatchObject({ name: "AbortError" });
    closing.destroy();
    await pending;
    expect(instances[1].terminated).toBe(true);
  } finally {
    vi.unstubAllGlobals();
  }
});
it("terminates an unresponsive math worker at its deadline", async () => {
  const terminate = vi.fn();
  vi.useFakeTimers();
  vi.stubGlobal(
    "Worker",
    class {
      onmessage = null;
      onerror = null;
      terminate = terminate;
      postMessage() {}
    },
  );
  try {
    const job = new MathWork({
      type: "document",
      expressions: [expression("x")],
    });
    const timeout = expect(job.result).rejects.toThrow(
      "eight-second worker budget",
    );
    await vi.advanceTimersByTimeAsync(8000);
    await timeout;
    expect(terminate).toHaveBeenCalledTimes(1);
    job.destroy();
    expect(terminate).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});
it("keeps simple author previews bounded and local, with exact spans for contextual formulas", async () => {
  const text = "Code `$inert$`; literal \\$skip$.\n\nReal $x+1$ and $$y=2$$.\n";
  const source = new AuthorMath(text);
  const from = text.indexOf("$x+1$");
  expect(await source.render(from, from + 1, "x+1", () => true)).toContain(
    "katex",
  );
  await expect(source.render(0, 1, "inert", () => true)).rejects.toThrow(
    "verified source span",
  );
  source.destroy();
  await expect(
    source.render(from, from + 1, "x+1", () => true),
  ).rejects.toMatchObject({ name: "AbortError" });
  const contextual =
    "# Heading\n\n$\\gdef\\a{q}$\n\n> $\\a$\n\n$\\eqref{x}$\n\n$$\\begin{equation}x\\label{x}\\end{equation}$$\n";
  const spans = authorMathSpans(contextual, mathPlanForText(contextual));
  expect(spans.map((span) => span.index)).toEqual([0, 1, 2, 3]);
  for (const span of spans) {
    const raw = contextual.slice(span.from, span.to);
    expect(raw).toBe(
      (span.display ? "$$" : "$") + span.source + (span.display ? "$$" : "$"),
    );
  }
});
it("indexes exactly the whole parser's math order across UTF-8 pages, sections and the footnote appendix", async () => {
  const source =
    String.raw`# Equations

$\eqref{later}$ and \$literal; ![not $math$](local.png).

\`code $inert$\`

$$
\gdef\abc{q}
$$

` +
    "Paragraph العربية 🦀.\n\n".repeat(300) +
    String.raw`
$$
\begin{equation}\abc=x\label{later}\end{equation}
$$

Note[^n].

[^n]: $\ref{later}$
`;
  const whole = mathExpressions(
    grammar().parse(source, { ...emptyContext(), beginning: true }),
  );
  for (const width of [73, 4096, 65536]) {
    const index = await buildIndex(input(source, width), () => {});
    expect(index.complete).toBe(true);
    expect(index.math?.needsContext).toBe(true);
    expect(index.math?.expressions.slice(0, whole.length)).toEqual(whole);
    const bytes = new TextEncoder().encode(source);
    const seen: MathExpression[] = [];
    for (const [i, section] of index.sections.entries()) {
      const text = section.note
        ? `### Footnote ${section.note}\n\n${index.context.notes[section.note]}`
        : new TextDecoder().decode(bytes.subarray(section.start, section.end));
      const rich = renderText(
        text,
        index.context,
        index.headings.filter((h) => h.section === i),
        section.start === 0,
      ).rich.filter((r) => r.kind === "math");
      if (rich.length) expect(section.mathStart).toBe(seen.length);
      seen.push(...rich.map(({ source, display }) => ({ source, display })));
    }
    expect(index.math?.expressions).toEqual(seen);
    expect(mathjaxDocument(seen).at(-1)).toContain('data-c="31"');
  }
});
