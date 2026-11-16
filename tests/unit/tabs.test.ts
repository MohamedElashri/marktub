import { describe, expect, it } from "vitest";
import { sessionSelection, moveTab } from "../../src/tabs";
import { libraryWidth } from "../../src/workspace";

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

describe("workspace layout", () => {
  it("moves tabs without replacing their editor, dirty state or selected document", () => {
    const editor = { undo: ["unsaved paragraph"], caret: 7 };
    const tabs = [
      { key: 10, document: "first", editor, dirty: true },
      { key: 20, document: "second", dirty: false },
      { key: 30, document: "third", dirty: false },
    ];
    const result = moveTab(tabs, 1, 10, 2);
    expect(result.tabs.map((tab) => tab.key)).toEqual([20, 30, 10]);
    expect(result.tabs[result.active]).toBe(tabs[1]);
    expect(result.tabs[2]).toBe(tabs[0]);
    expect(result.tabs[2].editor).toBe(editor);
    expect(
      sessionSelection(
        result.tabs.map((tab) => tab.document),
        result.active,
      ),
    ).toEqual({ ids: ["second", "third", "first"], active: 0 });
    expect(tabs.map((tab) => tab.key)).toEqual([10, 20, 30]);
  });
  it("keeps the active tab by key when moving it across empty or recovery tabs", () => {
    const tabs = [
      { key: 0, document: null },
      { key: 1, document: "recovery" },
      { key: 2, document: "note" },
    ];
    const result = moveTab(tabs, 2, 2, 0);
    expect(result.active).toBe(0);
    expect(result.tabs[0]).toBe(tabs[2]);
    expect(
      sessionSelection(
        result.tabs.map((tab) => tab.document),
        result.active,
      ),
    ).toEqual({ ids: ["note"], active: 0 });
    expect(moveTab(tabs, 2, 999, 0).tabs).toBe(tabs);
    expect(moveTab(tabs, 2, 2, NaN).tabs).toBe(tabs);
    expect(moveTab(tabs, 2, 2, 2).tabs).toBe(tabs);
    expect(moveTab(tabs, 2, 2, -100).active).toBe(0);
  });
  it("bounds library resizing while leaving space for the document", () => {
    expect(libraryWidth(100)).toBe(220);
    expect(libraryWidth(1000)).toBe(600);
    expect(libraryWidth(600, 800)).toBe(440);
    expect(libraryWidth(600, 640)).toBe(280);
    expect(libraryWidth(NaN)).toBe(280);
    expect(libraryWidth(321.7)).toBe(322);
  });
});
