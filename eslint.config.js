import js from "@eslint/js";
import ts from "typescript-eslint";
import svelte from "eslint-plugin-svelte";
import globals from "globals";

export default ts.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/**",
      "artifacts/**",
      ".pnpm-store/**",
      "tests/generated/**",
    ],
  },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs["flat/recommended"],
  { languageOptions: { globals: { ...globals.browser, ...globals.node } } },
  {
    files: ["tests/e2e/**/*.mjs"],
    languageOptions: { globals: globals.mocha },
  },
  {
    files: ["**/*.svelte"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
);
