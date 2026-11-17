import type { Values } from "../preferences";

export const bundledFamilies = [
  "Noto Naskh Arabic",
  "Noto Sans Arabic",
  "Amiri",
  "Cousine",
];

export const fontFaces = [
  {
    family: "Noto Naskh Arabic",
    file: "NotoNaskhArabic[wght].ttf",
    weight: "400 700",
  },
  {
    family: "Noto Sans Arabic",
    file: "NotoSansArabic[wdth,wght].ttf",
    weight: "400 700",
  },
  { family: "Amiri", file: "Amiri-Regular.ttf", weight: "400" },
  { family: "Amiri", file: "Amiri-Bold.ttf", weight: "700" },
  { family: "Cousine", file: "Cousine-Regular.ttf", weight: "400" },
  { family: "Cousine", file: "Cousine-Bold.ttf", weight: "700" },
];

const alternatives: Record<string, string> = {
  "thmanyah serif text": "Noto Naskh Arabic",
  "thmanyah serif display": "Amiri",
  "thmanyah sans": "Noto Sans Arabic",
};
export function portableFamily(name: string) {
  const folded = name.toLowerCase();
  return (
    alternatives[folded] ??
    bundledFamilies.find((family) => family.toLowerCase() === folded) ??
    name
  );
}

// Prepare explicitly selected bundled faces before showing a settings preview.
// Native installed fonts remain native; no font path or download is accepted.
export async function readyFontChoices(values: Values) {
  if (!document.fonts) return;
  const choices = [
    "reader.font",
    "reader.heading_font",
    "ui.font",
    "editor.font",
  ].map((key) => String(values[key]?.value ?? "").toLowerCase());
  const names = bundledFamilies.filter((family) =>
    choices.includes(family.toLowerCase()),
  );
  await Promise.all(
    names.flatMap((name) =>
      [400, 700].map((weight) =>
        document.fonts.load(
          `${weight} 18px ${JSON.stringify(name)}`,
          "العربية English",
        ),
      ),
    ),
  );
}
