// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
const engine = vi.hoisted(() => ({ render: vi.fn() }));
vi.mock("mermaid", () => ({
  default: { initialize() {}, render: engine.render },
}));
import { clearRichCache, hydrate, renderRich } from "../../src/renderers/rich";
beforeEach(() => {
  clearRichCache();
  engine.render.mockReset();
});
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
