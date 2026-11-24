import { portableFamily } from "./catalogue";
import { put, type Values } from "../preferences";

const riwaqFonts = [
  ["reader.font", "Thmanyah Serif Text"],
  ["reader.heading_font", "Thmanyah Serif Display"],
  ["editor.font", "Cousine"],
] as const;
const lineHeight = 190;

export function riwaqTypography(text: string, installed: readonly string[]) {
  for (const [key, preferred] of riwaqFonts) {
    const family =
      installed.find(
        (name) => name.toLowerCase() === preferred.toLowerCase(),
      ) ?? portableFamily(preferred);
    text = put(text, key, JSON.stringify(family));
  }
  return put(text, "reader.line_height", String(lineHeight));
}

export function riwaqStatus(
  assigned: Record<string, string>,
  effective: Values,
) {
  const selected = riwaqFonts.map(([key, preferred]) => {
    const fallback = portableFamily(preferred);
    return [
      key,
      [JSON.stringify(preferred), JSON.stringify(fallback)].find(
        (literal) =>
          literal.toLowerCase() === assigned[key]?.trim().toLowerCase(),
      ),
    ] as const;
  });
  if (
    Number(assigned["reader.line_height"]) !== lineHeight ||
    selected.some(([, font]) => !font)
  )
    return "";
  const overridden =
    effective["reader.line_height"]?.value !== lineHeight ||
    selected.some(
      ([key, font]) =>
        font?.toLowerCase() !==
        JSON.stringify(effective[key]?.value)?.toLowerCase(),
    );
  return overridden
    ? "A higher-priority scope overrides some Riwaq settings for this document."
    : "Riwaq typography is selected for this scope.";
}
