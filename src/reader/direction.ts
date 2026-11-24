const autoDirection = new WeakMap<Document, boolean>();

function resolvesDetachedDirection(root: DocumentFragment) {
  const document = root.ownerDocument;
  const known = autoDirection.get(document);
  if (known !== undefined) return known;
  const sample = document.createElement("p");
  sample.dir = "auto";
  sample.textContent = "العربية English";
  document.createDocumentFragment().append(sample);
  let supported = false;
  try {
    // Some implementations parse :dir() but resolve a detached auto block as LTR.
    supported = sample.matches(":dir(rtl)");
  } catch {
    // Retain HTML auto direction when the selector is unavailable.
  }
  autoDirection.set(document, supported);
  return supported;
}

// Resolve text before isolating links: dir=auto ignores children with a dir of
// their own, so annotating a link first loses an entirely linked Arabic title.
export function readingDirections(root: DocumentFragment) {
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

  const resolved = Boolean(
    (blocks.length || links.length) && resolvesDetachedDirection(root),
  );
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
