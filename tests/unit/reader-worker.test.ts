import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Index } from "../../src/reader/model";
const engine = vi.hoisted(() => ({ index: vi.fn(), find: vi.fn() }));
vi.mock("../../src/reader/indexer", () => ({
  buildIndex: engine.index,
  findDocument: engine.find,
  renderText: vi.fn(),
}));
beforeEach(() => {
  vi.resetModules();
  engine.index.mockReset();
  engine.find.mockReset();
});
afterEach(() => vi.unstubAllGlobals());

async function worker(indexed = true, complete = true) {
  const endpoint = {
    postMessage: vi.fn(),
    onmessage: undefined as
      undefined | ((event: { data: Record<string, unknown> }) => Promise<void>),
  };
  const index: Index = {
    sections: [{ start: 0, end: 4 }],
    headings: [],
    context: { references: {}, notes: {} },
    complete,
    scanned: 4,
    total: 8,
  };
  engine.index.mockImplementation(async (_read, publish) => {
    publish(index);
    return index;
  });
  vi.stubGlobal("self", endpoint);
  await import("../../src/reader/paged.worker");
  const send = (data: Record<string, unknown>) => endpoint.onmessage!({ data });
  if (indexed) await send({ type: "start" });
  return { endpoint, index, send };
}

it("answers requests before the first index instead of leaving them pending", async () => {
  const { endpoint, send } = await worker(false);
  for (const type of ["find", "page"]) {
    await send({ type, id: 7, query: "text", section: 0 });
    expect(endpoint.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        type,
        id: 7,
        error: expect.stringContaining("index is not available"),
      }),
    );
  }
});

it("stops superseded and explicitly canceled scans without publishing their results", async () => {
  const { endpoint, send } = await worker();
  let finish!: (value: unknown[]) => void;
  let canceled!: () => boolean;
  engine.find
    .mockImplementationOnce((_read, _index, _query, stop) => {
      canceled = stop;
      return new Promise((resolve) => {
        finish = resolve;
      });
    })
    .mockResolvedValue([{ section: 0, excerpt: "current" }]);
  const old = send({ type: "find", id: 1, query: "old" });
  await send({ type: "find", id: 2, query: "new" });
  expect(canceled()).toBe(true);
  finish([{ section: 0, excerpt: "old" }]);
  await old;
  expect(
    endpoint.postMessage.mock.calls.some(
      ([m]) => m.type === "find" && m.id === 1,
    ),
  ).toBe(false);
  expect(endpoint.postMessage).toHaveBeenLastCalledWith(
    expect.objectContaining({
      type: "find",
      id: 2,
      matches: [{ section: 0, excerpt: "current" }],
    }),
  );
  engine.find.mockImplementationOnce((_read, _index, _query, stop) => {
    canceled = stop;
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  const cleared = send({ type: "find", id: 3, query: "clear" });
  await send({ type: "cancel-find" });
  expect(canceled()).toBe(true);
  finish([]);
  await cleared;
  expect(
    endpoint.postMessage.mock.calls.some(
      ([m]) => m.type === "find" && m.id === 3,
    ),
  ).toBe(false);
});

it("does not claim a partial search covers context indexed while it was running", async () => {
  const { endpoint, index, send } = await worker(true, false);
  let finish!: (value: unknown[]) => void;
  engine.find.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const pending = send({ type: "find", id: 1, query: "text" });
  index.complete = true;
  finish([]);
  await pending;
  expect(endpoint.postMessage).toHaveBeenLastCalledWith({
    type: "find",
    id: 1,
    matches: [],
    complete: false,
  });
});
