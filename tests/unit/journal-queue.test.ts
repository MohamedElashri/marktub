import { afterEach, describe, expect, it } from "vitest";
import { EditQueue } from "../../src/document/queue";
import type {
  Ack,
  Batch,
  EditDescriptor,
  EditState,
  Transport,
} from "../../src/document/protocol";
const queues: EditQueue[] = [];
afterEach(() => {
  queues.splice(0).forEach((q) => q.dispose());
});
function harness() {
  const descriptor: Omit<EditDescriptor, "text"> = {
    id: "session",
    generation: "transport",
    journalGeneration: "journal",
    state: {
      receivedRevision: 0,
      durableRevision: 0,
      savedRevision: 0,
      dirty: false,
    },
    name: "fixture",
    caret: 0,
    needsSaveAs: false,
  };
  let received = 0,
    durable = 0,
    saved = 0;
  const seen: Batch[][] = [];
  const state = (): EditState => ({
    receivedRevision: received,
    durableRevision: durable,
    savedRevision: saved,
    dirty: received !== saved,
  });
  const receipts = new Map<string, Ack>();
  const transport: Transport = {
    apply: async (batches) => {
      seen.push(structuredClone(batches));
      const acknowledgements = batches.map((b) => {
        const previous = receipts.get(b.operationId);
        if (previous) return previous;
        if (b.baseRevision !== received) throw new Error("out of order");
        received = b.newRevision;
        const ack = {
          generation: "journal",
          appliedRevision: received,
          receivedRevision: received,
          durableRevision: durable,
          inverse: [
            {
              from: 0,
              to: 1,
              insert: { kind: "restore" as const, token: `${b.operationId}:0` },
            },
          ],
        };
        receipts.set(b.operationId, ack);
        return ack;
      });
      return { acknowledgements, state: state() };
    },
    flush: async () => {
      durable = received;
      return state();
    },
  };
  const queue = new EditQueue(descriptor, transport, () => {});
  queues.push(queue);
  return {
    queue,
    transport,
    seen,
    state,
    save: () => {
      saved = received;
      return state();
    },
  };
}
const change = (text: string) => [
  { from: 0, to: 0, insert: { kind: "text" as const, text } },
];
describe("ordered Source recovery queue", () => {
  it("retries an uncertain IPC result with the identical operation ID and coordinates", async () => {
    const h = harness();
    const real = h.transport.apply;
    let first = true;
    h.transport.apply = async (...args) => {
      const result = await real(...args);
      if (first) {
        first = false;
        throw new Error("reply lost after append");
      }
      return result;
    };
    h.queue.enqueue(change("🦀"));
    await expect(h.queue.flush()).rejects.toThrow("reply lost");
    expect(h.queue.pending).toHaveLength(1);
    expect(h.queue.dirty).toBe(true);
    expect(h.queue.durable).toBe(false);
    await h.queue.flush();
    expect(h.queue.pending).toHaveLength(0);
    expect(h.queue.durable).toBe(true);
    expect(h.seen[0]).toEqual(h.seen[1]);
    expect(h.seen[0][0].changes[0].insert).toEqual({
      kind: "text",
      text: "🦀",
    });
  });
  it("distinguishes received input from a failed durable flush", async () => {
    const h = harness();
    const real = h.transport.flush;
    h.transport.flush = async () => {
      throw new Error("disk full");
    };
    h.queue.enqueue(change("pending"));
    await expect(h.queue.flush()).rejects.toThrow("disk full");
    expect(h.queue.state.receivedRevision).toBe(1);
    expect(h.queue.state.durableRevision).toBe(0);
    expect(h.queue.dirty).toBe(true);
    h.transport.flush = real;
    await h.queue.flush();
    expect(h.queue.state.durableRevision).toBe(1);
  });
  it("refuses stale acknowledgements without dropping queued human input", async () => {
    const h = harness();
    const real = h.transport.apply;
    h.transport.apply = async (...args) => {
      const result = await real(...args);
      result.acknowledgements[0].generation = "old-session";
      return result;
    };
    h.queue.enqueue(change("human input"));
    await expect(h.queue.flush()).rejects.toThrow("Stale or invalid");
    expect(h.queue.pending).toHaveLength(1);
    expect(h.queue.dirty).toBe(true);
  });
  it("holds N+1 behind save N while accepting local input and preserving its dirty state", async () => {
    const h = harness();
    let resolve: (() => void) | undefined;
    const gate = new Promise<void>((r) => (resolve = r));
    const real = h.transport.apply;
    let first = true;
    h.transport.apply = async (...args) => {
      if (first) {
        first = false;
        await gate;
      }
      return real(...args);
    };
    h.queue.enqueue(change("N"));
    const barrier = h.queue.saveBarrier();
    h.queue.enqueue(change("N+1"));
    resolve!();
    expect(await barrier).toBe(1);
    expect(h.queue.pending.map((b) => b.newRevision)).toEqual([2]);
    expect(h.state().receivedRevision).toBe(1);
    h.queue.saved(h.save());
    expect(h.queue.dirty).toBe(true);
    h.queue.resume();
    await h.queue.flush();
    expect(h.queue.state.savedRevision).toBe(1);
    expect(h.queue.state.receivedRevision).toBe(2);
    expect(h.queue.dirty).toBe(true);
  });
  it("rejects an oversized batch before changing the local revision", () => {
    const h = harness();
    expect(() => h.queue.enqueue(change("x".repeat(1024 * 1024 + 1)))).toThrow(
      "backlog",
    );
    expect(h.queue.revision).toBe(0);
    expect(h.queue.pending).toHaveLength(0);
  });
});
