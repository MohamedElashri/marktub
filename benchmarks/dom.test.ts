import { it, expect } from "vitest";
import {
  openSync,
  readSync,
  closeSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import { renderSection } from "../src/reader/render";
import { sanitizePreview } from "../src/reader/sanitize";

it("records bounded DOM scaffolding for windows from real 1/10/100 MiB fixtures", () => {
  const results = [];
  for (const mib of [1, 10, 100]) {
    const file = openSync(`tests/generated/${mib}MiB.md`, "r");
    const bytes = Buffer.alloc(64 * 1024);
    const count = readSync(file, bytes, 0, bytes.length, 0);
    closeSync(file);
    let text = "";
    for (let trim = 0; trim <= 3; trim++) {
      try {
        text = new TextDecoder("utf-8", { fatal: true }).decode(
          bytes.subarray(0, count - trim),
        );
        break;
      } catch {
        /* A UTF-8 scalar may straddle the source window. */
      }
    }
    expect(text.length).toBeGreaterThan(0);
    const result = renderSection(text);
    const host = document.createElement("article");
    host.innerHTML = sanitizePreview(result.html);
    const nodes = host.querySelectorAll("*").length;
    expect(host.querySelector("h1")?.textContent).toBe("A measured section");
    expect(nodes).toBeLessThan(500);
    results.push({
      mib,
      sourceBytes: new TextEncoder().encode(text).byteLength,
      nodes,
      blocks: result.blocks,
      parseMs: result.parseMs,
      clipped: result.clipped,
    });
  }
  mkdirSync("artifacts", { recursive: true });
  writeFileSync(
    "artifacts/dom-scaffold.json",
    JSON.stringify(
      {
        runtime:
          "JSDOM scaffolding; not actual WebView layout/scroll performance",
        results,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify(results));
});
