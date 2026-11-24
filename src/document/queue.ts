import type {
  Ack,
  Batch,
  Change,
  EditDescriptor,
  EditState,
  Transport,
} from "./protocol";
export const MAX_QUEUE = 2 * 1024 * 1024;
const encoder = new TextEncoder();
export const wireBytes = (value: unknown) =>
  encoder.encode(JSON.stringify(value)).length;
export class EditQueue {
  revision: number;
  state: EditState;
  pending: Batch[] = [];
  receipts = new Map<string, Ack>();
  error: unknown = null;
  batchDelay = 500;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private flushTimer: ReturnType<typeof setTimeout> | undefined;
  private flushing: Promise<void> | undefined;
  private flight: Promise<void> | undefined;
  private fence = Infinity;
  private dead = false;
  retained: () => string[] = () => [];
  constructor(
    readonly descriptor: Omit<EditDescriptor, "text">,
    private transport: Transport,
    private notify: () => void,
  ) {
    this.revision = descriptor.state.receivedRevision;
    this.state = descriptor.state;
    this.scheduleFlush();
  }
  private scheduleFlush() {
    if (
      this.flushTimer !== undefined ||
      this.flushing ||
      this.error ||
      this.dead ||
      this.fence !== Infinity ||
      this.revision <= this.state.durableRevision
    )
      return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = undefined;
      void this.flush().catch(() => {});
    }, 1000);
  }
  private cancelFlushTimer() {
    clearTimeout(this.flushTimer);
    this.flushTimer = undefined;
  }
  private scheduleDrain(delay = this.batchDelay) {
    if (
      this.timer !== undefined ||
      this.flushing ||
      this.dead ||
      this.error ||
      this.state.receivedRevision >= Math.min(this.revision, this.fence)
    )
      return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.drain(Math.min(this.revision, this.fence)).catch(() => {});
    }, delay);
  }
  private emit() {
    if (!this.dead) this.notify();
  }
  get bytes() {
    return wireBytes(this.pending);
  }
  get dirty() {
    return this.revision !== this.state.receivedRevision || this.state.dirty;
  }
  get durable() {
    return this.revision === this.state.durableRevision && !this.error;
  }
  canAccept(changes: Change[]) {
    if (
      !changes.length ||
      changes.length > 1024 ||
      wireBytes(changes) > 1024 * 1024
    )
      return false;
    return this.bytes + wireBytes(changes) + 512 < MAX_QUEUE;
  }
  enqueue(changes: Change[]): string {
    if (!this.canAccept(changes))
      throw new Error(
        "Recovery backlog is full. Retry recovery before entering more text.",
      );
    const operationId = crypto.randomUUID();
    this.pending.push({
      schema: 1,
      generation: this.descriptor.journalGeneration,
      operationId,
      baseRevision: this.revision,
      newRevision: ++this.revision,
      changes,
    });
    this.scheduleDrain();
    this.scheduleFlush();
    this.emit();
    return operationId;
  }
  private async drain(target: number): Promise<void> {
    if (this.dead) throw new Error("Editing session is closed.");
    if (this.flight) {
      await this.flight;
      if (this.state.receivedRevision < target) return this.drain(target);
      return;
    }
    while (this.state.receivedRevision < target) {
      const chunk: Batch[] = [];
      for (const batch of this.pending) {
        if (batch.newRevision > target || chunk.length >= 64) break;
        if (chunk.length && wireBytes([...chunk, batch]) > MAX_QUEUE - 1024)
          break;
        chunk.push(batch);
      }
      if (!chunk.length)
        throw new Error("Missing queued revision; input was kept.");
      const retained = [
        ...new Set([
          ...this.retained(),
          ...this.pending.map((b) => b.operationId),
          ...this.pending.flatMap((b) =>
            b.changes.flatMap((c) =>
              c.insert.kind === "restore" ? [c.insert.token.split(":")[0]] : [],
            ),
          ),
        ]),
      ];
      const send = async () => {
        try {
          const result = await this.transport.apply(chunk, retained);
          if (result.acknowledgements.length !== chunk.length)
            throw new Error("Incomplete edit acknowledgement.");
          for (let i = 0; i < chunk.length; i++) {
            const ack = result.acknowledgements[i];
            if (
              ack.generation !== this.descriptor.journalGeneration ||
              ack.appliedRevision !== chunk[i].newRevision ||
              ack.durableRevision > ack.receivedRevision ||
              ack.receivedRevision > this.revision
            )
              throw new Error(
                "Stale or invalid edit acknowledgement; queued input was kept.",
              );
          }
          if (
            result.state.receivedRevision !==
              chunk[chunk.length - 1].newRevision ||
            result.state.durableRevision > result.state.receivedRevision
          )
            throw new Error("Out-of-order editing state; input was kept.");
          result.acknowledgements.forEach((ack, i) =>
            this.receipts.set(chunk[i].operationId, ack),
          );
          this.pending.splice(0, chunk.length);
          this.state = result.state;
          this.error = null;
          const keep = new Set([
            ...this.retained(),
            ...this.pending.map((b) => b.operationId),
          ]);
          for (const id of this.receipts.keys())
            if (!keep.has(id)) this.receipts.delete(id);
        } catch (error) {
          this.error = error;
          throw error;
        } finally {
          this.emit();
        }
      };
      this.flight = send();
      try {
        await this.flight;
      } finally {
        this.flight = undefined;
      }
    }
  }
  async flush(target = Math.min(this.revision, this.fence)) {
    while (this.flushing) {
      await this.flushing;
      if (this.state.durableRevision >= target) return;
    }
    this.cancelFlushTimer();
    clearTimeout(this.timer);
    this.timer = undefined;
    this.flushing = this.persist(target);
    try {
      await this.flushing;
    } finally {
      this.flushing = undefined;
      this.scheduleDrain();
      this.scheduleFlush();
    }
  }
  private async persist(target: number) {
    try {
      await this.drain(target);
      const state = await this.transport.flush();
      if (
        state.receivedRevision !== this.state.receivedRevision ||
        state.durableRevision < target ||
        state.durableRevision > state.receivedRevision
      )
        throw new Error("Invalid durable acknowledgement; input was kept.");
      this.state = state;
      this.error = null;
    } catch (error) {
      this.error = error;
      throw error;
    } finally {
      this.emit();
    }
  }
  async saveBarrier() {
    const revision = this.pause();
    try {
      await this.flush(revision);
      return revision;
    } catch (error) {
      this.resume();
      throw error;
    }
  }
  pause() {
    this.cancelFlushTimer();
    this.fence = this.revision;
    return this.fence;
  }
  resume() {
    this.fence = Infinity;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.scheduleDrain(0);
    this.scheduleFlush();
    this.emit();
  }
  saved(state: EditState) {
    this.state = state;
    this.error = null;
    this.emit();
  }
  dispose() {
    this.dead = true;
    clearTimeout(this.timer);
    this.cancelFlushTimer();
  }
}
