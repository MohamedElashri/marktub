import type { Section } from "./model";

// Only geometry is retained for offscreen text. Source chunks remain parser
// boundaries, never user-visible pages. A Fenwick tree keeps long jumps cheap.
export class FlowLayout {
  private tree = new Float64Array(131073);
  private heights: number[] = [];
  get length() {
    return this.heights.length;
  }
  resize(sections: Section[]) {
    while (this.heights.length > sections.length) {
      const i = this.heights.length - 1;
      this.set(i, 0);
      this.heights.pop();
    }
    for (let i = this.heights.length; i < sections.length; i++) {
      // Deliberately approximate until the native WebView measures the block.
      this.heights.push(0);
      this.set(i, Math.max(48, (sections[i].end - sections[i].start) * 0.55));
    }
  }
  set(index: number, height: number) {
    if (index < 0 || index >= this.length || !Number.isFinite(height)) return;
    const next = Math.max(0, height);
    const delta = next - this.heights[index];
    this.heights[index] = next;
    for (let i = index + 1; i < this.tree.length; i += i & -i)
      this.tree[i] += delta;
  }
  offset(index: number) {
    let value = 0;
    for (let i = Math.min(this.length, Math.max(0, index)); i > 0; i -= i & -i)
      value += this.tree[i];
    return value;
  }
  indexAt(offset: number) {
    let index = 0;
    let value = 0;
    for (let bit = 131072; bit; bit >>= 1) {
      const next = index + bit;
      if (
        next <= this.length &&
        value + this.tree[next] <=
          offset + Math.max(0.000001, Math.abs(offset) * 1e-12)
      ) {
        value += this.tree[next];
        index = next;
      }
    }
    return Math.min(Math.max(0, this.length - 1), index);
  }
  spacers(from: number, to: number) {
    const before = this.offset(from);
    const inside = this.offset(to) - before;
    const after = this.offset(this.length) - this.offset(to);
    // WebViews have finite scroll extents. Compress distant estimates only;
    // mounted paragraphs always retain their real size and normal typography.
    const scale = Math.min(
      1,
      Math.max(0, 8_000_000 - inside) / Math.max(1, before + after),
    );
    return { before: before * scale, after: after * scale, scale };
  }
  atScroll(top: number, from: number, to: number) {
    const { before, scale } = this.spacers(from, to);
    const inside = this.offset(to) - this.offset(from);
    if (top < before) return this.indexAt(top / Math.max(scale, 0.000001));
    if (top <= before + inside)
      return this.indexAt(this.offset(from) + top - before);
    return this.indexAt(
      this.offset(to) + (top - before - inside) / Math.max(scale, 0.000001),
    );
  }
}

export function readerWindow(
  layout: FlowLayout,
  index: number,
  viewport: number,
) {
  const point = layout.offset(index);
  const from = Math.min(
    index,
    Math.max(index - 12, layout.indexAt(Math.max(0, point - viewport * 3))),
  );
  const to = Math.max(
    index + 1,
    Math.min(
      layout.length,
      index + 13,
      layout.indexAt(point + viewport * 4) + 2,
    ),
  );
  return { from, to };
}

// The ordinary-note path keeps all semantic text in the DOM for selection and
// assistive technology. The large-document path also bounds generated HTML.
export const WHOLE_SOURCE_LIMIT = 256 * 1024;
export const WHOLE_HTML_LIMIT = 2 * 1024 * 1024;
export const WINDOW_HTML_LIMIT = 4 * 1024 * 1024;
