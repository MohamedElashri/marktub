import { describe, expect, it } from "vitest";
import { sessionSelection } from "../../src/tabs";

describe("restorable tab selection", () => {
  it("keeps the selected real document when empty tabs precede and follow it", () => {
    expect(sessionSelection([null, "first", "second", null], 1)).toEqual({
      ids: ["first", "second"],
      active: 0,
    });
    expect(sessionSelection([null, "first", "second", null], 2)).toEqual({
      ids: ["first", "second"],
      active: 1,
    });
  });
  it("never restores a recovery path and chooses the preceding real tab for an empty selection", () => {
    expect(sessionSelection(["first", "recovery", null, "second"], 2)).toEqual({
      ids: ["first", "second"],
      active: 0,
    });
    expect(sessionSelection([null, "recovery", "only"], 0)).toEqual({
      ids: ["only"],
      active: 0,
    });
  });
  it("writes an empty session without an invalid index after closing all documents", () => {
    expect(sessionSelection([null, "recovery"], 1)).toEqual({
      ids: [],
      active: 0,
    });
  });
});
