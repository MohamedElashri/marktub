import { format } from "prettier";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
const result = spawnSync("pnpm", ["licenses", "list", "--prod", "--json"], {
  encoding: "utf8",
});
if (result.status !== 0)
  throw Error(result.stderr || "License inventory failed.");
const groups = JSON.parse(result.stdout),
  inventory = [],
  notices = [];
// Collect a conservative Cargo resolved-graph superset, including target-only and
// build dependencies. Do not mistake license declarations for retained notices.
const metadata = spawnSync(
  "cargo",
  [
    "metadata",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--locked",
    "--offline",
    "--format-version",
    "1",
  ],
  { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 },
);
if (metadata.status !== 0)
  throw Error(metadata.stderr || "Cargo license inventory failed.");
const rust = JSON.parse(metadata.stdout).packages.filter((p) => p.source);
const rustCoverage = [],
  rustNotices = [];
const supplements = existsSync("src/rust-notice-supplements.json")
  ? JSON.parse(readFileSync("src/rust-notice-supplements.json", "utf8"))
  : [];
for (const pkg of rust) {
  const origin = `Source: https://crates.io/crates/${pkg.name}/${pkg.version} (unmodified registry archive); ${pkg.repository || "origin is recorded in the archive"}`;
  const directory = join(pkg.manifest_path, "..");
  const candidates = new Set(
    readdirSync(directory).filter((name) =>
      /^(?:licen[sc]e|copying|notice|copyright|authors)(?:[.\-_].*)?$/i.test(
        name,
      ),
    ),
  );
  if (pkg.license_file) candidates.add(pkg.license_file);
  const retained = [];
  for (const file of candidates) {
    try {
      const text = readFileSync(join(directory, file), "utf8");
      if (text.trim()) {
        retained.push(file);
        rustNotices.push(
          `${pkg.name} ${pkg.version} — ${pkg.license || "SEE LICENSE"}\n${origin}\n${file}\n${text}`,
        );
      }
    } catch {
      /* Directory or unavailable notice; reported in coverage below. */
    }
  }
  if (!retained.length) {
    const supplement = supplements.find(
      (s) => s.package === `${pkg.name}@${pkg.version}`,
    );
    for (const notice of supplement?.notices ?? []) {
      if (
        createHash("sha256").update(notice.text).digest("hex") !== notice.sha256
      )
        throw Error(
          `Retained notice checksum mismatch: ${pkg.name} ${pkg.version}`,
        );
      retained.push(notice.url);
      rustNotices.push(
        `${pkg.name} ${pkg.version} — ${pkg.license || "SEE LICENSE"}\n${origin}\n${notice.url}\n${notice.text}`,
      );
    }
  }
  rustCoverage.push({
    name: pkg.name,
    version: pkg.version,
    license: pkg.license,
    retainedFiles: retained.sort(),
  });
}
rustCoverage.sort(
  (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
);
writeFileSync(
  "docs/evidence/rust-licenses.json",
  await format(
    JSON.stringify(
      rustCoverage.map(({ name, version, license }) => ({
        name,
        version,
        license,
      })),
    ),
    { parser: "json" },
  ),
);
writeFileSync(
  "docs/evidence/rust-license-coverage.json",
  await format(
    JSON.stringify({
      scope:
        "Cargo resolved registry graph superset; notices present in installed crate archives, not legal clearance or proof of every nested vendored obligation",
      packages: rustCoverage,
      missing: rustCoverage
        .filter((p) => !p.retainedFiles.length)
        .map((p) => `${p.name}@${p.version}`),
    }),
    { parser: "json" },
  ),
);
for (const packages of Object.values(groups))
  for (const pkg of packages) {
    inventory.push({
      name: pkg.name,
      versions: pkg.versions,
      license: pkg.license,
    });
    const directory = pkg.paths[0];
    const files = readdirSync(directory).filter((name) =>
      /^(?:licen[sc]e|copying|notice)(?:[.\-_].*)?$/i.test(name),
    );
    const texts = files
      .map((file) => {
        try {
          return readFileSync(join(directory, file), "utf8");
        } catch {
          return "";
        }
      })
      .filter(Boolean);
    notices.push(
      `${pkg.name} ${pkg.versions.join(", ")} — ${pkg.license}\nSource: https://www.npmjs.com/package/${pkg.name} (versions listed above; unmodified installed sources)\n${texts.join("\n") || `License declaration: ${pkg.license}. Upstream: ${pkg.homepage || pkg.repository || pkg.name}`}`,
    );
  }
inventory.sort((a, b) => a.name.localeCompare(b.name));
writeFileSync(
  "docs/evidence/javascript-licenses.json",
  await format(JSON.stringify(inventory), { parser: "json" }),
);
const frontendNotices =
  "Installed production JavaScript dependency notices (superset of shipped modules).\n\n" +
  notices.join(
    "\n\n------------------------------------------------------------\n\n",
  ) +
  "\n\n" +
  readFileSync("src/renderers/font-notices.txt", "utf8");
writeFileSync("src/frontend-notices.txt", frontendNotices);
writeFileSync(
  "THIRD_PARTY_LICENSES.txt",
  frontendNotices +
    "\n\n" +
    readFileSync("src/library-notices.txt", "utf8") +
    "\n\n" +
    readFileSync("src/git-notices.txt", "utf8") +
    "\n\n" +
    readFileSync("src/power-notices.txt", "utf8") +
    "\n\nRust dependency notices (resolved graph superset).\n\n" +
    rustNotices.join(
      "\n\n------------------------------------------------------------\n\n",
    ),
);
console.log(
  `Collected ${inventory.length} production dependency license records plus embedded font notices.`,
);
