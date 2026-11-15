import { resolve } from "node:path";

const binary = resolve(
  process.platform === "darwin"
    ? "src-tauri/target/debug/bundle/macos/Marktub.app/Contents/MacOS/marktub"
    : "src-tauri/target/debug/marktub",
);
export const config = {
  runner: "local",
  outputDir: resolve("artifacts/wdio"),
  specs: [resolve("tests/e2e/smoke.e2e.mjs")],
  maxInstances: 1,
  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath: binary,
        driverProvider: "embedded",
        autoInstallTauriDriver: false,
        autoDownloadEdgeDriver: false,
        captureBackendLogs: true,
        captureFrontendLogs: true,
      },
    ],
  ],
  capabilities: [
    { browserName: "tauri", "tauri:options": { application: binary } },
  ],
  framework: "mocha",
  reporters: ["spec"],
  logLevel: "warn",
  waitforTimeout: 10000,
  connectionRetryTimeout: 20000,
  connectionRetryCount: 0,
  mochaOpts: { timeout: 180000 },
};
