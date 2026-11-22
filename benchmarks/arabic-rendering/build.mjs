import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { build } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const output = process.argv[2];
if (!output) throw new Error("Pass the prototype build output directory.");

await build({
  root,
  base: "./",
  configFile: false,
  build: { outDir: resolve(output), emptyOutDir: false, target: "safari16" },
});
