import { afterEach, expect, it, vi } from "vitest";
import { buildIndex, findDocument } from "../../src/reader/indexer";
import { emptyContext } from "../../src/reader/grammar";
import { ReaderClient } from "../../src/reader/client";
import type { Index, Read } from "../../src/reader/model";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function input(text: string, width = 65536): Read {
  const bytes = new TextEncoder().encode(text);
  return async (start) => {
    let end = Math.min(start + width, bytes.length);
    while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    return {
      start,
      end,
      total: bytes.length,
      text: new TextDecoder("utf-8", { ignoreBOM: true }).decode(
        bytes.subarray(start, end),
      ),
    };
  };
}

it("bounds UTF-8 encoding work for a common query with many hits in one section", async () => {
  const text = "a".repeat(16384);
  const read = input(text);
  const index: Index = {
    sections: [{ start: 0, end: text.length }],
    headings: [],
    context: emptyContext(),
    complete: true,
    scanned: text.length,
    total: text.length,
  };
  const encode = TextEncoder.prototype.encode;
  let units = 0;
  vi.spyOn(TextEncoder.prototype, "encode").mockImplementation(function (
    this: TextEncoder,
    value = "",
  ) {
    units += value.length;
    return encode.call(this, value);
  });
  const matches = await findDocument(read, index, "a");
  expect(matches.map((match) => match.section)).toEqual([0]);
  expect(units).toBeLessThanOrEqual(text.length * 4);
});

it("keeps UTF-8 section offsets correct for Arabic, astral characters and Unicode case folding", async () => {
  const text =
    "\uFEFF" +
    Array.from(
      { length: 8 },
      (_, i) => `## Chapter ${i}\r\n\r\n${"ع🦀ſa ".repeat(600)}\r\n\r\n`,
    ).join("");
  const read = input(text, 97);
  const index = await buildIndex(read, () => {});
  const bytes = new TextEncoder().encode(text);
  for (const query of ["S", "ع🦀", "🦀ſa"])
    expect(
      (await findDocument(read, index, query)).map((match) => match.section),
    ).toEqual(
      index.sections.flatMap((section, i) =>
        new RegExp(query, "iu").test(
          new TextDecoder().decode(bytes.subarray(section.start, section.end)),
        )
          ? [i]
          : [],
      ),
    );
});

it("cancels cleared queries without leaving a pending promise or admitting a late result", async () => {
  const { client, worker } = reader();
  try {
    const pending = client.find("old");
    const retired = expect(pending).rejects.toMatchObject({
      name: "AbortError",
    });
    const request = worker.postMessage.mock.calls.at(-1)![0];
    client.cancelFind();
    await retired;
    expect(worker.postMessage).toHaveBeenLastCalledWith({
      type: "cancel-find",
    });
    await worker.onmessage!({
      data: {
        type: "find",
        id: request.id,
        matches: [{ section: 0, excerpt: "stale" }],
        complete: true,
      },
    });
  } finally {
    client.destroy();
  }
});

it("does not process a source page that arrives after search cancellation", async () => {
  let canceled = false;
  const read: Read = async () => {
    canceled = true;
    return { start: 0, end: 4, total: 4, text: "aaaa" };
  };
  const index: Index = {
    sections: [{ start: 0, end: 4 }],
    headings: [],
    context: emptyContext(),
    complete: true,
    scanned: 4,
    total: 4,
  };
  expect(await findDocument(read, index, "a", () => canceled)).toEqual([]);
});

function reader() {
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onmessage: null as null | ((event: { data: unknown }) => Promise<void>),
    onerror: null,
  };
  vi.stubGlobal(
    "Worker",
    class {
      constructor() {
        return worker;
      }
    },
  );
  const client = new ReaderClient(
    input("text"),
    () => {},
    () => {},
  );
  return { client, worker };
}

it("retires superseded searches immediately without canceling a page request", async () => {
  const { client, worker } = reader();
  try {
    const first = client.find("old");
    const retired = expect(first).rejects.toMatchObject({ name: "AbortError" });
    const page = client.page(0);
    const latest = client.find("new");
    void page.catch(() => {});
    void latest.catch(() => {});
    await retired;
    const request = (type: string) =>
      worker.postMessage.mock.calls
        .map(([message]) => message)
        .filter((message) => message.type === type)
        .at(-1);
    await worker.onmessage!({
      data: {
        type: "find",
        id: request("find").id,
        matches: [{ section: 1, excerpt: "new" }],
        complete: true,
      },
    });
    await worker.onmessage!({
      data: {
        type: "page",
        id: request("page").id,
        result: { html: "kept", rich: [], provisional: false },
      },
    });
    expect((await latest).matches[0].excerpt).toBe("new");
    expect((await page).html).toBe("kept");
  } finally {
    client.destroy();
  }
});
