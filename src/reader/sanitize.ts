import DOMPurify from "dompurify";

// One audited sink. Generated output only, never direct document HTML.
export function sanitizePreview(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
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
}
