import { buildIndex, renderText, findDocument } from "./indexer";
import { escape } from "./grammar";
import { headingBoundary } from "./headings";
import type { Index, Page, Read } from "./model";
let epoch = 0,
  index: Index | undefined;
let search = 0;
let allowHtml = false;
const awaiting = new Map<
  number,
  {
    resolve: (p: Awaited<ReturnType<Read>>) => void;
    reject: (e: Error) => void;
  }
>();
let serial = 0;
const read: Read = (start) =>
  new Promise((resolve, reject) => {
    const id = ++serial;
    awaiting.set(id, { resolve, reject });
    self.postMessage({ type: "read", id, start });
  });
async function textBetween(start: number, end: number) {
  let text = "";
  while (start < end) {
    const p = await read(start);
    if (p.end <= start) throw Error("Read did not advance.");
    const remaining = end - start;
    const b = new TextEncoder().encode(p.text);
    text += new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
      b.subarray(0, Math.min(remaining, b.length)),
    );
    start = p.end;
  }
  return text;
}
self.onmessage = async ({ data }) => {
  if (data.type === "cancel-find") {
    search++;
    return;
  }
  if (data.type === "range") {
    const pending = awaiting.get(data.id);
    awaiting.delete(data.id);
    if (data.error) pending?.reject(Error(data.error));
    else pending?.resolve(data.page);
    return;
  }
  if (data.type === "start") {
    const current = ++epoch;
    search++;
    index = undefined;
    allowHtml = data.allowHtml === true;
    let sent = 0,
      headings = 0;
    try {
      await buildIndex(
        read,
        (next) => {
          if (epoch !== current) return;
          index = next;
          self.postMessage({
            type: "index",
            replaceFrom: Math.max(0, sent - 1),
            sections: next.sections.slice(Math.max(0, sent - 1)),
            headingsReplaceFrom: next.complete ? 0 : headings,
            headings: next.headings.slice(next.complete ? 0 : headings),
            complete: next.complete,
            scanned: next.scanned,
            total: next.total,
            math: next.complete ? next.math : undefined,
          });
          sent = next.sections.length;
          headings = next.headings.length;
        },
        () => epoch !== current,
        allowHtml,
      );
    } catch (error) {
      self.postMessage({ type: "error", error: String(error) });
    }
    return;
  }
  if (data.type === "page" && index) {
    const current = epoch;
    try {
      const section = index.sections[data.section];
      if (!section) throw Error("Section not indexed yet.");
      const text = section.note
        ? `### Footnote ${section.note}\n\n${index.context.notes[section.note]}`
        : await textBetween(section.start, section.end);
      if (current !== epoch) return;
      const rendered = section.fallback
        ? { html: `<pre>${escape(text)}</pre>`, rich: [] }
        : renderText(
            text,
            index.context,
            index.headings.slice(
              headingBoundary(index.headings, data.section),
              headingBoundary(index.headings, data.section + 1),
            ),
            section.start === 0,
            !section.note,
            allowHtml,
          );
      const result: Page = {
        ...rendered,
        provisional: !index.complete,
        ...(section.fallback ? { fallback: section.fallback } : {}),
      };
      if (index.math?.needsContext) {
        let ordinal = section.mathStart ?? 0;
        for (const rich of result.rich)
          if (rich.kind === "math") rich.mathIndex = ordinal++;
      }
      self.postMessage({ type: "page", id: data.id, result });
    } catch (error) {
      self.postMessage({ type: "page", id: data.id, error: String(error) });
    }
    return;
  }
  if (data.type === "find" && index) {
    const request = ++search;
    const complete = index.complete;
    try {
      const current = epoch;
      const query = String(data.query).slice(0, 200);
      const matches = await findDocument(
        read,
        index,
        query,
        () => current !== epoch || request !== search,
      );
      if (current !== epoch || request !== search) return;
      self.postMessage({
        type: "find",
        id: data.id,
        matches,
        complete,
      });
    } catch (error) {
      if (request !== search) return;
      self.postMessage({ type: "find", id: data.id, error: String(error) });
    }
    return;
  }
  if (data.type === "find" || data.type === "page")
    self.postMessage({
      type: data.type,
      id: data.id,
      error:
        "Document index is not available yet. Retry after content appears.",
    });
};
