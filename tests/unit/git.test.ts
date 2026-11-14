import { describe, it, expect } from "vitest";
import { intraline, windowStart } from "../../src/git/spans";
describe("source Git presentation", () => {
  it("preserves Unicode scalar boundaries and common source around changed spans", () => {
    const result = intraline("a 🦀 old 文", "a 🦀 new 文");
    expect(result.left).toEqual(["a 🦀 ", "old", " 文"]);
    expect(result.right).toEqual(["a 🦀 ", "new", " 文"]);
  });
  it("handles insertions, deletions, empty text and identical lines literally", () => {
    expect(intraline("", "🦀").right.join("")).toBe("🦀");
    expect(intraline("<script>", "<safe>").left.join("")).toBe("<script>");
    expect(intraline("same", "same").left.join("")).toBe("same");
  });
  it("requests stable bounded display windows at scroll boundaries", () => {
    expect(windowStart(0)).toBe(0);
    expect(windowStart(2400)).toBe(0);
    expect(windowStart(4800)).toBe(100);
    expect(windowStart(240000)).toBe(9900);
  });
});
