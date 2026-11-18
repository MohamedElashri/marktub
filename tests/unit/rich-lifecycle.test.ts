// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const engine = vi.hoisted(() => ({ render: vi.fn() }));
vi.mock("mermaid", () => ({
  default: { initialize() {}, render: engine.render },
}));
import { clearRichCache, hydrate, renderRich } from "../../src/renderers/rich";
beforeEach(() => {
  clearRichCache();
  engine.render.mockReset();
});
afterEach(() => vi.restoreAllMocks());
const diagram = (n: number) => ({
  kind: "mermaid" as const,
  source: `graph LR\nA-->B${n}`,
  display: true,
});

it("does not start a retired queued diagram and lets the next live job run", async () => {
  let finish!: (value: { svg: string }) => void;
  engine.render
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValue({ svg: "<svg><text>live</text></svg>" });
  const first = renderRich(diagram(1));
  await vi.waitFor(() => expect(engine.render).toHaveBeenCalledTimes(1));
  let live = true;
  const second = renderRich(diagram(2), "auto", () => live);
  const canceled = expect(second).rejects.toMatchObject({ name: "AbortError" });
  live = false;
  finish({ svg: "<svg><text>first</text></svg>" });
  await first;
  await canceled;
  expect(engine.render).toHaveBeenCalledTimes(1);
  expect(await renderRich(diagram(3))).toContain("live");
  expect(engine.render).toHaveBeenCalledTimes(2);
});

it("shares queued identical work without double charging the bounded cache", async () => {
  engine.render.mockResolvedValue({
    svg: `<svg><text>${"x".repeat(200_000)}</text></svg>`,
  });
  await Promise.all(Array.from({ length: 4 }, () => renderRich(diagram(0))));
  expect(engine.render).toHaveBeenCalledTimes(1);
  for (let n = 1; n <= 17; n++) await renderRich(diagram(n));
  const count = engine.render.mock.calls.length;
  await renderRich(diagram(0));
  expect(engine.render).toHaveBeenCalledTimes(count);
});

it("keeps KaTeX decorative SVG under its hidden HTML alternative", async () => {
  const host = document.createElement("div");
  host.innerHTML = await renderRich({
    kind: "math",
    source: "\\sqrt{x}",
    display: true,
  });
  expect(host.querySelector("math")).not.toBeNull();
  expect(host.querySelector('.katex-html[aria-hidden="true"]')).not.toBeNull();
  expect(host.querySelector('svg[role="img"]')).toBeNull();
});

it("hydrates every small formula after the fourth rich block in a section", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const items = Array.from({ length: 9 }, (_, i) => ({
    kind: "math" as const,
    source: i === 8 ? "a^2+b^2=c^2" : `x_${i}`,
    display: false,
  }));
  host.innerHTML = items
    .map((_, i) => `<span data-rich="${i}"></span>`)
    .join(" ");
  try {
    await hydrate(host, items, () => true);
    expect(host.querySelectorAll(".katex")).toHaveLength(9);
    expect(host.querySelector(".render-error")).toBeNull();
    expect(host.lastElementChild?.textContent).toContain("a");
  } finally {
    host.remove();
  }
});

it("bounds aggregate rich output and preserves the remaining source", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const items = Array.from({ length: 12 }, (_, i) => ({
    kind: "math" as const,
    source: `x_${i}`,
    display: false,
    mathIndex: i,
  }));
  host.innerHTML = items
    .map((_, i) => `<span data-rich="${i}"></span>`)
    .join("");
  const render = vi.fn(async () => `<span>${"x".repeat(700_000)}</span>`);
  try {
    await hydrate(host, items, () => true, { render } as never);
    expect(render.mock.calls.length).toBeLessThan(12);
    expect(host.querySelector(".render-error")?.textContent).toContain("8 MiB");
    expect(host.lastElementChild?.textContent).toContain("x_11");
  } finally {
    host.remove();
  }
});

it("defers offscreen rich output and hydrates it once when it becomes visible", async () => {
  const host = document.createElement("div");
  host.innerHTML = '<span data-rich="0"></span><span data-rich="1"></span>';
  document.body.append(host);
  const items = [diagram(100), diagram(101)];
  engine.render.mockResolvedValue({ svg: "<svg><text>visible</text></svg>" });
  try {
    await hydrate(
      host,
      items,
      () => true,
      undefined,
      (node) => node.dataset.rich === "0",
    );
    expect(engine.render).toHaveBeenCalledTimes(1);
    expect(host.children[1].innerHTML).toBe("");
    await hydrate(
      host,
      items,
      () => true,
      undefined,
      () => true,
    );
    expect(engine.render).toHaveBeenCalledTimes(2);
    await hydrate(
      host,
      items,
      () => true,
      undefined,
      () => true,
    );
    expect(engine.render).toHaveBeenCalledTimes(2);
  } finally {
    host.remove();
  }
});

it("lets queued input retire a batch of cached rich output", async () => {
  engine.render.mockResolvedValue({ svg: "<svg><text>cached</text></svg>" });
  await renderRich(diagram(0));
  const host = document.createElement("div");
  const items = Array.from({ length: 40 }, () => diagram(0));
  host.innerHTML = items
    .map((_, i) => `<span data-rich="${i}"></span>`)
    .join("");
  document.body.append(host);
  let live = true;
  let clock = 0;
  vi.spyOn(performance, "now").mockImplementation(() => (clock += 2));
  const input = setTimeout(() => {
    live = false;
  }, 0);
  try {
    await hydrate(host, items, () => live);
    expect(host.querySelectorAll("svg").length).toBeLessThan(items.length);
    expect(live).toBe(false);
    expect(engine.render).toHaveBeenCalledTimes(1);
  } finally {
    clearTimeout(input);
    host.remove();
  }
});

it("leaves rich blocks retryable when queued scrolling moves them offscreen", async () => {
  engine.render.mockResolvedValue({ svg: "<svg><text>cached</text></svg>" });
  await renderRich(diagram(0));
  const host = document.createElement("div");
  const items = Array.from({ length: 40 }, () => diagram(0));
  host.innerHTML = items
    .map((_, i) => `<span data-rich="${i}"></span>`)
    .join("");
  document.body.append(host);
  let visible = true;
  let clock = 0;
  vi.spyOn(performance, "now").mockImplementation(() => (clock += 2));
  const scroll = setTimeout(() => {
    visible = false;
  }, 0);
  try {
    await hydrate(
      host,
      items,
      () => true,
      undefined,
      () => visible,
    );
    const rendered = host.querySelectorAll("svg").length;
    expect(rendered).toBeLessThan(items.length);
    expect(host.querySelectorAll('[data-hydrated="true"]')).toHaveLength(
      rendered,
    );
    visible = true;
    await hydrate(
      host,
      items,
      () => true,
      undefined,
      () => visible,
    );
    expect(host.querySelectorAll("svg")).toHaveLength(items.length);
    expect(engine.render).toHaveBeenCalledTimes(1);
  } finally {
    clearTimeout(scroll);
    host.remove();
  }
});

it("rechecks code visibility after yielding and highlights deferred blocks later", async () => {
  await import("../../src/renderers/highlight");
  const host = document.createElement("div");
  host.innerHTML =
    '<pre><code class="language-js">const x = 1;</code></pre>'.repeat(40);
  document.body.append(host);
  let visible = true;
  let clock = 0;
  vi.spyOn(performance, "now").mockImplementation(() => (clock += 2));
  const scroll = setTimeout(() => {
    visible = false;
  }, 0);
  try {
    await hydrate(
      host,
      [],
      () => true,
      undefined,
      () => visible,
    );
    expect(
      host.querySelectorAll('[data-highlighted="true"]').length,
    ).toBeLessThan(40);
    visible = true;
    await hydrate(
      host,
      [],
      () => true,
      undefined,
      () => visible,
    );
    expect(host.querySelectorAll('[data-highlighted="true"]')).toHaveLength(40);
    expect(host.querySelectorAll(".hljs-keyword")).toHaveLength(40);
  } finally {
    clearTimeout(scroll);
    host.remove();
  }
});
