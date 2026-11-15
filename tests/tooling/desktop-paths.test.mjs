import { test } from "node:test";
import assert from "node:assert/strict";
import { resolve } from "node:path";
import { desktopBinary } from "../../scripts/desktop-paths.mjs";

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
