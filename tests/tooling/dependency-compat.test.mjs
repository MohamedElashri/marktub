import assert from "node:assert/strict";
import fs from "node:fs";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

const root = createRequire(import.meta.url);
function dependency(from, name) {
  // Resolve in the actual consumer's installed dependency context. Some
  // ESM-only WDIO packages deliberately do not export their manifest to CJS.
  for (const directory of from.resolve.paths(name) ?? []) {
    const manifest = join(directory, name, "package.json");
    if (fs.existsSync(manifest))
      return createRequire(fs.realpathSync(manifest));
  }
  throw new Error(`Missing installed dependency: ${name}`);
}
const adapter = dependency(root, "@wdio/mocha-framework");
const mocha = dependency(adapter, "mocha");

test("Mocha's patched serializer escapes script endings and runs real parallel workers", async () => {
  assert.equal(mocha("serialize-javascript/package.json").version, "7.1.3");
  const serialize = mocha("serialize-javascript");
  function spoofed() {}
  spoofed.toString = () => "function f(){}//</script";
  assert.doesNotMatch(serialize({ f: spoofed }, { space: 2 }), /<\/script/i);
  const folder = await mkdtemp(join(tmpdir(), "marktub-mocha-compat-"));
  try {
    await writeFile(
      join(folder, "hook.cjs"),
      "exports.mochaHooks = { beforeEach() { global.phase9Hook = true; } };\n",
    );
    await writeFile(
      join(folder, "one.cjs"),
      "const assert = require('node:assert/strict'); describe('one', () => { it('compatible hook', () => assert.equal(global.phase9Hook, true)); it('compatible regex', () => assert.match('文🦀', /文/u)); });\n",
    );
    await writeFile(
      join(folder, "two.cjs"),
      "const assert = require('node:assert/strict'); it('compatible second worker', () => assert.equal(global.phase9Hook, true));\n",
    );
    const result = spawnSync(
      process.execPath,
      [
        mocha.resolve("./bin/mocha.js"),
        "--parallel",
        "--jobs",
        "2",
        "--no-config",
        "--no-package",
        "--require",
        join(folder, "hook.cjs"),
        "--grep",
        "compatible",
        "--reporter",
        "json",
        join(folder, "one.cjs"),
        join(folder, "two.cjs"),
      ],
      {
        cwd: folder,
        timeout: 20000,
        encoding: "utf8",
        env: { ...process.env, NODE_OPTIONS: "", MOCHA_OPTIONS: "" },
      },
    );
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const report = JSON.parse(result.stdout);
    assert.equal(report.stats.passes, 3);
    assert.equal(report.stats.failures, 0);
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test("WDIO's patched glob finds Unicode specs and stops before an aborted filesystem walk", async () => {
  const cli = dependency(root, "@wdio/cli");
  const config = dependency(cli, "@wdio/config");
  assert.equal(config("glob/package.json").version, "14.0.4");
  const { glob } = config("glob");
  const folder = await mkdtemp(join(tmpdir(), "marktub-glob-compat-"));
  try {
    await mkdir(join(folder, "nested"));
    await writeFile(join(folder, "nested", "文.spec.cjs"), "fixture");
    let reads = 0;
    const observedFs = {
      ...fs,
      readdir(...args) {
        reads++;
        return fs.readdir(...args);
      },
      readdirSync(...args) {
        reads++;
        return fs.readdirSync(...args);
      },
    };
    assert.deepEqual(await glob("**/*.cjs", { cwd: folder, fs: observedFs }), [
      "nested/文.spec.cjs",
    ]);
    assert.ok(
      reads > 0,
      "the filesystem observer must be exercised by the control",
    );
    reads = 0;
    const reason = new Error("cancelled fixture walk");
    try {
      await glob("**/*", {
        cwd: folder,
        fs: observedFs,
        signal: AbortSignal.abort(reason),
      });
    } catch (error) {
      assert.ok(error === reason || error.name === "AbortError");
    }
    await new Promise(setImmediate);
    assert.equal(reads, 0);
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test("ESLint's patched matcher preserves dot, Unicode and exclusion semantics", () => {
  const eslint = dependency(root, "eslint");
  assert.equal(eslint("minimatch/package.json").version, "11.0.8");
  const { minimatch, Minimatch } = eslint("minimatch");
  assert.equal(minimatch("src/文.svelte", "src/**/*.{ts,svelte}"), true);
  assert.equal(minimatch("src/.private.ts", "src/**/*.ts"), false);
  assert.equal(
    minimatch("src/.private.ts", "src/**/*.ts", { dot: true }),
    true,
  );
  assert.equal(new Minimatch("**/!(*.test).ts").match("src/editor.ts"), true);
  assert.equal(
    new Minimatch("**/!(*.test).ts").match("src/editor.test.ts"),
    false,
  );
});
