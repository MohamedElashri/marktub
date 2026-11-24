import { fontFaceCSS, fontFaces } from "./catalogue";

const assets = import.meta.glob<string>("./assets/*.ttf", {
  query: "?url&no-inline",
  import: "default",
  eager: true,
});

export function installBundledFonts() {
  const style = document.createElement("style");
  style.textContent = fontFaces
    .map((face) => fontFaceCSS(face, assets[`./assets/${face.file}`]))
    .join("\n");
  document.head.append(style);
}
