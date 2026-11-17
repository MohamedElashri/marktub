// Resolve document text independently of the surrounding application's language.
export function readingDirections(root: ParentNode) {
  for (const code of root.querySelectorAll("code, pre:has(code), [data-rich]"))
    code.setAttribute("dir", "ltr");
  for (const link of root.querySelectorAll("a"))
    link.setAttribute("dir", "auto");

  // Resolve groups before their children get independent direction. HTML's
  // dir=auto algorithm otherwise skips descendants which have their own dir.
  const groups = [...root.querySelectorAll("blockquote, ul, ol, table")].map(
    (group) => {
      group.setAttribute("dir", "auto");
      let direction = "auto";
      try {
        direction = group.matches(":dir(rtl)") ? "rtl" : "ltr";
      } catch {
        // Older WebViews still support paragraph dir=auto without :dir().
      }
      group.removeAttribute("dir");
      return { group, direction };
    },
  );
  for (const { group, direction } of groups)
    group.setAttribute("dir", direction);
  for (const block of root.querySelectorAll(
    "p, h1, h2, h3, h4, h5, h6, li, dt, dd, th, td, pre:not(:has(code))",
  ))
    block.setAttribute("dir", "auto");
}
