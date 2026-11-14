import DOMPurify from "dompurify";
const forbidden = [
  "script",
  "iframe",
  "object",
  "embed",
  "foreignObject",
  "form",
  "input",
  "video",
  "audio",
  "link",
  "style",
  "base",
  "image",
  "animate",
  "animateTransform",
  "animateMotion",
  "set",
  "feImage",
  "meta",
];
export function sanitizeReader(html: string) {
  const clean = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [...forbidden, "img"],
    FORBID_ATTR: ["style", "name", "src", "srcset", "target"],
    ALLOW_DATA_ATTR: true,
    SANITIZE_NAMED_PROPS: false,
  });
  const host = document.createElement("template");
  host.innerHTML = clean;
  for (const el of host.content.querySelectorAll<HTMLElement>("*")) {
    if (el.id && !/^h-[\p{L}\p{N}_-]+$/u.test(el.id)) el.removeAttribute("id");
    const href = el.getAttribute("href");
    if (href && !safeLink(href)) el.removeAttribute("href");
    else if (href?.startsWith("#") && !href.startsWith("#h-"))
      el.setAttribute("href", "#h-" + href.slice(1));
  }
  return host.innerHTML;
}
export function safeLink(href: string) {
  return (
    href.length <= 2048 &&
    !Array.from(href).some(
      (c) => c.charCodeAt(0) <= 32 || c.charCodeAt(0) === 127 || c === "\\",
    ) &&
    !/^(?:[a-z][a-z\d+.-]*:|\/\/|\/)/i.test(href) &&
    !/%(?:00|2f|5c)/i.test(href)
  );
}
export function sanitizeRich(html: string, namespace: string) {
  const clean = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true, svg: true, mathMl: true },
    ADD_TAGS: ["use"],
    ADD_ATTR: ["href", "xlink:href"],
    FORBID_TAGS: forbidden,
    FORBID_ATTR: ["src", "srcset", "name", "target"],
    ALLOW_DATA_ATTR: false,
  });
  const template = document.createElement("template");
  template.innerHTML = clean;
  if (template.content.querySelectorAll("*").length > 512)
    throw Error(
      "Rendered block exceeds the 512-node output budget; source is available.",
    );
  const ids = new Map<string, string>();
  for (const node of template.content.querySelectorAll("[id]")) {
    const old = node.id;
    const id = `r-${namespace}-${ids.size}`;
    ids.set(old, id);
    node.id = id;
  }
  for (const node of template.content.querySelectorAll("*")) {
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name.toLowerCase(),
        value = attr.value;
      if (name.startsWith("on") || name === "href" || name === "xlink:href") {
        if (
          (name === "href" || name === "xlink:href") &&
          value.startsWith("#") &&
          ids.has(value.slice(1))
        )
          node.setAttribute(attr.name, "#" + ids.get(value.slice(1)));
        else node.removeAttribute(attr.name);
      } else if (name === "style") {
        // Generated layout only; reject URL/import/expression/escaped CSS entirely.
        if (
          /url\s*\(|@|expression|\\|behavior|position\s*:\s*fixed/i.test(value)
        )
          node.removeAttribute(attr.name);
      } else if (/url\s*\(/i.test(value)) {
        const m = /^url\(#([\w:-]+)\)$/.exec(value);
        if (m && ids.has(m[1]))
          node.setAttribute(attr.name, `url(#${ids.get(m[1])})`);
        else node.removeAttribute(attr.name);
      }
    }
  }
  return template.innerHTML;
}
