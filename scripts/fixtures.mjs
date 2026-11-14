import { mkdir, open } from "node:fs/promises";

await mkdir("tests/generated", { recursive: true });
const paragraph =
  "# A measured section\n\nKeep the source. العربية 日本語 🦀\n\n- First\n- Second\n\n";
const seed = Buffer.from(paragraph.repeat(1024), "utf8");
for (const mib of [1, 10, 100]) {
  const file = await open(`tests/generated/${mib}MiB.md`, "w");
  let remaining = mib * 1024 * 1024;
  while (remaining > 0) {
    const count = Math.min(remaining, seed.length);
    // Trailing partial scalar is avoided by padding the final small segment as ASCII.
    await file.write(remaining < seed.length ? Buffer.alloc(count, 32) : seed);
    remaining -= count;
  }
  await file.close();
  console.log(`Created ${mib} MiB repeat-section fixture`);
}
