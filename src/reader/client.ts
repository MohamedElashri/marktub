import type { Read, Section, Heading, Page } from "./model";
import { DocumentMath } from "../renderers/math-document";
export interface Update {
  sections: Section[];
  headings: Heading[];
  complete: boolean;
  scanned: number;
  total: number;
}
export class ReaderClient {
  math?: DocumentMath;
  approvedImages = new Map<string, string>();
  private imagePixels = new Map<string, number>();
  approveImage(
    link: string,
    image: { dataUrl: string; width: number; height: number },
  ) {
    if (this.dead) throw Error("Reader closed.");
    const bytes =
      [...this.approvedImages.values()].reduce((n, v) => n + v.length, 0) -
      (this.approvedImages.get(link)?.length ?? 0) +
      image.dataUrl.length;
    const pixels =
      [...this.imagePixels.values()].reduce((n, v) => n + v, 0) -
      (this.imagePixels.get(link) ?? 0) +
      image.width * image.height;
    if (
      bytes > 16 * 1024 * 1024 ||
      pixels > 32_000_000 ||
      (!this.approvedImages.has(link) && this.approvedImages.size >= 64)
    )
      throw Error(
        "Approved image budget reached (16 MiB encoded / 32 megapixels / 64 images). Reopen to release approved images.",
      );
    this.approvedImages.set(link, image.dataUrl);
    this.imagePixels.set(link, image.width * image.height);
  }
  private worker: Worker;
  private serial = 0;
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  sections: Section[] = [];
  headings: Heading[] = [];
  complete = false;
  dead = false;
  constructor(
    read: Read,
    update: (v: Update) => void,
    error: (message: string) => void,
  ) {
    this.worker = new Worker(new URL("./paged.worker.ts", import.meta.url), {
      type: "module",
    });
    this.worker.onmessage = async ({ data }) => {
      if (this.dead) return;
      if (data.type === "read") {
        try {
          const page = await read(data.start);
          if (!this.dead)
            this.worker.postMessage({ type: "range", id: data.id, page });
        } catch (e) {
          if (!this.dead)
            this.worker.postMessage({
              type: "range",
              id: data.id,
              error: String(e),
            });
        }
        return;
      }
      if (data.type === "index") {
        this.sections.splice(
          data.replaceFrom,
          this.sections.length - data.replaceFrom,
          ...data.sections,
        );
        this.headings.length = data.headingsReplaceFrom;
        // A finalized index can replace all 100,000 headings. Avoid spreading
        // that array into a call (WebView argument limits differ by platform).
        for (const heading of data.headings) this.headings.push(heading);
        this.complete = data.complete;
        if (data.complete && data.math?.needsContext) {
          this.math?.destroy();
          this.math = new DocumentMath(data.math);
        }
        update({
          ...data,
          sections: [...this.sections],
          headings: [...this.headings],
        });
        return;
      }
      if (data.type === "error") {
        error(data.error);
        return;
      }
      const pending = this.pending.get(data.id);
      this.pending.delete(data.id);
      if (data.error) pending?.reject(Error(data.error));
      else pending?.resolve(data.type === "page" ? data.result : data);
    };
    this.worker.onerror = () => {
      if (this.dead) return;
      this.destroy();
      error(
        "Reader worker failed. Reopen the document to retry. Source and recovery are preserved.",
      );
    };
    this.worker.postMessage({ type: "start" });
  }
  private request<T>(type: string, args: Record<string, unknown>): Promise<T> {
    if (this.dead) return Promise.reject(Error("Reader closed."));
    return new Promise((resolve, reject) => {
      const id = ++this.serial;
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
      });
      this.worker.postMessage({ type, id, ...args });
    });
  }
  page(section: number) {
    return this.request<Page>("page", { section });
  }
  find(query: string) {
    return this.request<{
      matches: { section: number; excerpt: string }[];
      complete: boolean;
    }>("find", { query });
  }
  destroy() {
    this.dead = true;
    this.worker.terminate();
    this.worker.onmessage = null;
    this.worker.onerror = null;
    for (const p of this.pending.values()) p.reject(Error("Reader closed."));
    this.pending.clear();
    this.math?.destroy();
    this.math = undefined;
    this.approvedImages.clear();
    this.imagePixels.clear();
    this.sections.length = 0;
    this.headings.length = 0;
  }
}
