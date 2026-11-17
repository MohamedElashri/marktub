import { desktopBinary } from "./desktop-paths.mjs";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const binary = readFileSync(desktopBinary("debug"));
if (!binary.includes(Buffer.from("e2e_fixture")))
  throw Error(
    "Build the isolated E2E app with pnpm test:e2e before running it again.",
  );

const baseline = spawnSync(
  "python3",
  ["benchmarks/source_memory.py", "--baseline"],
  { stdio: "inherit" },
);
if (baseline.status !== 0) process.exit(baseline.status ?? 1);
const environment = { ...process.env };
if (process.platform === "linux") {
  // Isolate native Trash and desktop records as well as the existing per-process
  // e2e private store. Keep the directory as failure evidence; never clean users' data.
  environment.XDG_DATA_HOME = mkdtempSync(join(tmpdir(), "marktub-e2e-data-"));
  environment.XDG_CONFIG_HOME = join(environment.XDG_DATA_HOME, "config");
  mkdirSync(environment.XDG_CONFIG_HOME);
  const applications = join(environment.XDG_DATA_HOME, "applications");
  mkdirSync(applications);
  writeFileSync(
    join(applications, "marktub-e2e-pdf.desktop"),
    "[Desktop Entry]\nType=Application\nName=Export test viewer\nExec=/usr/bin/evince %f\nMimeType=application/pdf;\nTerminal=false\n",
  );
  writeFileSync(
    join(environment.XDG_CONFIG_HOME, "mimeapps.list"),
    "[Default Applications]\napplication/pdf=marktub-e2e-pdf.desktop;\n",
  );
  writeFileSync(
    "artifacts/native-e2e-environment.json",
    JSON.stringify({ xdgDataHome: environment.XDG_DATA_HOME }, null, 2),
  );
}
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
  { stdio: "inherit", env: environment },
);
const finalize = spawnSync(
  "python3",
  ["benchmarks/source_memory.py", "--finalize"],
  { stdio: "inherit" },
);
process.exit(run.status || finalize.status || 0);
