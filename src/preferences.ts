import { portableFamily } from "./fonts/catalogue";
import { invoke } from "@tauri-apps/api/core";
import { readable } from "svelte/store";
import type { SettingsResult, SettingValue } from "./bridge";
export type Values = Record<string, SettingValue>;
export interface Spec {
  key: string;
  label: string;
  category: string;
  kind: string;
  default: string;
  min: number | null;
  max: number | null;
  options: string[];
}
export const catalogue = () => invoke<Spec[]>("preference_catalogue");
export const fonts = () => invoke<string[]>("list_fonts");
export const preview = (id: string | null, scope: string, text: string) =>
  invoke<SettingsResult>("preview_settings", { id, scope, text });
export const osDark = readable(false, (set) => {
  const media = globalThis.matchMedia?.("(prefers-color-scheme: dark)");
  if (!media) return;
  set(media.matches);
  const changed = () => set(media.matches);
  media.addEventListener("change", changed);
  return () => media.removeEventListener("change", changed);
});
const keys = [
  "paper",
  "ink",
  "quiet",
  "accent",
  "on_accent",
  "line",
  "surface",
  "selection",
  "code",
  "keyword",
  "string",
  "number",
  "added",
  "added_bg",
  "removed",
  "removed_bg",
];
const palette = (values: string[]) =>
  Object.fromEntries(keys.map((key, i) => [key, values[i]]));
export const themes: Record<
  string,
  { label: string; mode: "light" | "dark"; colors: Record<string, string> }
> = {
  light: {
    label: "Light",
    mode: "light",
    colors: palette([
      "#ffffff",
      "#24262d",
      "#5b6170",
      "#3d55b4",
      "#ffffff",
      "#d9dce5",
      "#f3f4f7",
      "#dce3fa",
      "#f3f4f7",
      "#74409a",
      "#345c4b",
      "#87521e",
      "#17532e",
      "#e2f2e7",
      "#922c33",
      "#fce7e9",
    ]),
  },
  paper: {
    label: "Parchment",
    mode: "light",
    colors: palette([
      "#fbf5e9",
      "#382e23",
      "#6e5c48",
      "#825013",
      "#ffffff",
      "#d5c9b5",
      "#f1e8d8",
      "#e2cfab",
      "#f1e8d8",
      "#744069",
      "#365b36",
      "#7a481c",
      "#24532b",
      "#e3efde",
      "#8c2930",
      "#fae6df",
    ]),
  },
  dark: {
    label: "Dark",
    mode: "dark",
    colors: palette([
      "#202127",
      "#ebeaf0",
      "#b3b3bf",
      "#bcc7ff",
      "#222944",
      "#494b59",
      "#2b2d35",
      "#434963",
      "#2b2d35",
      "#d6b9f5",
      "#bad8c4",
      "#edc99a",
      "#ace2b8",
      "#1e3a29",
      "#ffbdc3",
      "#43272d",
    ]),
  },
  riwaq_day: {
    label: "Riwaq Day",
    mode: "light",
    colors: palette([
      "#faf6ee",
      "#2d2a24",
      "#6d6253",
      "#825f08",
      "#ffffff",
      "#d4c8a8",
      "#eee5d6",
      "#ead5a4",
      "#eee5d6",
      "#744069",
      "#345c36",
      "#7a481c",
      "#24532b",
      "#e3efde",
      "#8c2930",
      "#fae6df",
    ]),
  },
  riwaq_night: {
    label: "Riwaq Night",
    mode: "dark",
    colors: palette([
      "#1a1a2e",
      "#e0d8c8",
      "#aaa0b6",
      "#d4a853",
      "#1a1a2e",
      "#474358",
      "#24243a",
      "#444058",
      "#24243a",
      "#d6b9f5",
      "#bad8c4",
      "#edc99a",
      "#ace2b8",
      "#1e3a29",
      "#ffbdc3",
      "#43272d",
    ]),
  },
  midnight: {
    label: "Ink",
    mode: "dark",
    colors: palette([
      "#17202c",
      "#e2e9f5",
      "#aabbd3",
      "#a6c9ff",
      "#132849",
      "#40526e",
      "#222f43",
      "#3e5579",
      "#222f43",
      "#d7bcff",
      "#a7dcb6",
      "#f6ca8a",
      "#b0e2c2",
      "#203c33",
      "#ffc1ce",
      "#462b3d",
    ]),
  },
};
export function value(
  values: Values,
  key: string,
  fallback: string | number | boolean,
) {
  return values[key]?.value ?? fallback;
}
export function currentPalette(values: Values, dark = false) {
  const selected = String(value(values, "theme", "system"));
  if (selected === "custom") {
    return {
      label: "Custom",
      mode: value(values, "theme.custom_mode", "light") as "light" | "dark",
      colors: Object.fromEntries(
        keys.map((key) => [
          key,
          String(value(values, `color.${key}`, themes.light.colors[key])),
        ]),
      ),
    };
  }
  return (
    themes[selected === "system" ? (dark ? "dark" : "light") : selected] ??
    themes.light
  );
}
export function fontStack(name: string, mono = false) {
  const ui = '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
  const fallback = mono
    ? '"SFMono-Regular", Menlo, "DejaVu Sans Mono", "Noto Sans Mono", monospace'
    : ui;
  const special: Record<string, string> = {
    "system ui": ui,
    "system mono": '"SFMono-Regular", Menlo, "DejaVu Sans Mono", monospace',
    serif: "serif",
    "sans serif": "sans-serif",
  };
  const family = portableFamily(name);
  const face =
    family.toLowerCase() !== name.toLowerCase()
      ? `${JSON.stringify(name)}, ${JSON.stringify(family)}`
      : JSON.stringify(family);
  return `${special[name.toLowerCase()] ?? face}, "Marktub Arabic", "Geeza Pro", "Noto Sans Hebrew", "Arial Hebrew", "Noto Sans CJK SC", "Hiragino Sans", "PingFang SC", "Apple Color Emoji", "Noto Color Emoji", ${mono ? "monospace" : fallback}`;
}
export function appearance(values: Values, dark = false) {
  const p = currentPalette(values, dark);
  const body = String(value(values, "reader.font", "Palatino"));
  const heading = String(value(values, "reader.heading_font", "Same as body"));
  const color = (k: string) =>
    /^#[0-9a-f]{6}$/i.test(p.colors[k]) ? p.colors[k] : themes.light.colors[k];
  const number = (k: string, f: number, min: number, max: number) =>
    Math.min(max, Math.max(min, Number(value(values, k, f)) || f));
  const css = keys
    .map((key) => `--${key.replaceAll("_", "-")}:${color(key)}`)
    .join(";");
  return `${css};color-scheme:${p.mode};--reader-font:${fontStack(body)};--reader-heading-font:${fontStack(heading.toLowerCase() === "same as body" ? body : heading)};--ui-font:${fontStack(String(value(values, "ui.font", "System UI")))};--source-font:${fontStack(String(value(values, "editor.font", "System Mono")), true)};--reader-size:${number("reader.font_size", 18, 12, 40)}px;--reader-width:${number("reader.width", 760, 400, 1600)}px;--reader-line-height:${number("reader.line_height", 175, 120, 240) / 100};--source-size:${number("editor.font_size", 14, 11, 32)}px;--source-line-height:${number("editor.line_height", 165, 120, 220) / 100};--ui-factor:${number("ui.scale", 100, 80, 150) / 100};font-family:var(--ui-font);`;
}
// Keep unknown/duplicate literal lines visible for native diagnostics; never evaluate text.
export function put(text: string, key: string, literal: string | null) {
  const lines = text.split(/\r?\n/);
  const matches = lines
    .map((line, i) => (line.trim().split("=")[0].trim() === `$${key}` ? i : -1))
    .filter((i) => i >= 0);
  if (literal === null)
    return lines.filter((_, i) => !matches.includes(i)).join("\n");
  if (matches.length) {
    lines[matches[0]] = `$${key}=${literal}`;
  } else lines.push(`$${key}=${literal}`);
  return lines.join("\n");
}
export function literals(text: string) {
  const result: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const match = /^\s*\$([\w.-]+)\s*=\s*(.*)$/.exec(line);
    if (match) result[match[1]] = match[2];
  }
  return result;
}

export function riwaqTypography(text: string, installed: readonly string[]) {
  for (const [key, preferred] of [
    ["reader.font", "Thmanyah Serif Text"],
    ["reader.heading_font", "Thmanyah Serif Display"],
    ["ui.font", "Thmanyah Sans"],
    ["editor.font", "Cousine"],
  ]) {
    const family = installed.some(
      (name) => name.toLowerCase() === preferred.toLowerCase(),
    )
      ? preferred
      : portableFamily(preferred);
    text = put(text, key, JSON.stringify(family));
  }
  text = put(text, "reader.line_height", "190");
  return text;
}
