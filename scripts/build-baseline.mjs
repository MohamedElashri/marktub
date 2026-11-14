import { cpSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";

const build = spawnSync(
  "pnpm",
  [
    "tauri",
    "build",
    "--features",
    "blank-baseline",
    "--bundles",
    process.platform === "darwin" ? "app" : "deb",
  ],
  { stdio: "inherit", env: { ...process.env, MARKTUB_BASELINE: "1" } },
);
if (build.status !== 0) process.exit(build.status ?? 1);
mkdirSync("artifacts/baseline", { recursive: true });
if (process.platform === "darwin")
  cpSync(
    "src-tauri/target/release/bundle/macos/Marktub.app",
    "artifacts/baseline/Marktub.app",
    { recursive: true },
  );
else cpSync("src-tauri/target/release/marktub", "artifacts/baseline/marktub");
console.log(
  "Blank baseline copied to artifacts/baseline. Rebuild normally before using the app or checking release assets.",
);
