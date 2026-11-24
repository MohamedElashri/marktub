import { spawnSync } from "node:child_process";
import { defineConfig } from "vitest/config";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig(({ mode }) => ({
  plugins: [
    {
      name: "generated-attribution",
      resolveId(id) {
        if (id === "virtual:marktub-notices") return "\0" + id;
      },
      load(id) {
        if (id !== "\0virtual:marktub-notices") return;
        if (!existsSync("artifacts/licenses/frontend.txt")) {
          const result = spawnSync(process.execPath, ["scripts/licenses.mjs"], {
            stdio: "inherit",
          });
          if (result.status !== 0)
            throw Error("Dependency attribution generation failed");
        }
        return `export default ${JSON.stringify(readFileSync("artifacts/licenses/frontend.txt", "utf8"))}`;
      },
    },
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
  // Component tests mount into jsdom and need Svelte's client entry point.
  resolve: mode === "test" ? { conditions: ["browser"] } : undefined,
  test: { include: ["tests/unit/**/*.test.ts"], environment: "node" },
}));
