import { desktopTargetArgs, desktopBinary } from "./desktop-paths.mjs";
import { resolve } from "node:path";
import { cpSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";

const build = spawnSync(
  "pnpm",
  [
    "tauri",
    "build",
    ...desktopTargetArgs,
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
    resolve(desktopBinary("release"), "../../.."),
    "artifacts/baseline/Marktub.app",
    { recursive: true },
  );
else cpSync("src-tauri/target/release/marktub", "artifacts/baseline/marktub");
console.log(
  "Blank baseline copied to artifacts/baseline. Rebuild normally before using the app or checking release assets.",
);
