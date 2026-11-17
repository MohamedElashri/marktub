import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import {
  mkdtempSync,
  mkdirSync,
  copyFileSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import {
  validateJavaScriptOverrides,
  auditUpstream,
  dependency,
} from "../../scripts/check-javascript-overrides.mjs";
const root = fileURLToPath(new URL("../../", import.meta.url));
const state = validateJavaScriptOverrides(root);
const attacks = [
  "+(*)",
  "*(*)",
  "+(*a)",
  "*(*a)",
  "+(*?a)",
  "*(*?)",
  "+(a*)",
  "+(a|aa)",
  "+(@(a|aa))",
  "+(*(a)|*(b))",
  "+(a)+(a)",
];
function probe(folder, patterns, timeout, size = 4096) {
  return spawnSync(
    process.execPath,
    [
      "-e",
      `const pm=require(process.argv[1]); require('node:fs').writeSync(1,'READY'); for(const pattern of JSON.parse(process.argv[2])) for(const options of [{},{maxExtglobRecursion:false},{nocase:true},{capture:true}]) { const result=pm.isMatch('a'.repeat(${size})+'!',pattern,options); if(result) throw Error('Attack string matched'); }`,
      folder,
      JSON.stringify(patterns),
    ],
    { timeout, encoding: "utf8" },
  );
}
test("both reviewed backports literalize repeated extglobs and retain ordinary matching APIs", () => {
  for (const item of state.record.packages) {
    const folder = join(root, item.path);
    const require = createRequire(join(folder, "package.json"));
    const pm = require(folder);
    const result = probe(folder, attacks, 3000);
    assert.equal(result.status, 0, result.stderr || result.error?.message);
    for (const pattern of attacks)
      assert.equal(pm.makeRe(pattern).test(pattern), true, pattern);
    const cases = [
      ["src/文.svelte", "src/**/*.{ts,svelte}", {}, true],
      ["src/.private.ts", "src/**/*.ts", {}, false],
      ["src/.private.ts", "src/**/*.ts", { dot: true }, true],
      ["src/editor.ts", "src/**/!(*.test).ts", {}, true],
      ["src/editor.test.ts", "src/**/!(*.test).ts", {}, false],
      ["note.mjs", "note.?(m)js", {}, true],
      ["note.ts", "note.@(js|ts)", {}, true],
      ["SRC/NOTE.TS", "src/**/*.ts", { nocase: true }, true],
      ["src\\文.ts", "src/**/*.ts", { windows: true }, true],
    ];
    for (const [value, pattern, options, expected] of cases)
      assert.equal(
        pm.isMatch(value, pattern, options),
        expected,
        `${item.version}: ${pattern}`,
      );
    assert.equal(typeof pm.makeRe("**/*.ts").test, "function");
    assert.equal(pm.scan("src/**/*.ts").base, "src");
    assert.equal(typeof pm.parse("**/*.ts").output, "string");
  }
});
test("reversing each recorded patch reproduces the exact upstream files and a killed ReDoS control", () => {
  for (const item of state.record.packages) {
    const folder = mkdtempSync(join(tmpdir(), "marktub-picomatch-control-"));
    try {
      for (const file of item.files) {
        mkdirSync(dirname(join(folder, file.path)), { recursive: true });
        copyFileSync(join(root, item.path, file.path), join(folder, file.path));
      }
      const reversed = spawnSync(
        "git",
        ["apply", "--reverse", "--unidiff-zero", join(root, item.patch)],
        { cwd: folder, encoding: "utf8" },
      );
      assert.equal(
        reversed.status,
        0,
        reversed.stderr || reversed.error?.message,
      );
      for (const file of item.files)
        assert.equal(
          createHash("sha256")
            .update(readFileSync(join(folder, file.path)))
            .digest("hex"),
          file.upstreamSha256,
        );
      const control = probe(folder, ["+(*a)"], 700, 512);
      assert.equal(
        control.stdout,
        "READY",
        "The vulnerable control must enter the matcher before timing out",
      );
      assert.equal(
        control.error?.code,
        "ETIMEDOUT",
        "Upstream vulnerable control must reach the child-process watchdog",
      );
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  }
});
test("the actual Chokidar/anymatch and Vite/Vitest consumers use the restricted source", async () => {
  const require = createRequire(import.meta.url);
  const mocha = dependency(
    dependency(require, "@wdio/mocha-framework").require,
    "mocha",
  );
  const chokidar = dependency(mocha.require, "chokidar");
  const anymatch = dependency(chokidar.require, "anymatch").require("anymatch");
  assert.equal(
    anymatch(["**/*.md", "!**/private/**"], "notes/العربية.md"),
    true,
  );
  assert.equal(
    anymatch(["**/*.md", "!**/private/**"], "notes/private/one.md"),
    false,
  );
  const vite = dependency(require, "vite");
  const glob = (await import(vite.require.resolve("tinyglobby"))).glob;
  const folder = mkdtempSync(join(tmpdir(), "marktub-picomatch-glob-"));
  try {
    mkdirSync(join(folder, "notes"));
    writeFileSync(join(folder, "notes/العربية.md"), "fixture");
    writeFileSync(join(folder, "notes/.hidden.md"), "fixture");
    assert.deepEqual(await glob("**/*.md", { cwd: folder }), [
      "notes/العربية.md",
    ]);
    assert.equal((await glob("**/*.md", { cwd: folder, dot: true })).length, 2);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
test("upstream findings stay visible and new or changed advisories fail qualification", async () => {
  const report = await auditUpstream(root, async () => ({
    ok: true,
    text: async () =>
      JSON.stringify({
        picomatch: [
          state.record.reviewedAdvisory,
          {
            url: "https://github.com/advisories/GHSA-unreviewed",
            title: "Another finding",
          },
          {
            ...state.record.reviewedAdvisory,
            title: "Changed finding with the same identifier",
          },
        ],
      }),
  }));
  assert.equal(report.findings.length, 3);
  assert.deepEqual(report.findings[0].finding, state.record.reviewedAdvisory);
  assert.equal(
    report.findings[0].status,
    "mitigated by reviewed development-only backport",
  );
  assert.equal(report.unreviewed, 2);
});
