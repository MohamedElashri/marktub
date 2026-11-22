import { writable } from "svelte/store";

export const arabicCompatibility = writable(false);

// WebKitGTK's horizontal Skia text path can discard the glyphs' individual
// vertical offsets. A fatha tanween must leave the base letter where it was.
// https://bugs.webkit.org/show_bug.cgi?id=255972
export async function applyArabicCompatibilityFont() {
  if (!navigator.platform.startsWith("Linux")) return;
  const font = '64px "Noto Naskh Arabic"';
  const marks = await document.fonts.load(font, "بً");
  if (!marks.length) return;

  const canvas = document.createElement("canvas");
  canvas.width = 96;
  canvas.height = 112;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return;
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
  if (!basePixels || !markedPixels || retainedPixels >= basePixels * 0.8)
    return;
  await Promise.all(
    [400, 700].map((weight) =>
      document.fonts.load(
        `${weight} 18px "Marktub Arabic Compatibility"`,
        "العربية",
      ),
    ),
  );
  arabicCompatibility.set(true);
}
