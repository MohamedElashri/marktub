import assert from "node:assert/strict";
import fs from "node:fs";
import {
  mkdtemp,
  mkdir,
  writeFile,
  readFile,
  lstat,
  symlink,
  rm,
} from "node:fs/promises";
import { createRequire } from "node:module";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
const root = createRequire(import.meta.url);
function dependency(from, name) {
  for (const directory of from.resolve.paths(name) ?? []) {
    const path = join(directory, name, "package.json");
    if (fs.existsSync(path)) return createRequire(fs.realpathSync(path));
  }
  throw Error(`Missing ${name}`);
}
const plugin = dependency(root, "@sveltejs/vite-plugin-svelte");
const merge = plugin("deepmerge");
test("Svelte's resolved merge drops poison keys from both operands, preserving normal config and symbols", () => {
  const bad = JSON.parse(
    '{"__proto__":{"poisoned":true},"constructor":{"prototype":{"poisoned":true}},"nested":{"__proto__":{"poisoned":true},"value":1}}',
  );
  for (const [a, b] of [
    [bad, {}],
    [Object.create(null), bad],
    [{ nested: {} }, bad],
  ]) {
    const result = merge(a, b);
    assert.equal(Object.getPrototypeOf(result), Object.prototype);
    assert.equal(result.poisoned, undefined);
    assert.equal(Object.hasOwn(result, "constructor"), false);
    assert.equal(Object.getPrototypeOf(result.nested), Object.prototype);
    assert.equal(result.nested.poisoned, undefined);
  }
  const key = Symbol("config");
  const result = merge(
    { compilerOptions: { dev: true }, items: [1], [key]: "symbol" },
    { compilerOptions: { css: "external" }, items: [2] },
  );
  assert.deepEqual(result.compilerOptions, { dev: true, css: "external" });
  assert.deepEqual(result.items, [1, 2]);
  assert.equal(result[key], "symbol");
  assert.equal({}.poisoned, undefined);
});
const utils = dependency(
  dependency(root, "@wdio/mocha-framework"),
  "@wdio/utils",
);
const browsers = dependency(utils, "@puppeteer/browsers");
const extract = browsers("extract-zip");
function archive(folder, name, entries) {
  const path = join(folder, name);
  const result = spawnSync(
    "python3",
    [
      "-c",
      `import sys,json,zipfile,stat
with zipfile.ZipFile(sys.argv[1], 'w') as z:
 for e in json.loads(sys.argv[2]):
  i=zipfile.ZipInfo(e['name']);i.create_system=3;i.external_attr=(stat.S_IFLNK|0o777 if e.get('link') else stat.S_IFREG|0o644)<<16
  z.writestr(i,e['data'])
`,
      path,
      JSON.stringify(entries),
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  return path;
}
test("actual ZIP consumer refuses both symlink attacks, existing links and duplicate-file writes", async () => {
  const folder = await mkdtemp(join(tmpdir(), "marktub-zip-mitigation-"));
  try {
    const outside = join(folder, "outside");
    await mkdir(outside);
    const victim = join(outside, "keep");
    await writeFile(victim, "exact original");
    for (const [name, entries] of [
      [
        "relative",
        [
          { name: "link", data: "../outside", link: true },
          { name: "link/keep", data: "bad" },
        ],
      ],
      [
        "duplicate-link",
        [
          { name: "keep", data: victim, link: true },
          { name: "keep", data: "bad" },
        ],
      ],
      ["absolute-link", [{ name: "keep", data: victim, link: true }]],
    ]) {
      const dest = join(folder, name);
      await assert.rejects(
        extract(archive(folder, name + ".zip", entries), { dir: dest }),
        /symlink/,
      );
      assert.equal(await readFile(victim, "utf8"), "exact original");
      assert.equal(fs.existsSync(join(dest, "link")), false);
    }
    const dest = join(folder, "existing");
    await mkdir(dest);
    await symlink(victim, join(dest, "keep"));
    await assert.rejects(
      extract(
        archive(folder, "existing.zip", [{ name: "keep", data: "bad" }]),
        { dir: dest },
      ),
      /EEXIST/,
    );
    assert.equal(await readFile(victim, "utf8"), "exact original");
    assert.equal((await lstat(join(dest, "keep"))).isSymbolicLink(), true);
    await symlink(outside, join(dest, "directory"));
    await assert.rejects(
      extract(
        archive(folder, "parent.zip", [{ name: "directory/new", data: "bad" }]),
        { dir: dest },
      ),
      /links/,
    );
    assert.equal(fs.existsSync(join(outside, "new")), false);
    const duplicate = join(folder, "duplicate");
    await assert.rejects(
      extract(
        archive(folder, "duplicate.zip", [
          { name: "same", data: "first" },
          { name: "same", data: "second" },
        ]),
        { dir: duplicate },
      ),
      /EEXIST/,
    );
    assert.equal(await readFile(join(duplicate, "same"), "utf8"), "first");
    const ordinary = join(folder, "ordinary");
    await extract(
      archive(folder, "ordinary.zip", [
        { name: "nested/文.txt", data: "exact ordinary 🦀" },
      ]),
      { dir: ordinary },
    );
    assert.equal(
      await readFile(join(ordinary, "nested/文.txt"), "utf8"),
      "exact ordinary 🦀",
    );
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});
