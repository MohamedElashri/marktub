import { it, expect } from "vitest";
import { mkdirSync, writeFileSync } from "node:fs";
import { sanitizeReader } from "../src/reader/safe";

it("records reader HTML preparation cost separately from native layout", () => {
  const source = `<blockquote><ul>${Array.from({ length: 64 }, (_, i) => `<li><p><a href="note.md">العربية ${i}</a> with English <code>x + 1</code></p></li>`).join("")}</ul></blockquote>`;
  const samples = [];
  let result = "";
  for (let i = 0; i < 25; i++) {
    const start = performance.now();
    result = sanitizeReader(source);
    const elapsed = performance.now() - start;
    if (i >= 5) samples.push(elapsed);
  }
  const host = document.createElement("template");
  host.innerHTML = result;
  expect(host.content.querySelectorAll("li")).toHaveLength(64);
  expect(host.content.querySelector("script,img,iframe")).toBeNull();
  samples.sort((a, b) => a - b);
  mkdirSync("artifacts", { recursive: true });
  writeFileSync(
    "artifacts/reader-html-cost.json",
    JSON.stringify(
      {
        scope:
          "JSDOM HTML preparation; not native layout, startup or paint performance",
        inputCharacters: source.length,
        outputCharacters: result.length,
        measuredIterations: samples.length,
        medianMs: samples[Math.floor(samples.length / 2)],
        p95Ms: samples[Math.ceil(samples.length * 0.95) - 1],
      },
      null,
      2,
    ),
  );
});

it("records bounded outline work against the previous full-index scan", async () => {
  const { headingBoundary, matchingHeadings } =
    await import("../src/reader/headings");
  const headings = Array.from({ length: 100000 }, (_, i) => ({
    title: `عنوان ${i}`,
    id: `h-${i}`,
    level: 2,
    section: i,
  }));
  const expected = headings
    .filter((h) => h.title.toLocaleLowerCase().includes(""))
    .slice(0, 200);
  expect(matchingHeadings(headings, "")).toEqual(expected);
  expect(headingBoundary(headings, 80000)).toBe(80000);
  const measure = (run: () => unknown) => {
    const samples = [];
    for (let i = 0; i < 10; i++) {
      const start = performance.now();
      run();
      samples.push(performance.now() - start);
    }
    samples.sort((a, b) => a - b);
    return samples[5];
  };
  const beforeMs = measure(() =>
    headings
      .filter((h) => h.title.toLocaleLowerCase().includes(""))
      .slice(0, 200),
  );
  const afterMs = measure(() => matchingHeadings(headings, ""));
  writeFileSync(
    "artifacts/reader-outline-cost.json",
    JSON.stringify(
      {
        scope:
          "JSDOM/JavaScript outline selection at 100,000 headings; not native page paint",
        headings: headings.length,
        beforeMedianMs: beforeMs,
        afterMedianMs: afterMs,
      },
      null,
      2,
    ),
  );
});
