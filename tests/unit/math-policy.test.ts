// @vitest-environment jsdom
import { expect, it } from "vitest";
import { mathContextIssue } from "../../src/renderers/math-policy";
import { renderRich, hydrate, clearRichCache } from "../../src/renderers/rich";
it("requires source for global math semantics before either renderer/cache can hide missing context", async () => {
  for (const source of [
    String.raw`x\label{a}`,
    String.raw`\eqref{later}`,
    String.raw`\gdef\x{1}`,
    String.raw`\global\def\x{1}`,
    String.raw`\begin{align}a&=b\end{align}`,
    String.raw`\newcommand{\x}{\label{a}}`,
  ]) {
    expect(mathContextIssue(source)).toBeDefined();
    for (const engine of ["auto", "mathjax"] as const)
      await expect(
        renderRich({ kind: "math", source, display: true }, engine),
      ).rejects.toThrow("Document-wide math context");
  }
  const source = String.raw`x\label{later}`;
  const host = document.createElement("div");
  host.innerHTML = '<div data-rich="0"></div>';
  document.body.append(host);
  await hydrate(host, [{ kind: "math", source, display: true }], () => true);
  expect(host.querySelector(".render-error")?.textContent).toContain(source);
  expect(host.querySelector(".render-error summary")?.textContent).toContain(
    "Document-wide math context",
  );
  expect(host.querySelector("math,svg")).toBeNull();
  expect(host.querySelector("button")).toBeNull();
  host.remove();
});
it("keeps local macros and explicit tags independent of viewport/document order", async () => {
  for (const source of [
    String.raw`\begin{aligned}x&=y\end{aligned}`,
    String.raw`\begin{align*}x&=y\end{align*}`,
    String.raw`\tag{7}x=y`,
    "x % \\label{comment}\n+y",
    String.raw`\\label`,
    String.raw`\% x`,
  ])
    expect(mathContextIssue(source)).toBeUndefined();
  for (const engine of ["auto", "mathjax"] as const) {
    const local = {
      kind: "math" as const,
      source: String.raw`\def\localonly{q}\localonly`,
      display: true,
    };
    expect(await renderRich(local, engine)).toMatch(/katex|svg/);
    clearRichCache();
    await expect(
      renderRich(
        { kind: "math", source: String.raw`\localonly`, display: true },
        engine,
      ),
    ).rejects.toThrow();
    expect(await renderRich(local, engine)).toMatch(/katex|svg/);
  }
});
