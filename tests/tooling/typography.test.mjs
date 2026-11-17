import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";

test("bundled typography retains the recorded upstream bytes and font licences", () => {
  const record = JSON.parse(
    readFileSync("docs/evidence/riwaq-assets.json", "utf8"),
  );
  const fonts = record.files.filter((file) => file.path.endsWith(".ttf"));
  assert.deepEqual(
    fonts.map((file) => file.path.split("/").at(-1)).sort(),
    readdirSync("src/fonts/assets").sort(),
  );
  for (const file of record.files) {
    const bytes = readFileSync(file.path);
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      file.sha256,
      file.path,
    );
    assert.equal(
      createHash("sha1")
        .update(`blob ${bytes.length}\0`)
        .update(bytes)
        .digest("hex"),
      file.gitBlob,
      file.path,
    );
    assert.match(file.source, /^https:\/\/github.com\/google\/fonts\/blob\//);
    assert.doesNotMatch(file.path, /thmanyah/i);
    if (file.path.endsWith("-OFL.txt"))
      assert.match(bytes.toString(), /SIL OPEN FONT LICENSE Version 1.1/);
  }
  assert.equal(
    record.files.filter((file) => file.path.endsWith("-OFL.txt")).length,
    4,
  );
  assert.match(
    readFileSync("src/fonts/riwaq-MIT.txt", "utf8"),
    /Permission is hereby granted/,
  );
});
