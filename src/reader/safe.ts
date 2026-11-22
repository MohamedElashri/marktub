import DOMPurify from "dompurify";
import { readingDirections } from "./direction";
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
const readerAttributes = [
  "class",
  "data-image",
  "data-width",
  "data-height",
  "data-note-link",
  "data-rich",
  "data-run-kind",
  "data-run-source",
  "data-source-line",
  "data-source-end-line",
];
const markdownTags = [
  "a",
  "abbr",
  "b",
  "blockquote",
  "br",
  "button",
  "caption",
  "cite",
  "code",
  "dd",
  "del",
  "details",
  "div",
  "dl",
  "dt",
  "em",
  "figcaption",
  "figure",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "i",
  "img",
  "kbd",
  "li",
  "mark",
  "ol",
  "p",
  "picture",
  "pre",
  "q",
  "s",
  "samp",
  "small",
  "span",
  "strong",
  "sub",
  "summary",
  "sup",
  "table",
  "tbody",
  "td",
  "tfoot",
  "th",
  "thead",
  "tr",
  "u",
  "ul",
  "var",
];

export function sanitizeReader(html: string, metadataKey?: string) {
  const marked = metadataKey
    ? readerAttributes.map((name) => `data-${metadataKey}-${name}`)
    : [];
  const clean = DOMPurify.sanitize(html, {
    ...(metadataKey
      ? { ALLOWED_TAGS: markdownTags }
      : { USE_PROFILES: { html: true } }),
    RETURN_DOM_FRAGMENT: true,
    FORBID_TAGS: metadataKey ? forbidden : [...forbidden, "img"],
    FORBID_ATTR: [
      "style",
      "name",
      "srcset",
      "target",
      "autofocus",
      ...(metadataKey ? [] : ["src"]),
    ],
    ADD_ATTR: marked,
    ALLOW_DATA_ATTR: !metadataKey,
    SANITIZE_NAMED_PROPS: false,
  });
  const host = document.createElement("template");
  host.content.append(clean);
  for (const el of host.content.querySelectorAll<HTMLElement>("*")) {
    if (metadataKey) {
      const generated = el.hasAttribute(marked[0]);
      el.removeAttribute("class");
      for (const [i, attribute] of marked.entries()) {
        const value = el.getAttribute(attribute);
        if (value === null) continue;
        el.setAttribute(readerAttributes[i], value);
        el.removeAttribute(attribute);
      }
      const src = el.getAttribute("src");
      el.removeAttribute("src");
      if (el.localName === "button" && !generated) {
        el.replaceWith(...el.childNodes);
        continue;
      }
      if (el.localName === "img") {
        const link = src?.trim().replaceAll(" ", "%20") ?? "";
        const alt = el.getAttribute("alt") || "image";
        if (!link || (!safeLink(link) && !remoteImageURL(link))) {
          el.replaceWith(alt);
          continue;
        }
        const placeholder = document.createElement("button");
        placeholder.className = "image-request";
        placeholder.dataset.image = link;
        placeholder.textContent = `Load image: ${alt}`;
        for (const name of ["width", "height"] as const) {
          const value = el.getAttribute(name) ?? "";
          if (
            /^\d{1,4}$/.test(value) &&
            Number(value) > 0 &&
            Number(value) <= 4096
          )
            placeholder.dataset[name] = value;
        }
        el.replaceWith(placeholder);
        continue;
      }
    }
    if (el.id && !/^h-[\p{L}\p{N}_-]+$/u.test(el.id)) el.removeAttribute("id");
    const align = el.getAttribute("align");
    if (align && !["left", "center", "right"].includes(align))
      el.removeAttribute("align");
    const href = el.getAttribute("href");
    if (href && !safeLink(href)) el.removeAttribute("href");
    else if (href?.startsWith("#") && !href.startsWith("#h-"))
      el.setAttribute("href", "#h-" + href.slice(1));
  }
  readingDirections(host.content);
  return host.innerHTML;
}
export function remoteImageURL(link: string) {
  if (link.length > 2048 || !/^https:\/\//i.test(link)) return null;
  try {
    const url = new URL(link);
    return url.username || url.password ? null : url.href;
  } catch {
    return null;
  }
}

export function readerImage(target: HTMLElement, src: string) {
  const img = document.createElement("img");
  img.alt = (target.textContent ?? "Image").replace(/^Load image: /, "");
  img.decoding = "async";
  img.style.maxWidth = "100%";
  img.referrerPolicy = "no-referrer";
  for (const name of ["width", "height"] as const) {
    const value = Number(target.dataset[name]);
    if (Number.isInteger(value) && value > 0 && value <= 4096)
      img[name] = value;
  }
  img.src = src;
  return img;
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
  let serial = 0;
  for (const node of template.content.querySelectorAll("[id]")) {
    const old = node.id;
    const id = `r-${namespace}-${serial++}`;
    if (!ids.has(old)) ids.set(old, id);
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
