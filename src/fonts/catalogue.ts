type Face = {
  family: string;
  file: string;
  weight: string;
  unicodeRange?: string;
};

export const systemFamilies = new Map<string, readonly string[]>([
  [
    "System UI",
    ["-apple-system", "BlinkMacSystemFont", '"Segoe UI"', "sans-serif"],
  ],
  [
    "System Mono",
    ['"SFMono-Regular"', "Menlo", '"DejaVu Sans Mono"', "monospace"],
  ],
  ["Serif", ["serif"]],
  ["Sans Serif", ["sans-serif"]],
]);

const bundled: Face[] = [
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

export const arabicFallback = "Marktub Arabic";
export const arabicCompatibilityFamily = "Marktub Arabic Compatibility";
export const arabicProbeFamily = "Noto Naskh Arabic";
const arabicRange =
  "U+0600-06FF,U+0750-077F,U+0870-08FF,U+FB50-FDFF,U+FE70-FEFF,U+10EC0-10EFF,U+1EE00-1EEFF";

export const fontFaces: readonly Face[] = [
  ...bundled,
  ...bundled
    .filter(({ family }) => family === "Noto Sans Arabic")
    .map((face) => ({
      ...face,
      family: arabicFallback,
      unicodeRange: arabicRange,
    })),
  ...bundled
    .filter(({ family }) => family === "Amiri")
    .map((face) => ({
      ...face,
      family: arabicCompatibilityFamily,
      unicodeRange: `${arabicRange},U+200C-200D`,
    })),
];

const familyChoices = [
  ...new Set([...bundled.map((face) => face.family), ...systemFamilies.keys()]),
];
const canonicalFamilies = new Map(
  familyChoices.map((family) => [family.toLowerCase(), family]),
);
const portableAlternatives = new Map([
  ["thmanyah serif text", "Noto Naskh Arabic"],
  ["thmanyah serif display", "Amiri"],
  ["thmanyah sans", "Noto Sans Arabic"],
]);

export function fontChoices(installed: readonly string[]) {
  const choices = new Map(canonicalFamilies);
  for (const name of installed) {
    const key = name.toLowerCase();
    if (!choices.has(key)) choices.set(key, name);
  }
  return [...choices.values()];
}

export function canonicalFamily(name: string) {
  return canonicalFamilies.get(name.toLowerCase()) ?? name;
}

export function portableFamily(name: string) {
  return portableAlternatives.get(name.toLowerCase()) ?? canonicalFamily(name);
}

export function fontFaceCSS(face: Face, url: string) {
  const range = face.unicodeRange ? `;unicode-range:${face.unicodeRange}` : "";
  return `@font-face{font-family:${JSON.stringify(face.family)};font-style:normal;font-weight:${face.weight};font-display:swap;src:url(${JSON.stringify(url)}) format('truetype')${range}}`;
}
