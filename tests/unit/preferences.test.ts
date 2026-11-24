// @vitest-environment jsdom
import { describe, it, expect } from "vitest";
import {
  appearance,
  themes,
  put,
  literals,
  type Values,
} from "../../src/preferences";
import { fontChoices } from "../../src/fonts/catalogue";
import { riwaqStatus, riwaqTypography } from "../../src/fonts/presets";
import { resolveTypography } from "../../src/fonts/typography";
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
  it("ships readable light/dark palettes including syntax and diff specimens", () => {
    expect(
      Object.values(themes).filter((t) => t.mode === "light"),
    ).toHaveLength(3);
    expect(Object.values(themes).filter((t) => t.mode === "dark")).toHaveLength(
      3,
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
    expect(resolveTypography(values).css).toContain('x\\";background:url');
  });
  it("resolves case-insensitive bundled and system choices with script fallbacks", () => {
    const settings = (family: string): Values => ({
      "reader.font": { type: "text", value: family },
    });
    for (const family of ["System UI", "System Mono", "Serif", "Sans Serif"]) {
      const css = resolveTypography(settings(family)).css;
      expect(css).toMatch(/^--reader-font:"Marktub Arabic",/);
      expect(css).toContain('"Noto Sans Hebrew"');
      expect(css).toBe(resolveTypography(settings(family.toLowerCase())).css);
    }
    for (const family of [
      "Amiri",
      "Noto Naskh Arabic",
      "Noto Sans Arabic",
      "constructor",
    ])
      expect(resolveTypography(settings(family)).css).toMatch(
        new RegExp(`^--reader-font:"${family}",`),
      );
    expect(
      resolveTypography({
        "editor.font": { type: "text", value: "Cousine" },
      }).css,
    ).toContain('--source-font:"Cousine", "Marktub Arabic",');
    expect(
      fontChoices(["amiri", "Noto Serif", "noto serif"]).filter((name) =>
        ["amiri", "noto serif"].includes(name.toLowerCase()),
      ),
    ).toEqual(["Amiri", "Noto Serif"]);
    expect(appearance({})).toContain(
      "--source-size:16px;--source-line-height:1.8",
    );
    expect(
      appearance({
        "editor.font_size": { type: "number", value: 21 },
        "editor.line_height": { type: "number", value: 200 },
      }),
    ).toContain("--source-size:21px;--source-line-height:2");
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
  it("keeps color choices independent and applies portable typography only on request", () => {
    const original =
      '$theme=riwaq_night\n$reader.font_size=23\n$reader.width=640\n$reader.font="My font"\n$unknown="keep"';
    const selected = literals(riwaqTypography(original, []));
    expect(selected.theme).toBe("riwaq_night");
    expect(selected["reader.font_size"]).toBe("23");
    expect(selected["reader.width"]).toBe("640");
    expect(selected.unknown).toBe('"keep"');
    expect(selected["reader.font"]).toBe('"Noto Naskh Arabic"');
    expect(selected["reader.heading_font"]).toBe('"Amiri"');
    expect(selected["editor.font"]).toBe('"Cousine"');
    expect(selected["reader.line_height"]).toBe("190");
    const installed = literals(
      riwaqTypography(original, [
        "Thmanyah Serif Text",
        "Thmanyah Sans",
        "Thmanyah Serif Display",
      ]),
    );
    expect(installed["reader.font"]).toBe('"Thmanyah Serif Text"');
    expect(installed["reader.heading_font"]).toBe('"Thmanyah Serif Display"');
    expect(installed["ui.font"]).toBeUndefined();
    expect(
      literals(riwaqTypography('$ui.font="System UI"', []))["ui.font"],
    ).toBe('"System UI"');
    const values: Values = {
      theme: { type: "enum", value: "riwaq_day" },
      "reader.font": { type: "text", value: "My font" },
    };
    const css = appearance(values);
    expect(css).toContain('--reader-font:"My font"');
    expect(css).toContain('--reader-heading-font:"My font"');
    expect(appearance({})).toContain('--reader-font:"Noto Naskh Arabic"');
    const compatibility = appearance(
      values,
      false,
      resolveTypography(values, "webkit-compatibility"),
    );
    for (const role of ["reader", "reader-heading", "source"])
      expect(compatibility).toContain(
        `--${role}-font:"Marktub Arabic Compatibility", `,
      );
    expect(css.split("--ui-font:")[1]?.split(";")[0]).toBe(
      appearance({}).split("--ui-font:")[1]?.split(";")[0],
    );
    expect(
      resolveTypography({
        "reader.heading_font": {
          type: "text",
          value: "Thmanyah Serif Display",
        },
      }).css,
    ).toContain('"Thmanyah Serif Display", "Amiri"');
    expect(css).not.toContain("direction:");
  });
  it("reports the Riwaq preset when a higher-priority scope masks any of its choices", () => {
    const assigned = literals(riwaqTypography("", []));
    const effective: Values = {
      "reader.font": { type: "text", value: "Noto Naskh Arabic" },
      "reader.heading_font": { type: "text", value: "Amiri" },
      "reader.line_height": { type: "number", value: 190 },
      "editor.font": { type: "text", value: "Cousine" },
    };
    expect(riwaqStatus(assigned, effective)).toBe(
      "Riwaq typography is selected for this scope.",
    );
    expect(
      riwaqStatus(assigned, {
        ...effective,
        "editor.font": { type: "text", value: "System Mono" },
      }),
    ).toContain("higher-priority scope");
    expect(riwaqStatus({}, effective)).toBe("");
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
