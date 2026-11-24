import type { SettingsResult } from "../bridge";
import {
  arabicCompatibilityFamily,
  arabicFallback,
  canonicalFamily,
  fontFaces,
  portableFamily,
  systemFamilies,
} from "./catalogue";

type Values = SettingsResult["resolved"];
export type Shaping = "standard" | "webkit-compatibility";

export const sameAsBody = "Same as body";

const scriptFallbacks = [
  "Geeza Pro",
  "Noto Sans Hebrew",
  "Arial Hebrew",
  "Noto Sans CJK SC",
  "Hiragino Sans",
  "PingFang SC",
  "Apple Color Emoji",
  "Noto Color Emoji",
];

function fontStack(name: string, mono: boolean, shaping: Shaping) {
  const family = canonicalFamily(name);
  const system = systemFamilies.get(family);
  const portable = portableFamily(name);
  const requested = family === portable ? [family] : [family, portable];
  const bundled = system ? [arabicFallback] : [...requested, arabicFallback];
  if (shaping === "webkit-compatibility")
    bundled.unshift(arabicCompatibilityFamily);
  const css = [
    ...bundled.map((family) => JSON.stringify(family)),
    ...(system ?? []),
    ...scriptFallbacks.map((family) => JSON.stringify(family)),
    ...(mono ? ["monospace"] : systemFamilies.get("System UI")!),
  ];
  return { css: [...new Set(css)].join(", "), families: bundled };
}

export function resolveTypography(
  values: Values,
  shaping: Shaping = "standard",
) {
  const name = (key: string, fallback: string) =>
    String(values[key]?.value ?? fallback);
  const body = name("reader.font", "Noto Naskh Arabic");
  const heading = name("reader.heading_font", sameAsBody);
  const sourceFamily = canonicalFamily(name("editor.font", "System Mono"));
  const number = (key: string, fallback: number, min: number, max: number) =>
    Math.min(max, Math.max(min, Number(values[key]?.value) || fallback));
  const reader = {
    size: number("reader.font_size", 18, 12, 40),
    lineHeight: number("reader.line_height", 175, 120, 240) / 100,
  };
  const source = {
    family: sourceFamily,
    portable: portableFamily(sourceFamily),
    size: number("editor.font_size", 16, 11, 32),
    lineHeight: number("editor.line_height", 180, 120, 220) / 100,
  };
  const stacks = {
    reader: fontStack(body, false, shaping),
    "reader-heading": fontStack(
      heading.toLowerCase() === sameAsBody.toLowerCase() ? body : heading,
      false,
      shaping,
    ),
    source: fontStack(sourceFamily, true, shaping),
    ui: fontStack(name("ui.font", "System UI"), false, "standard"),
  };
  const families = new Set(
    Object.values(stacks).flatMap((stack) => stack.families),
  );
  const css = Object.entries(stacks)
    .map(([role, stack]) => `--${role}-font:${stack.css}`)
    .join(";");
  return {
    css: `${css};--reader-size:${reader.size}px;--reader-line-height:${reader.lineHeight};--source-size:${source.size}px;--source-line-height:${source.lineHeight}`,
    reader,
    source,
    faces: fontFaces.filter((face) => families.has(face.family)),
  };
}
