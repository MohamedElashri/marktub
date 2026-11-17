import type { Heading } from "./model";

// Headings follow section order, including synthetic footnotes at the end.
export function headingBoundary(headings: readonly Heading[], section: number) {
  let low = 0,
    high = headings.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (headings[middle].section < section) low = middle + 1;
    else high = middle;
  }
  return low;
}

export function matchingHeadings(headings: readonly Heading[], query: string) {
  const needle = query.toLocaleLowerCase();
  const result: Heading[] = [];
  for (const heading of headings) {
    if (!needle || heading.title.toLocaleLowerCase().includes(needle)) {
      result.push(heading);
      if (result.length === 200) break;
    }
  }
  return result;
}
