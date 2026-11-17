import DOMPurify from "dompurify";
import { readingDirections } from "./direction";

// One audited sink. Generated output only, never direct document HTML.
export function sanitizePreview(html: string): string {
  const clean = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    RETURN_DOM_FRAGMENT: true,
    FORBID_TAGS: [
      "img",
      "svg",
      "math",
      "style",
      "form",
      "input",
      "iframe",
      "object",
      "embed",
      "video",
      "audio",
      "link",
    ],
    FORBID_ATTR: ["href", "src", "srcset", "style", "id", "name"],
    ALLOW_DATA_ATTR: false,
  });
  const host = document.createElement("template");
  host.content.append(clean);
  readingDirections(host.content);
  return host.innerHTML;
}
