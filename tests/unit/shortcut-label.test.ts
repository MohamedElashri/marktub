// @vitest-environment jsdom
import { expect, it } from "vitest";
import { shortcutLabel, matches } from "../../src/commands";
it("displays native shortcut symbols without changing canonical matching", () => {
  expect(shortcutLabel("Mod+Shift+P", true)).toEqual({
    visual: "⇧⌘P",
    spoken: "Shift plus Command plus P",
  });
  expect(shortcutLabel("Mod+Shift+P", false).visual).toBe("Ctrl+Shift+P");
  expect(shortcutLabel("Mod+Comma", true).visual).toBe("⌘,");
  expect(shortcutLabel("Alt+ArrowLeft", false).visual).toBe("Alt+←");
  expect(shortcutLabel("none", true).visual).toBe("");
  expect(
    matches(
      new KeyboardEvent("keydown", {
        key: "P",
        code: "KeyP",
        metaKey: true,
        shiftKey: true,
      }),
      "Mod+Shift+P",
      true,
    ),
  ).toBe(true);
});
