import { mount } from "svelte";
import App from "./App.svelte";
import { detectShaping } from "./fonts/arabic-compatibility";
import { installBundledFonts } from "./fonts/browser";
import "./styles.css";
import "./reader/typography.css";
import "./editor/typography.css";

installBundledFonts();
void detectShaping();

if (import.meta.env.VITE_MARKTUB_E2E === "1") {
  await import("@wdio/tauri-plugin");
  await import("../tests/e2e/reader-cost");
}

mount(App, { target: document.getElementById("app")! });
