// Resolve text before isolating links: dir=auto ignores children with a dir of
// their own, so annotating a link first loses an entirely linked Arabic title.
export function readingDirections(root: ParentNode) {
  const blocks = [
    ...root.querySelectorAll(
      "blockquote, ul, ol, table, p, h1, h2, h3, h4, h5, h6, li, dt, dd, th, td, pre:not(:has(code))",
    ),
  ];
  const links = [...root.querySelectorAll("a")];
  for (const node of [...blocks, ...links]) node.removeAttribute("dir");
  for (const code of root.querySelectorAll(
    "code, pre:has(code), [data-rich], button[data-image]",
  ))
    code.setAttribute("dir", "ltr");

  let resolved = true;
  try {
    (blocks[0] ?? links[0])?.matches(":dir(ltr)");
  } catch {
    resolved = false;
  }
  // Preorder lets neutral children inherit their resolved parent while future
  // child annotations cannot hide text from the parent's direction calculation.
  const directions = new Map<Element, string>();
  for (const block of blocks) {
    block.setAttribute("dir", "auto");
    if (resolved && block.tagName !== "PRE") {
      const parent = block.parentElement;
      const inherited = parent && directions.get(parent);
      // Loose list items commonly wrap one paragraph. Its text is identical
      // to its parent's, so resolve it once rather than scan it again.
      const sameText =
        inherited &&
        [...parent!.childNodes].every(
          (node) =>
            node === block ||
            node.nodeType === Node.COMMENT_NODE ||
            (node.nodeType === Node.TEXT_NODE && !node.textContent?.trim()),
        );
      const direction = sameText
        ? inherited
        : block.matches(":dir(rtl)")
          ? "rtl"
          : "ltr";
      block.setAttribute("dir", direction);
      directions.set(block, direction);
    }
  }
  // Older WebViews retain auto blocks and unannotated link labels so their
  // native first-strong detection can still see those labels.
  if (resolved) for (const link of links) link.setAttribute("dir", "auto");
}
