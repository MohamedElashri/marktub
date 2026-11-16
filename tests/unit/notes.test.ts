// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { grammar, emptyContext } from "../../src/reader/grammar";
import { sanitizeReader } from "../../src/reader/safe";
import { wikiHref } from "../../src/reader/wikilinks";
import {
  noteReferences,
  noteRelationships,
  resolveNote,
} from "../../src/notes";
import type { NoteCatalogue } from "../../src/library";

function catalogue(
  files: Record<string, string | null>,
  active = "Note.md",
  complete = true,
): NoteCatalogue {
  const root = { id: "root", root: "root", name: "Notes", label: "Notes" };
  return {
    root,
    active,
    complete,
    skipped: 0,
    notes: Object.entries(files).map(([path, text]) => ({
      path,
      text,
      item: {
        id: path,
        name: path.split("/").at(-1)!,
        bytes: text?.length ?? 70000,
        modified: 0,
        kind: "file",
        directory: null,
        parent: root,
      },
    })),
  };
}
describe("saved-note links", () => {
  it("parses aliases and headings while keeping code, escaped links, embeds and front matter literal", () => {
    const parsed = noteReferences(
      '---\nkey: "[[Metadata]]"\n---\n\n[[Note#Heading|a label]] and [a file][later].\n\n`[[Inline]]` \\[[Escaped]] ![[Embedded]]\n\n```md\n[[Fenced]]\n```\n\n[later]: folder/Other.md\n',
    );
    expect(parsed.links).toEqual([
      { target: "Note#Heading", wiki: true },
      { target: "folder/Other.md", wiki: false },
    ]);
    expect(parsed.clipped).toBe(false);
    const html = sanitizeReader(
      grammar().render("[[Note|<img src=x onerror=alert(1)>]]", {
        ...emptyContext(),
        beginning: true,
      }),
    );
    expect(html).toContain('data-note-link="Note"');
    expect(html).toContain("&lt;img");
    expect(html).not.toContain("<img");
    for (const value of [
      "https://example.com",
      "//host/x",
      "\\network",
      "bad\nname",
      "bad[name]",
    ])
      expect(wikiHref(value)).toBeNull();
  });
  it("prefers exact same-folder files and supports explicit root paths, aliases and Unicode", () => {
    const data = catalogue(
      { "projects/Note.md": "", "Note.md": "", "folder/文.md": "" },
      "projects/Start.md",
    );
    expect(
      resolveNote(data, data.active, { target: "Note#Heading", wiki: true })
        .matches[0].path,
    ).toBe("projects/Note.md");
    expect(
      resolveNote(data, data.active, { target: "folder/文", wiki: true })
        .matches[0].path,
    ).toBe("folder/文.md");
    expect(
      resolveNote(data, data.active, {
        target: "../folder/%E6%96%87.md#Heading",
        wiki: false,
      }).matches[0].path,
    ).toBe("folder/文.md");
    expect(
      resolveNote(data, data.active, { target: "../../outside", wiki: true })
        .matches,
    ).toEqual([]);
  });
  it("does not invent a unique destination from ambiguous names or incomplete scans", () => {
    const data = catalogue(
      { "one/Note.md": "", "two/Note.md": "" },
      "Start.md",
    );
    expect(
      resolveNote(data, data.active, { target: "Note", wiki: true }).matches,
    ).toHaveLength(2);
    expect(
      resolveNote(data, data.active, { target: "Note", wiki: true }).definite,
    ).toBe(false);
    const partial = catalogue({ "one/Note.md": "" }, "Start.md", false);
    expect(
      resolveNote(partial, partial.active, { target: "Note", wiki: true })
        .definite,
    ).toBe(false);
  });
  it("derives backlinks from saved wiki and Markdown references without counting fences or unresolved names", () => {
    const data = catalogue({
      "Note.md": "[[Other]]",
      "Other.md": "[[Note#Heading|label]]",
      "folder/Third.md": "[reference][saved]\n\n[saved]: ../Note.md\n",
      "Code.md": "```md\n[[Note]]\n```\n",
      "large.md": null,
    });
    const result = noteRelationships(data);
    expect(result.outgoing).toEqual([{ target: "Other", wiki: true }]);
    expect(result.backlinks.map((note) => note.path)).toEqual([
      "Other.md",
      "folder/Third.md",
    ]);
    expect(result.clipped).toBe(false);
    expect(
      noteReferences(
        Array.from({ length: 150 }, (_, i) => `[[Note${i}]]`).join(" "),
      ).clipped,
    ).toBe(true);
  });
});
