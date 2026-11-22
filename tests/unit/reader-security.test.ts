// @vitest-environment jsdom
import { expect, it } from "vitest";
import {
  readerImage,
  remoteImageURL,
  sanitizeReader,
  sanitizeRich,
  safeLink,
} from "../../src/reader/safe";
it("assigns unique SVG/MathML IDs while keeping references to the first original ID", () => {
  const host = document.createElement("div");
  host.innerHTML = sanitizeRich(
    '<svg><g id="same"></g><g id="same"></g><g id="next"></g><use href="#same"></use></svg><math><mrow id="same"></mrow></math>',
    "duplicates",
  );
  const ids = [...host.querySelectorAll("[id]")].map((node) => node.id);
  expect(new Set(ids).size).toBe(ids.length);
  expect(host.querySelector("use")?.getAttribute("href")).toBe("#" + ids[0]);
});
import { admissibleDiagram, renderRich } from "../../src/renderers/rich";
import { renderText } from "../../src/reader/indexer";
import { emptyContext } from "../../src/reader/grammar";
it("keeps document links delegated and converts image URLs into placeholders", () => {
  const host = document.createElement("div");
  host.innerHTML = sanitizeReader(
    renderText(
      "[x](javascript:run()) ![img](https://attacker.invalid/a) <script>run()</script>",
      emptyContext(),
    ).html,
  );
  expect(host.querySelector("script,img,iframe,object")).toBeNull();
  expect(host.querySelector("button")?.dataset.image).toBe(
    "https://attacker.invalid/a",
  );
  for (const value of [
    "javascript:run()",
    "file:///etc/passwd",
    "//remote",
    "/absolute",
    "a%2fb",
    "a\\b",
  ])
    expect(safeLink(value)).toBe(false);
  expect(safeLink("../chapter.md#heading")).toBe(true);
});
it("namespaces SVG IDs and strips CSS/resource/clobbering vectors", () => {
  const clean = sanitizeRich(
    '<svg id="__TAURI__" onload="run()"><defs><path id="a" d="M0 0"/></defs><use href="#a"/><use href="https://attacker.invalid/a"/><image href="file:///secret"/><g style="background:url(https://attacker.invalid)"></g><foreignObject><script>run()</script></foreignObject></svg>',
    "test",
  );
  expect(clean).not.toMatch(
    /https:|file:|onload|__TAURI__|foreignObject|script/,
  );
  expect(clean).toContain('href="#r-test-1"');
});
it("rejects diagram configuration, HTML, link directives and oversized layouts before import", () => {
  for (const input of [
    "%%{init:{securityLevel:loose}}%%\ngraph TD; A-->B",
    'graph TD\nclick A "https://x"',
    "graph TD\nA[<img src=x>]",
    "graph TD\n" + "a-->b\n".repeat(101),
  ])
    expect(() => admissibleDiagram(input)).toThrow();
  expect(() =>
    admissibleDiagram("graph TD\nA[Read] --> B[Write]"),
  ).not.toThrow();
});
it("renders local bounded KaTeX and rejects resource commands", async () => {
  expect(
    await renderRich({ kind: "math", source: "a^2+b^2=c^2", display: true }),
  ).toContain("katex");
  await expect(
    renderRich({
      kind: "math",
      source: "\\includegraphics{file:///etc/passwd}",
      display: true,
    }),
  ).rejects.toThrow("disabled");
});
it("uses local MathJax SVG without a loader or shared macro context", async () => {
  const output = await renderRich(
    { kind: "math", source: "\\prescript{a}{b}{X}", display: true },
    "mathjax",
  );
  expect(output).toContain("<svg");
  expect(output).toContain('role="img"');
  expect(output).toContain('aria-label="TeX math:');
  expect(
    await renderRich(
      { kind: "math", source: "\\prescript{a}{b}{X}", display: true },
      "mathjax",
    ),
  ).toContain('aria-label="TeX math:');
  expect(output.replace(/xmlns="[^"]*"/g, "")).not.toMatch(/https?:|file:/);
  await expect(
    renderRich(
      { kind: "math", source: "\\gdef\\unsupportedglobal{q}", display: true },
      "mathjax",
    ),
  ).rejects.toThrow();
  await renderRich(
    {
      kind: "math",
      source: "\\def\\localviewmacro{q}\\localviewmacro",
      display: true,
    },
    "mathjax",
  );
  await expect(
    renderRich(
      { kind: "math", source: "\\localviewmacro", display: true },
      "mathjax",
    ),
  ).rejects.toThrow();
});
it("keeps exported heading anchors aligned and admits numeric image dimensions only", () => {
  const html = sanitizeReader(
    renderText(
      "[Jump](#title)\n\n# Title\n\n![x](a.png){width=32 height=20}",
      emptyContext(),
      [{ title: "Title", id: "h-title", level: 1, section: 0 }],
    ).html,
  );
  expect(html).toContain('href="#h-title"');
  expect(html).toContain('id="h-title"');
  expect(html).toContain('data-width="32"');
  const math = renderText("$$a^2$$ and \\(b^2\\)", emptyContext());
  expect(math.rich.map((r) => r.source)).toEqual(["a^2", "b^2"]);
});

it("renders opted-in README HTML with inert image references and authentic reader controls", () => {
  const source = [
    '<details open><summary>More</summary><img src="assets/logo.png" alt="Logo" width="120" onerror="run()"></details>',
    "",
    "Press <kbd>Enter</kbd>. ![Badge](https://example.invalid/badge.svg)",
    "",
    '<div class="reader-fence-tools" data-run-kind="sh" data-rich="0" data-source-line="99">Injected</div>',
    '<img src="file:///etc/passwd" alt="Unavailable">',
    '<iframe src="https://example.invalid/"></iframe>',
    "",
    "```sh",
    "echo hello",
    "```",
  ].join("\n");
  const host = document.createElement("div");
  host.innerHTML = sanitizeReader(renderText(source, emptyContext()).html);
  expect(host.querySelector("details,kbd")).toBeNull();
  const page = renderText(source, emptyContext(), [], true, true, true);
  host.innerHTML = sanitizeReader(page.html, page.metadataKey);
  expect(host.querySelector("details[open] summary")?.textContent).toBe("More");
  expect(host.querySelector("kbd")?.textContent).toBe("Enter");
  expect(
    host.querySelector(
      "iframe,img,[src],[onerror],[data-rich],.reader-fence-tools",
    ),
  ).toBeNull();
  expect(host.querySelectorAll("[data-run-kind]")).toHaveLength(1);
  expect(host.querySelector("[data-run-kind] pre")?.textContent).toBe(
    "echo hello\n",
  );
  const logo = host.querySelector<HTMLElement>(
    'button[data-image="assets/logo.png"]',
  )!;
  expect(logo.dataset.width).toBe("120");
  const badge = host.querySelector<HTMLElement>(
    'button[data-image^="https:"]',
  )!;
  const image = readerImage(badge, remoteImageURL(badge.dataset.image!)!);
  expect(image.alt).toBe("Badge");
  expect(image.src).toBe("https://example.invalid/badge.svg");
  expect(image.referrerPolicy).toBe("no-referrer");
  for (const denied of [
    "file:///image.png",
    "//host/image.png",
    "http://host/image.png",
    "https://user:pass@host/image.png",
  ])
    expect(remoteImageURL(denied)).toBeNull();
});
