import { describe, expect, it } from "vitest";
import { FlowLayout, readerWindow } from "../../src/reader/flow";

const sections = (count: number) =>
  Array.from({ length: count }, (_, i) => ({
    start: i * 4096,
    end: (i + 1) * 4096,
  }));
describe("continuous reader geometry", () => {
  it("locates measured paragraph boundaries and updates earlier estimates", () => {
    const flow = new FlowLayout();
    flow.resize(sections(5));
    [100, 200, 50, 300, 100].forEach((height, i) => flow.set(i, height));
    expect(flow.offset(3)).toBe(350);
    expect(flow.indexAt(349)).toBe(2);
    expect(flow.indexAt(350)).toBe(3);
    flow.set(0, 200);
    expect(flow.offset(3)).toBe(450);
    expect(flow.indexAt(449)).toBe(2);
    expect(flow.indexAt(Infinity)).toBe(4);
    expect(flow.indexAt(-1)).toBe(0);
  });
  it("retains measured heights when the provisional index grows or shrinks", () => {
    const flow = new FlowLayout();
    flow.resize(sections(3));
    flow.set(0, 123);
    flow.resize(sections(10));
    expect(flow.offset(1)).toBe(123);
    flow.resize(sections(1));
    expect(flow.offset(10)).toBe(123);
    flow.resize(sections(10));
    expect(flow.offset(10)).toBeCloseTo(123 + 9 * 4096 * 0.55);
  });
  it("bounds the live window at the beginning, middle and end of a 100,000-block index", () => {
    const flow = new FlowLayout();
    flow.resize(sections(100000));
    for (const index of [0, 1, 50000, 99999]) {
      const { from, to } = readerWindow(flow, index, 800);
      expect(from).toBeLessThanOrEqual(index);
      expect(to).toBeGreaterThan(index);
      expect(to - from).toBeLessThanOrEqual(25);
      expect(flow.indexAt(flow.offset(index))).toBe(index);
    }
  });
  it("compresses distant spacers without scaling rendered paragraphs or losing long jumps", () => {
    const flow = new FlowLayout();
    flow.resize(sections(100000));
    const from = 50000,
      to = 50010;
    const { before, after, scale } = flow.spacers(from, to);
    const inside = flow.offset(to) - flow.offset(from);
    expect(scale).toBeLessThan(1);
    expect(before + inside + after).toBeCloseTo(8_000_000);
    expect(flow.atScroll(before + 50, from, to)).toBe(from);
    expect(flow.atScroll(flow.offset(100) * scale, from, to)).toBe(100);
    expect(
      flow.atScroll(
        before + inside + (flow.offset(90000) - flow.offset(to)) * scale,
        from,
        to,
      ),
    ).toBe(90000);
  });
  it("keeps a valid window around zero-height definition-only chunks", () => {
    const flow = new FlowLayout();
    flow.resize(sections(100));
    for (let i = 0; i < 100; i++) flow.set(i, 0);
    const range = readerWindow(flow, 20, 600);
    expect(range.from).toBeLessThanOrEqual(20);
    expect(range.to).toBeGreaterThan(20);
    expect(range.to - range.from).toBeLessThanOrEqual(25);
  });
  it("rejects invalid measurements without corrupting later offsets", () => {
    const flow = new FlowLayout();
    flow.resize(sections(2));
    const total = flow.offset(2);
    flow.set(0, NaN);
    flow.set(1, Infinity);
    flow.set(2, 20);
    expect(flow.offset(2)).toBe(total);
  });
});
