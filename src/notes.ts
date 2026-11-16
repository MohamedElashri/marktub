import type { NoteCapture, NoteCatalogue } from "./library";
import { grammar, emptyContext } from "./reader/grammar";
import { wikiHref } from "./reader/wikilinks";
import { safeLink } from "./reader/safe";

export interface NoteReference {
  target: string;
  wiki: boolean;
}
const parser = grammar();
export function noteReferences(text: string) {
  const tokens = parser.parse(text.replace(/^\uFEFF/, ""), {
    ...emptyContext(),
    beginning: true,
  });
  const links: NoteReference[] = [];
  let clipped = false;
  const seen = new Set<string>();
  for (const block of tokens)
    for (const token of block.children ?? []) {
      const reference =
        token.type === "wiki_link"
          ? { target: String(token.meta?.target ?? ""), wiki: true }
          : token.type === "link_open" && token.attrGet("href")
            ? { target: String(token.attrGet("href")), wiki: false }
            : null;
      if (
        !reference ||
        (!reference.wiki && !safeLink(reference.target)) ||
        seen.has(JSON.stringify(reference))
      )
        continue;
      if (links.length >= 128) {
        clipped = true;
        break;
      }
      seen.add(JSON.stringify(reference));
      links.push(reference);
    }
  return { links, clipped };
}
function normalized(base: string, path: string) {
  const parts = base ? base.split("/") : [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(part);
  }
  return parts.join("/");
}
export function resolveNote(
  catalogue: NoteCatalogue,
  from: string,
  reference: NoteReference,
) {
  const empty = { matches: [] as NoteCapture[], definite: false, fragment: "" };
  if (
    reference.wiki ? !wikiHref(reference.target) : !safeLink(reference.target)
  )
    return empty;
  let path: string, fragment: string;
  try {
    const hash = reference.target.indexOf("#");
    path = hash < 0 ? reference.target : reference.target.slice(0, hash);
    fragment = hash < 0 ? "" : reference.target.slice(hash + 1);
    if (!reference.wiki) {
      path = decodeURIComponent(path);
      fragment = decodeURIComponent(fragment);
    }
  } catch {
    return empty;
  }
  if (
    Array.from(path).some(
      (c) =>
        c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127 || "\\:?#".includes(c),
    ) ||
    path.startsWith("/")
  )
    return empty;
  const parent = from.split("/").slice(0, -1).join("/");
  const explicit = /\.(?:md|markdown)$/i.test(path);
  const names = !path
    ? [from]
    : reference.wiki && !explicit
      ? [path + ".md", path + ".markdown"]
      : [path];
  for (const name of names) {
    const relative = !path
      ? from
      : normalized(
          reference.wiki && path.includes("/") && !path.startsWith(".")
            ? ""
            : parent,
          name,
        );
    const found = catalogue.notes.find((note) => note.path === relative);
    if (found)
      return {
        matches: [found],
        definite: explicit || name === names[0] || catalogue.complete,
        fragment,
      };
  }
  if (!reference.wiki || path.includes("/") || !path)
    return { ...empty, fragment };
  const matches = catalogue.notes.filter((note) =>
    names.includes(note.path.split("/").at(-1)!),
  );
  return {
    matches,
    definite: catalogue.complete && matches.length === 1,
    fragment,
  };
}
export function noteRelationships(catalogue: NoteCatalogue) {
  const outgoing = catalogue.notes.find(
    (note) => note.path === catalogue.active,
  )?.text;
  const references =
    outgoing === null || outgoing === undefined
      ? { links: [], clipped: true }
      : noteReferences(outgoing);
  const backlinks: NoteCapture[] = [];
  let clipped = references.clipped;
  for (const note of catalogue.notes) {
    if (note.path === catalogue.active || note.text === null) continue;
    const parsed = noteReferences(note.text);
    clipped ||= parsed.clipped;
    if (
      parsed.links.some((link) => {
        const result = resolveNote(catalogue, note.path, link);
        return result.definite && result.matches[0]?.path === catalogue.active;
      })
    )
      backlinks.push(note);
  }
  return { outgoing: references.links, backlinks, clipped };
}
