import assert from "node:assert/strict";
import fs from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { pathToFileURL } from "node:url";
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
test("WDIO's resolved browser provider retains its imported APIs and local cache contract", async () => {
  const manifest = browsers("./package.json");
  assert.equal(manifest.version, "3.2.7");
  assert.equal(manifest.type, "module");
  assert.equal(manifest.dependencies["extract-zip"], undefined);
  assert.throws(() => browsers.resolve("extract-zip"), /Cannot find module/);
  const api = await import(pathToFileURL(browsers.resolve("./lib/main.js")));
  for (const name of [
    "install",
    "canDownload",
    "resolveBuildId",
    "detectBrowserPlatform",
    "computeExecutablePath",
    "Cache",
    "getDownloadUrl",
  ])
    assert.equal(typeof api[name], "function", name);
  assert.equal(api.Browser.CHROME, "chrome");
  assert.equal(api.ChromeReleaseChannel.STABLE, "stable");
  const folder = await mkdtemp(join(tmpdir(), "marktub-browser-compat-"));
  try {
    const options = {
      cacheDir: folder,
      browser: api.Browser.CHROME,
      platform: api.BrowserPlatform.MAC_ARM,
      buildId: "123.0.0.0",
    };
    assert.deepEqual(new api.Cache(folder).getInstalledBrowsers(), []);
    assert.ok(api.computeExecutablePath(options).startsWith(folder + "/"));
    assert.equal(
      api.getDownloadUrl(options.browser, options.platform, options.buildId)
        .protocol,
      "https:",
    );
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});
test("actual WDIO setup accepts Tauri capabilities with automatic acquisition disabled", async () => {
  const { setupBrowser, setupDriver } = await import(
    pathToFileURL(utils.resolve("./build/node.js"))
  );
  const { config } = await import("../e2e/wdio.conf.mjs");
  const options = config.services[0][1];
  assert.equal(options.autoInstallTauriDriver, false);
  assert.equal(options.autoDownloadEdgeDriver, false);
  assert.equal(options.driverProvider, "embedded");
  assert.deepEqual(await setupBrowser({}, config.capabilities), [undefined]);
  assert.deepEqual(await setupDriver({}, config.capabilities), [undefined]);
});
