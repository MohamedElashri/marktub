import { expect, it } from "vitest";
import { headingBoundary, matchingHeadings } from "../../src/reader/headings";
import type { Heading } from "../../src/reader/model";
const headings: Heading[] = Array.from({ length: 100000 }, (_, i) => ({
  id: `h-${i}`,
  title: `قسم ${i} English`,
  level: 2,
  section: Math.floor(i / 3),
}));

it("selects all headings in a section with the same order as a full scan", () => {
  for (const section of [-1, 0, 1, 100, 33333, 33334, 100000]) {
    const selected = headings.slice(
      headingBoundary(headings, section),
      headingBoundary(headings, section + 1),
    );
    expect(selected).toEqual(headings.filter((h) => h.section === section));
    expect(headings[headingBoundary(headings, section + 1) - 1]).toEqual(
      headings.filter((h) => h.section <= section).at(-1),
    );
  }
  expect(headingBoundary([], 0)).toBe(0);
});

it("keeps section lookup logarithmic at the declared heading capacity", () => {
  let reads = 0;
  const counted = new Proxy(headings, {
    get(target, key, receiver) {
      if (typeof key === "string" && /^\d+$/.test(key)) reads++;
      return Reflect.get(target, key, receiver);
    },
  });
  expect(headingBoundary(counted, 20000)).toBe(60000);
  expect(reads).toBeLessThan(20);
});

it("bounds common outline queries while preserving Unicode matching and result order", () => {
  let reads = 0;
  const counted = new Proxy(headings, {
    get(target, key, receiver) {
      if (typeof key === "string" && /^\d+$/.test(key)) reads++;
      return Reflect.get(target, key, receiver);
    },
  });
  expect(matchingHeadings(counted, "")).toEqual(headings.slice(0, 200));
  expect(reads).toBe(200);
  for (const query of ["قسم", "ENGLISH", "99999", "missing"])
    expect(matchingHeadings(headings, query)).toEqual(
      headings
        .filter((h) =>
          h.title.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
        )
        .slice(0, 200),
    );
});
