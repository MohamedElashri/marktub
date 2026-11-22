import { test } from "node:test";
import assert from "node:assert/strict";
import { resolve } from "node:path";
import { readFileSync } from "node:fs";
import { desktopBinary } from "../../scripts/desktop-paths.mjs";

test("native tests cannot attach to the user's running application instance", () => {
  const normal = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const isolated = JSON.parse(readFileSync("src-tauri/e2e.conf.json", "utf8"));
  assert.equal(isolated.identifier, `${normal.identifier}.e2e`);
});

test("macOS app consumers select the universal desktop bundle in either profile", () => {
  for (const profile of ["debug", "release"]) {
    assert.equal(
      desktopBinary(profile, "darwin", "custom-target"),
      resolve(
        `custom-target/universal-apple-darwin/${profile}/bundle/macos/Marktub.app/Contents/MacOS/marktub`,
      ),
    );
    assert.equal(
      desktopBinary(profile, "linux", "custom-target"),
      resolve(`custom-target/${profile}/marktub`),
    );
  }
});
