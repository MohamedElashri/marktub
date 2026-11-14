import { mount } from "svelte";
import App from "./App.svelte";
import "./styles.css";

if (import.meta.env.VITE_MARKTUB_E2E === "1") {
  await import("@wdio/tauri-plugin");
}

mount(App, { target: document.getElementById("app")! });
