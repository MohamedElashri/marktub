import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { include: ["benchmarks/dom.test.ts"], environment: "jsdom" },
});
