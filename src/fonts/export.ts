import { value, type Values } from "../preferences";
import { fontFaces, portableFamily } from "./catalogue";

const assets = import.meta.glob<string>("./assets/*.ttf", {
  query: "?inline",
  import: "default",
});

export async function exportFontCSS(values: Values) {
  const families = new Set(
    ["reader.font", "reader.heading_font", "editor.font"].map((key) =>
      portableFamily(String(value(values, key, ""))),
    ),
  );
  const faces = [
    ...fontFaces.filter((face) => families.has(face.family)),
    {
      family: "Marktub Arabic",
      file: "NotoSansArabic[wdth,wght].ttf",
      weight: "400 700",
    },
  ];
  return (
    await Promise.all(
      faces.map(async (face) => {
        const load = assets[`./assets/${face.file}`];
        if (!load) throw Error("Bundled export font is missing.");
        const source = await load();
        if (!/^data:font\/ttf;base64,[A-Za-z0-9+/=]+$/.test(source))
          throw Error("Bundled export font is not self-contained.");
        const range =
          face.family === "Marktub Arabic"
            ? ";unicode-range:U+0600-06FF,U+0750-077F,U+0870-08FF,U+FB50-FDFF,U+FE70-FEFF,U+10EC0-10EFF,U+1EE00-1EEFF"
            : "";
        return `@font-face{font-family:${JSON.stringify(face.family)};font-style:normal;font-weight:${face.weight};src:url(${source}) format('truetype')${range}}`;
      }),
    )
  ).join("\n");
}
