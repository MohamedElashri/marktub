import { defineConfig } from "vitest/config";
import { readFileSync, readdirSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [
    {
      name: "offline-export-fonts",
      resolveId(id) {
        if (id === "virtual:marktub-export-fonts") return "\0" + id;
      },
      load(id) {
        if (id === "\0virtual:marktub-export-fonts") {
          const directory = "node_modules/katex/dist/fonts";
          const fonts = Object.fromEntries(
            readdirSync(directory)
              .filter((f) => f.endsWith(".woff2"))
              .map((f) => [
                f,
                "data:font/woff2;base64," +
                  readFileSync(`${directory}/${f}`).toString("base64"),
              ]),
          );
          return `export default ${JSON.stringify(fonts)}`;
        }
      },
    },
    {
      name: "blank-baseline",
      transformIndexHtml: {
        order: "pre",
        handler: (html) =>
          process.env.MARKTUB_BASELINE === "1"
            ? html.replace(/<script[\s\S]*?<\/script>/g, "")
            : html,
      },
    },
    svelte(),
  ],
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: "es2022" },
  test: { include: ["tests/unit/**/*.test.ts"], environment: "node" },
});
