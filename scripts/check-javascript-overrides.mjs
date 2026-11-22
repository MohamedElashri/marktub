import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  existsSync,
  readFileSync,
  readdirSync,
  realpathSync,
  lstatSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";

const advisory = "https://github.com/advisories/GHSA-fc5q-39xc-vjpf";
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([key, item]) => [key, canonical(item)]),
    );
  return value;
}
const findingHash = (value) =>
  createHash("sha256")
    .update(JSON.stringify(canonical(value)))
    .digest("hex");
const sha = (file) =>
  createHash("sha256").update(readFileSync(file)).digest("hex");
function files(folder) {
  return readdirSync(folder, { withFileTypes: true }).flatMap((entry) => {
    const path = join(folder, entry.name);
    assert(
      !lstatSync(path).isSymbolicLink(),
      "Vendored JavaScript source must not contain symlinks",
    );
    return entry.isDirectory() ? files(path) : [path];
  });
}
export function dependency(from, name) {
  for (const directory of from.resolve.paths(name) ?? []) {
    const path = join(directory, name, "package.json");
    if (existsSync(path))
      return {
        folder: dirname(realpathSync(path)),
        require: createRequire(realpathSync(path)),
        manifest: JSON.parse(readFileSync(path)),
      };
  }
  throw Error(`Missing installed dependency: ${name}`);
}
export function validateJavaScriptOverrides(directory) {
  const root = realpathSync(directory);
  const record = JSON.parse(
    readFileSync(join(root, "patches/dependency-overrides.json")),
  ).javascript;
  assert.equal(record.advisory, advisory);
  assert.equal(record.reviewedAdvisory.url, advisory);
  assert.equal(findingHash(record.reviewedAdvisory), record.advisorySha256);
  assert.deepEqual(record.packages.map((p) => p.version).sort(), [
    "2.3.2",
    "4.0.7",
  ]);
  const workspace = readFileSync(join(root, "pnpm-workspace.yaml"), "utf8");
  const lock = readFileSync(join(root, "pnpm-lock.yaml"), "utf8");
  for (const item of record.packages) {
    assert.equal(item.name, "picomatch");
    assert.equal(item.upstreamName, "picomatch");
    assert.equal(item.path, `vendor/picomatch-${item.version}`);
    assert.equal(item.patch, `patches/picomatch-${item.version}.patch`);
    assert(
      workspace.includes(`file:${item.path}`),
      "Missing closed file override",
    );
    assert(
      !new RegExp(
        `^  picomatch@${item.version.replaceAll(".", "\\.")}:`,
        "m",
      ).test(lock),
      "Unpatched registry matcher returned to the lockfile",
    );
    const folder = join(root, item.path);
    assert.equal(realpathSync(folder), folder);
    assert.deepEqual(
      files(folder)
        .map((file) => relative(folder, file).replaceAll("\\", "/"))
        .sort(),
      item.files.map((f) => f.path).sort(),
      "Unreviewed JavaScript source inventory",
    );
    for (const file of item.files) {
      assert(
        !file.path.split("/").some((part) => part === ".." || part === "."),
      );
      assert.equal(
        sha(join(folder, file.path)),
        file.sha256,
        `Changed backport source: ${item.version}/${file.path}`,
      );
      assert.match(file.upstreamSha256, /^[a-f0-9]{64}$/);
    }
    assert.equal(
      sha(join(root, item.patch)),
      item.patchSha256,
      "Changed backport patch",
    );
    const manifest = JSON.parse(readFileSync(join(folder, "package.json")));
    assert.equal(manifest.name, item.name);
    assert.equal(manifest.version, item.version);
    assert.equal(manifest.license, "MIT");
  }
  const require = createRequire(join(root, "package.json"));
  const vite = dependency(require, "vite");
  const mocha = dependency(
    dependency(require, "@wdio/mocha-framework").require,
    "mocha",
  );
  const chokidar = dependency(mocha.require, "chokidar");
  const matchers = [
    dependency(vite.require, "picomatch"),
    dependency(dependency(chokidar.require, "anymatch").require, "picomatch"),
    dependency(dependency(chokidar.require, "readdirp").require, "picomatch"),
    dependency(dependency(require, "vitest").require, "picomatch"),
    dependency(
      dependency(dependency(require, "svelte-check").require, "fdir").require,
      "picomatch",
    ),
  ];
  assert.deepEqual(
    matchers.map((matcher) => matcher.manifest.version),
    ["4.0.7", "2.3.2", "2.3.2", "4.0.7", "4.0.7"],
  );
  for (const matcher of matchers) {
    const item = record.packages.find(
      (p) => p.version === matcher.manifest.version,
    );
    assert(
      item && matcher.manifest.name === item.name,
      "Consumer resolved an unreviewed matcher",
    );
    for (const file of item.files)
      assert.equal(
        sha(join(matcher.folder, file.path)),
        file.sha256,
        "Installed matcher differs from reviewed source",
      );
  }
  return { root, record, matchers };
}
export async function auditUpstream(directory, fetcher = fetch) {
  const { record } = validateJavaScriptOverrides(directory);
  const versions = record.packages.map((p) => p.version);
  const response = await fetcher(
    "https://registry.npmjs.org/-/npm/v1/security/advisories/bulk",
    {
      method: "POST",
      redirect: "error",
      signal: AbortSignal.timeout(20000),
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ picomatch: versions }),
    },
  );
  assert(
    response.ok,
    `Upstream identity audit failed: HTTP ${response.status}`,
  );
  const text = await response.text();
  assert(
    text.length <= 1024 * 1024,
    "Upstream identity audit response exceeds its bound",
  );
  const raw = JSON.parse(text);
  const reports = Object.entries(raw).flatMap(([name, findings]) =>
    findings.map((finding) => ({
      name,
      finding,
      status:
        name === "picomatch" &&
        finding.url === advisory &&
        findingHash(finding) === record.advisorySha256
          ? "mitigated by reviewed development-only backport"
          : "unreviewed",
    })),
  );
  return {
    scope:
      "Raw registry findings for exact upstream identities; local backport mitigation requires verified source plus mandatory adversarial and consumer regressions. No registry finding is discarded.",
    upstream: { picomatch: versions },
    findings: reports,
    unreviewed: reports.filter((r) => r.status === "unreviewed").length,
  };
}
if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  if (process.argv.includes("--audit")) {
    const report = await auditUpstream(root);
    mkdirSync(join(root, "artifacts"), { recursive: true });
    writeFileSync(
      join(root, "artifacts/javascript-backports-advisories.json"),
      JSON.stringify(report, null, 2) + "\n",
    );
    console.log(
      `Upstream identity audit: ${report.findings.length} visible finding(s), ${report.unreviewed} unreviewed. Mitigation is source- and regression-bound.`,
    );
    assert.equal(
      report.unreviewed,
      0,
      "Unreviewed advisory affects the vendored matcher; inspect the raw audit report",
    );
  } else {
    validateJavaScriptOverrides(root);
    console.log(
      "PASS: local JavaScript backport provenance and actual Vite/Vitest/Chokidar resolutions match reviewed source.",
    );
  }
}
