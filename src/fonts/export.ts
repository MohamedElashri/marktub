import { fontFaceCSS } from "./catalogue";
import type { resolveTypography } from "./typography";

const assets = import.meta.glob<string>("./assets/*.ttf", {
  query: "?inline",
  import: "default",
});

export async function exportFontCSS({
  faces,
}: ReturnType<typeof resolveTypography>) {
  return (
    await Promise.all(
      faces.map(async (face) => {
        const load = assets[`./assets/${face.file}`];
        if (!load) throw Error("Bundled export font is missing.");
        const source = await load();
        if (!/^data:font\/ttf;base64,[A-Za-z0-9+/=]+$/.test(source))
          throw Error("Bundled export font is not self-contained.");
        return fontFaceCSS(face, source);
      }),
    )
  ).join("\n");
}
