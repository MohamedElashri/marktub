import { format } from "prettier";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { licenseFiles, renderNotices } from "./license-files.mjs";
const output = "artifacts/licenses";
mkdirSync(output, { recursive: true });
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
const overrides = JSON.parse(
  readFileSync("patches/dependency-overrides.json", "utf8"),
).packages;
const localOverride = (pkg) =>
  overrides.find(
    (entry) => resolve(entry.path, "Cargo.toml") === resolve(pkg.manifest_path),
  );
const rust = JSON.parse(metadata.stdout).packages.filter(
  (pkg) => pkg.source || localOverride(pkg),
);
const rustCoverage = [],
  rustNotices = [];
const supplements = JSON.parse(
  readFileSync("src/rust-notice-supplements.json", "utf8"),
);
for (const pkg of rust) {
  const override = localOverride(pkg);
  const origin = override
    ? `Source: ${override.upstream}; locally modified source in ${override.path}, with original archive/file hashes and changes recorded in patches/dependency-overrides.json and ${override.patch}. ${override.repository}`
    : `Source: https://crates.io/crates/${pkg.name}/${pkg.version} (unmodified registry archive); ${pkg.repository || "origin is recorded in the archive"}`;
  const directory = join(pkg.manifest_path, "..");
  const candidates = licenseFiles(directory, pkg.license_file);
  const retained = [];
  for (const { path, text } of candidates) {
    retained.push(path);
    rustNotices.push({
      component: `${pkg.name} ${pkg.version} — ${pkg.license || "SEE LICENSE"}\n${origin}\n${path}`,
      text,
    });
  }
  const supplement = supplements.packages.find(
    (s) => s.package === `${pkg.name}@${pkg.version}`,
  );
  for (const notice of supplement?.notices ?? []) {
    const text = supplements.texts[notice.sha256];
    if (typeof text !== "string")
      throw Error(`Missing retained notice: ${pkg.name}`);
    if (createHash("sha256").update(text).digest("hex") !== notice.sha256)
      throw Error(
        `Retained notice checksum mismatch: ${pkg.name} ${pkg.version}`,
      );
    retained.push(notice.url);
    rustNotices.push({
      component: `${pkg.name} ${pkg.version} — ${pkg.license || "SEE LICENSE"}\n${origin}\n${notice.url}`,
      text,
    });
  }
  rustCoverage.push({
    name: pkg.name,
    version: pkg.version,
    license: pkg.license,
    source: override ? "reviewed local override" : "registry archive",
    retainedFiles: retained.sort(),
    archiveNotices: candidates.map(({ path, sha256 }) => ({ path, sha256 })),
  });
}
rustCoverage.sort(
  (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
);
writeFileSync(
  `${output}/rust-licenses.json`,
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
  `${output}/rust-license-coverage.json`,
  await format(
    JSON.stringify({
      scope:
        "Cargo resolved graph superset, including reviewed local overrides; retained notice texts with byte hashes; not legal clearance or active-linkage certification",
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
    for (const file of files.sort()) {
      const text = readFileSync(join(directory, file), "utf8");
      notices.push({
        component: `${pkg.name} ${pkg.versions.join(", ")} — ${pkg.license}\nSource: https://www.npmjs.com/package/${pkg.name}\n${file}`,
        text,
      });
    }
    if (!files.length)
      notices.push({
        component: `${pkg.name} ${pkg.versions.join(", ")}`,
        text: `License declaration: ${pkg.license}. Upstream: ${pkg.homepage || pkg.repository || pkg.name}`,
      });
  }
inventory.sort((a, b) => a.name.localeCompare(b.name));
writeFileSync(
  `${output}/javascript-licenses.json`,
  await format(JSON.stringify(inventory), { parser: "json" }),
);
const typographyNotices = readdirSync("src/fonts")
  .filter((name) => name.endsWith(".txt"))
  .sort()
  .map((name) => ({
    component: `Riwaq typography — ${name}`,
    text: readFileSync(join("src/fonts", name), "utf8"),
  }));
const frontendNotices = renderNotices([
  { component: "Marktub", text: readFileSync("LICENSE", "utf8") },
  ...notices,
  ...typographyNotices,
  {
    component: "Bundled math fonts",
    text: readFileSync("src/renderers/font-notices.txt", "utf8"),
  },
]);
writeFileSync(`${output}/frontend.txt`, frontendNotices);
writeFileSync(
  `${output}/licenses.txt`,
  renderNotices([
    { component: "Marktub", text: readFileSync("LICENSE", "utf8") },
    ...notices,
    ...rustNotices,
    ...typographyNotices,
    {
      component: "Bundled math fonts",
      text: readFileSync("src/renderers/font-notices.txt", "utf8"),
    },
  ]),
);
if (rustCoverage.some((pkg) => !pkg.retainedFiles.length))
  throw Error("Dependency notice coverage is incomplete");
console.log(
  `Generated build attribution for ${inventory.length} JavaScript and ${rustCoverage.length} Rust packages.`,
);
