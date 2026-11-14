import { spawnSync } from "node:child_process";

const build = spawnSync(
  "pnpm",
  [
    "tauri",
    "build",
    "--debug",
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
const baseline = spawnSync(
  "python3",
  ["benchmarks/source_memory.py", "--baseline"],
  { stdio: "inherit" },
);
if (baseline.status !== 0) process.exit(baseline.status ?? 1);
const run = spawnSync(
  "pnpm",
  [
    "exec",
    "wdio",
    "run",
    "tests/e2e/wdio.conf.mjs",
    ...(process.env.MARKTUB_E2E_GREP
      ? ["--mochaOpts.grep", process.env.MARKTUB_E2E_GREP]
      : []),
  ],
  { stdio: "inherit" },
);
const finalize = spawnSync(
  "python3",
  ["benchmarks/source_memory.py", "--finalize"],
  { stdio: "inherit" },
);
process.exit(run.status || finalize.status || 0);
