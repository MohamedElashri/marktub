import { writable, readonly } from "svelte/store";
import { arabicCompatibilityFamily, arabicProbeFamily } from "./catalogue";
import type { Shaping } from "./typography";

const detectedShaping = writable<Shaping>("standard");
export const shaping = readonly(detectedShaping);

export function isWebKitGTK() {
  return (
    navigator.platform.startsWith("Linux") &&
    navigator.userAgent.includes("AppleWebKit") &&
    !/(Chrome|Chromium)\//.test(navigator.userAgent)
  );
}

// WebKitGTK's horizontal Skia text path can discard the glyphs' individual
// vertical offsets. A fatha tanween must leave the base letter where it was.
// https://bugs.webkit.org/show_bug.cgi?id=255972
async function needsCompatibilityFace() {
  if (!isWebKitGTK() || !document.fonts) return false;
  const font = `64px ${JSON.stringify(arabicProbeFamily)}`;
  const marks = await document.fonts.load(font, "بً");
  if (!marks.length) return false;

  const canvas = document.createElement("canvas");
  canvas.width = 96;
  canvas.height = 112;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return false;
  context.font = font;
  context.direction = "rtl";
  context.textAlign = "left";

  const ink = (text: string) => {
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.fillText(text, 8, 80);
    return context.getImageData(0, 0, canvas.width, canvas.height).data;
  };

  const bare = ink("ب"),
    marked = ink("بً");
  let basePixels = 0,
    retainedPixels = 0,
    markedPixels = 0;
  for (let index = 3; index < bare.length; index += 4) {
    if (marked[index] > 127) markedPixels++;
    if (bare[index] <= 127) continue;
    basePixels++;
    if (marked[index] > 127) retainedPixels++;
  }
  return Boolean(
    basePixels && markedPixels && retainedPixels < basePixels * 0.8,
  );
}

export async function detectShaping() {
  try {
    if (!(await needsCompatibilityFace())) return;
    const loaded = await document.fonts.load(
      `400 18px ${JSON.stringify(arabicCompatibilityFamily)}`,
      "العربية",
    );
    if (loaded.length) detectedShaping.set("webkit-compatibility");
  } catch {
    // Font loading or canvas sampling can fail independently of preferences.
  }
}
