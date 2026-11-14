import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";

const build = spawnSync(
  "cargo",
  [
    "build",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--release",
    "--locked",
    "--example",
    "range_benchmark",
  ],
  { stdio: "inherit" },
);
if (build.status !== 0) process.exit(build.status ?? 1);
const samples = [];
for (const mib of [1, 10, 100]) {
  for (let repetition = 0; repetition < 5; repetition++) {
    const result = spawnSync(
      "src-tauri/target/release/examples/range_benchmark",
      [`tests/generated/${mib}MiB.md`],
      { encoding: "utf8" },
    );
    if (result.status !== 0) throw new Error(result.stderr);
    samples.push({ mib, repetition, ...JSON.parse(result.stdout) });
  }
}
mkdirSync("artifacts", { recursive: true });
writeFileSync(
  "artifacts/ranges.json",
  JSON.stringify(
    { cacheState: "uncontrolled/warm after fixture generation", samples },
    null,
    2,
  ),
);
console.log(JSON.stringify(samples, null, 2));
