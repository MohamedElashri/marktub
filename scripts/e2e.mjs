import {
  desktopTestTargetArgs,
  desktopBinary,
  macosTestArchitecture,
} from "./desktop-paths.mjs";
import { spawnSync } from "node:child_process";

const build = spawnSync(
  "pnpm",
  [
    "tauri",
    "build",
    "--debug",
    ...desktopTestTargetArgs,
    "--features",
    "e2e",
    "--config",
    "src-tauri/e2e.conf.json",
    "--bundles",
    process.platform === "darwin" ? "app" : "deb",
  ],
  {
    stdio: "inherit",
    env: { ...process.env, VITE_MARKTUB_E2E: "1" },
  },
);
if (build.status !== 0) process.exit(build.status ?? 1);
if (process.platform === "darwin") {
  const verify = spawnSync(
    "python3",
    [
      "-c",
      "import sys; sys.path.insert(0, 'scripts'); from macos_binary import verify; verify(sys.argv[1], sys.argv[2:])",
      desktopBinary("debug"),
      macosTestArchitecture,
    ],
    { stdio: "inherit" },
  );
  if (verify.status !== 0) process.exit(verify.status ?? 1);
}
await import("./e2e-run.mjs");
