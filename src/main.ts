import { mount } from "svelte";
import App from "./App.svelte";
import { applyArabicCompatibilityFont } from "./fonts/arabic-compatibility";
import "./styles.css";
import "./reader/typography.css";
import "./fonts/fonts.css";

if (import.meta.env.VITE_MARKTUB_E2E === "1") {
  await import("@wdio/tauri-plugin");
  await import("../tests/e2e/reader-cost");
}

mount(App, { target: document.getElementById("app")! });
void applyArabicCompatibilityFont().catch(() => {
  // Keep the requested typography if the renderer cannot sample the font.
});
