// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
const engine = vi.hoisted(() => ({ render: vi.fn() }));
vi.mock("mermaid", () => ({
  default: { initialize() {}, render: engine.render },
}));
import { clearRichCache, renderRich } from "../../src/renderers/rich";
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
