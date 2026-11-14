// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { ReaderClient } from "../../src/reader/client";
afterEach(() => vi.unstubAllGlobals());
it("replaces provisional anchors when the finalized worker reserves late footnotes", async () => {
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
  const update = vi.fn();
  const client = new ReaderClient(
    async () => ({ start: 0, end: 0, total: 0, text: "" }),
    update,
    vi.fn(),
  );
  const heading = { title: "Note-61", id: "h-note-61", section: 0, level: 1 };
  await worker.onmessage!({
    data: {
      type: "index",
      replaceFrom: 0,
      sections: [{ start: 0, end: 20 }],
      headingsReplaceFrom: 0,
      headings: [heading],
      complete: false,
    },
  });
  await worker.onmessage!({
    data: {
      type: "index",
      replaceFrom: 0,
      sections: [
        { start: 0, end: 20 },
        { start: 20, end: 20, note: "a" },
      ],
      headingsReplaceFrom: 0,
      headings: [
        { ...heading, id: "h-note-61-1" },
        { title: "Footnote a", id: "h-note-61", section: 1, level: 3 },
      ],
      complete: true,
    },
  });
  expect(client.headings.map((h) => h.id)).toEqual([
    "h-note-61-1",
    "h-note-61",
  ]);
  expect(update.mock.lastCall![0].headings).toEqual(client.headings);
  expect(client.complete).toBe(true);
  client.destroy();
});
it("rejects pending and future work after close without retaining approved image data", async () => {
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onmessage: null,
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
    async () => ({ start: 0, end: 0, total: 0, text: "" }),
    () => {},
    () => {},
  );
  client.approveImage("local.png", {
    dataUrl: "data:image/png;base64,AAAA",
    width: 1,
    height: 1,
  });
  const pending = client.page(0);
  client.destroy();
  await expect(pending).rejects.toThrow("Reader closed");
  const sent = worker.postMessage.mock.calls.length;
  await expect(client.page(0)).rejects.toThrow("Reader closed");
  await expect(client.find("query")).rejects.toThrow("Reader closed");
  expect(() =>
    client.approveImage("local.png", {
      dataUrl: "data:image/png;base64,AAAA",
      width: 1,
      height: 1,
    }),
  ).toThrow("Reader closed");
  expect(worker.postMessage).toHaveBeenCalledTimes(sent);
  expect(client.approvedImages.size).toBe(0);
});
it("settles pending work on a worker failure while leaving Source recovery out of its scope", async () => {
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onmessage: null,
    onerror: null as null | (() => void),
  };
  vi.stubGlobal(
    "Worker",
    class {
      constructor() {
        return worker;
      }
    },
  );
  const error = vi.fn();
  const client = new ReaderClient(
    async () => ({ start: 0, end: 0, total: 0, text: "" }),
    () => {},
    error,
  );
  const pending = client.page(0);
  worker.onerror?.();
  await expect(pending).rejects.toThrow("Reader closed");
  expect(error).toHaveBeenCalledWith(
    expect.stringContaining("Source and recovery are preserved"),
  );
  expect(worker.terminate).toHaveBeenCalledTimes(1);
});
