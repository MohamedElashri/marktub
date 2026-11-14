import { describe, it, expect } from "vitest";
import corpus from "../fixtures/commonmark-0.31.2.json";
import { compare } from "../support/reader-parity";
// Tests our safe grammar's whole/paged equivalence, not canonical HTML compliance.
describe("CommonMark 0.31.2 whole/paged parity", () => {
  it("retains the complete attributed upstream corpus", () => {
    expect(corpus.examples).toHaveLength(652);
    expect(corpus.license).toBe("CC BY-SA 4.0");
  });
  for (const example of corpus.examples) {
    it(`${example.example}: ${example.section}`, async () => {
      for (const width of [7, 31, 127, 65536])
        await compare(example.markdown, width);
    });
  }
  for (const section of new Set(
    corpus.examples.map((example) => example.section),
  )) {
    it(`cross-section boundaries: ${section}`, async () => {
      const text = corpus.examples
        .filter((example) => example.section === section)
        .map(
          (example) =>
            `# Example ${example.example}\n\n${example.markdown}\n\n`,
        )
        .join("");
      for (const width of [19, 257, 65536]) await compare(text, width);
    });
  }
});
