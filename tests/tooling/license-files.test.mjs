import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  symlinkSync,
  rmSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { licenseFiles, renderNotices } from "../../scripts/license-files.mjs";

test("groups identical notices while preserving all component attributions and exact text", () => {
  const text = "\ufeffCopyright 文\r\nPermission retained.\r\n";
  const rendered = renderNotices([
    { component: "Second component", text },
    { component: "First component", text },
    { component: "Different terms", text: "Other license\n" },
  ]);
  assert.equal(rendered.split(text).length - 1, 1);
  assert(rendered.includes("First component\n\nSecond component"));
  assert(rendered.includes("Different terms\n\nOther license\n"));
});

function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), "marktub-notices-"));
  try {
    run(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
test("retains nested vendored notices and their exact BOM/Unicode/CRLF bytes", () =>
  fixture((root) => {
    mkdirSync(join(root, "vendor"));
    const text = "\ufeffCopyright 文\r\nPermission retained.\r\n";
    writeFileSync(join(root, "LICENSE"), "Main license\n");
    writeFileSync(join(root, "vendor", "COPYING.MIT"), text);
    writeFileSync(join(root, "vendor", "code.c"), "not a notice");
    writeFileSync(join(root, "vendor", "copying.rs"), "ordinary source code");
    const notices = licenseFiles(root);
    assert.deepEqual(
      notices.map((n) => n.path),
      ["LICENSE", "vendor/COPYING.MIT"],
    );
    assert.equal(notices[1].text, text);
    assert.equal(
      notices[1].sha256,
      createHash("sha256").update(text).digest("hex"),
    );
  }));
test("refuses declared traversal and symlink notices without reading outside bytes", () =>
  fixture((root) => {
    mkdirSync(join(root, "pkg"));
    writeFileSync(join(root, "secret"), "outside canary");
    assert.throws(
      () => licenseFiles(join(root, "pkg"), "../secret"),
      /escapes/,
    );
    symlinkSync(join(root, "secret"), join(root, "pkg", "LICENSE"));
    assert.throws(() => licenseFiles(join(root, "pkg")), /symlink/);
  }));
test("does not follow nested directory links and refuses incomplete budgeted scans", () =>
  fixture((root) => {
    mkdirSync(join(root, "pkg"));
    mkdirSync(join(root, "outside"));
    writeFileSync(join(root, "outside", "LICENSE"), "outside canary");
    symlinkSync(join(root, "outside"), join(root, "pkg", "vendor"));
    assert.deepEqual(licenseFiles(join(root, "pkg")), []);
    assert.throws(
      () => licenseFiles(root, undefined, { entries: 1 }),
      /entry budget/,
    );
    assert.throws(
      () => licenseFiles(root, undefined, { depth: 0 }),
      /depth budget/,
    );
  }));
test("refuses binary and oversized notices instead of retaining corrupted text", () =>
  fixture((root) => {
    writeFileSync(join(root, "LICENSE"), Buffer.from([0xc0, 0xaf]));
    assert.throws(() => licenseFiles(root), /UTF-8/);
    writeFileSync(join(root, "LICENSE"), "x".repeat(1024 * 1024 + 1));
    assert.throws(() => licenseFiles(root), /bounded/);
  }));
