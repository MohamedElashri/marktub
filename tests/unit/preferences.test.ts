// @vitest-environment jsdom
import { describe, it, expect } from "vitest";
import {
  appearance,
  themes,
  put,
  literals,
  fontStack,
  type Values,
} from "../../src/preferences";
import { matches, score, definitions } from "../../src/commands";
const contrast = (a: string, b: string) => {
  const light = (s: string) => {
    const c = [1, 3, 5]
      .map((i) => parseInt(s.slice(i, i + 2), 16) / 255)
      .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  };
  return (
    (Math.max(light(a), light(b)) + 0.05) /
    (Math.min(light(a), light(b)) + 0.05)
  );
};
describe("presentation and command contracts", () => {
  it("ships four readable light/dark palettes including syntax and diff specimens", () => {
    expect(
      Object.values(themes).filter((t) => t.mode === "light"),
    ).toHaveLength(2);
    expect(Object.values(themes).filter((t) => t.mode === "dark")).toHaveLength(
      2,
    );
    for (const theme of Object.values(themes)) {
      for (const [a, b] of [
        ["ink", "paper"],
        ["quiet", "paper"],
        ["accent", "paper"],
        ["on_accent", "accent"],
        ["ink", "surface"],
        ["ink", "selection"],
        ["ink", "code"],
        ["quiet", "code"],
        ["accent", "surface"],
        ["keyword", "paper"],
        ["string", "paper"],
        ["number", "paper"],
        ["keyword", "code"],
        ["string", "code"],
        ["number", "code"],
        ["added", "added_bg"],
        ["removed", "removed_bg"],
      ])
        expect(
          contrast(theme.colors[a], theme.colors[b]),
          `${theme.label} ${a}/${b}`,
        ).toBeGreaterThanOrEqual(4.5);
    }
  });
  it("uses system appearance and bounded typed styles without interpolating CSS font values", () => {
    expect(appearance({}, true)).toContain(themes.dark.colors.paper);
    expect(appearance({}, false)).toContain(themes.light.colors.paper);
    const values: Values = {
      "ui.scale": { type: "number", value: 900 },
      "reader.font": { type: "text", value: 'x";background:url(https://evil)' },
    };
    expect(appearance(values)).toContain("--ui-factor:1.5");
    expect(fontStack('x";background:blue')).toContain('x\\";background:blue');
    expect(fontStack("System Mono", true)).toContain("Noto Color Emoji");
    expect(fontStack("System UI", true)).toMatch(/^-apple-system/);
    expect(fontStack("Serif")).toContain("Noto Sans Hebrew");
  });
  it("edits one literal override without hiding duplicate/invalid diagnostics", () => {
    const text = '# original\n$reader.width=760\n$unknown="keep"';
    expect(put(text, "reader.width", "900")).toContain('$unknown="keep"');
    expect(literals(put(text, "theme", "dark")).theme).toBe("dark");
    expect(put("$theme=dark\n$theme=light", "theme", "paper")).toBe(
      "$theme=paper\n$theme=light",
    );
    expect(put(text, "reader.width", null)).not.toContain("reader.width");
  });
  it("matches physical letters, portable modifiers and suppresses IME/AltGraph", () => {
    expect(
      matches(
        new KeyboardEvent("keydown", { code: "KeyS", key: "س", metaKey: true }),
        "Mod+S",
        true,
      ),
    ).toBe(true);
    expect(
      matches(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
        "Mod+S",
        false,
      ),
    ).toBe(true);
    expect(
      matches(
        new KeyboardEvent("keydown", {
          key: "s",
          ctrlKey: true,
          isComposing: true,
        }),
        "Mod+S",
        false,
      ),
    ).toBe(false);
    expect(
      matches(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
        "none",
        false,
      ),
    ).toBe(false);
  });
  it("keeps AltGraph and composition keycode events out of app shortcuts", () => {
    const altGraph = new KeyboardEvent("keydown", {
      code: "KeyS",
      key: "s",
      ctrlKey: true,
      altKey: true,
    });
    Object.defineProperty(altGraph, "getModifierState", {
      value: (modifier: string) => modifier === "AltGraph",
    });
    expect(matches(altGraph, "Ctrl+Alt+S", false)).toBe(false);
    expect(
      matches(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true, keyCode: 229 }),
        "Mod+S",
        false,
      ),
    ).toBe(false);
  });
  it("ranks contextual actions with finite fuzzy filtering", () => {
    expect(score("Save As Files", "svs")).toBeLessThan(Infinity);
    expect(score("Open file", "zz")).toBe(Infinity);
    expect(new Set(definitions.map((d) => d[0])).size).toBe(definitions.length);
  });
});
