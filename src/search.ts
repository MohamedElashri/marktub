import type { Progress } from "./library";
type SearchArguments = [string, boolean, boolean, boolean];
interface SearchAPI {
  scan: (...args: SearchArguments) => Promise<string>;
  job: (id: string, cancel?: boolean) => Promise<Progress>;
}
export class SearchJobs {
  private id: string | null = null;
  private queue: Promise<unknown> = Promise.resolve();
  constructor(private api: SearchAPI) {}
  private enqueue<T>(action: () => Promise<T>): Promise<T> {
    const task = this.queue.then(action);
    this.queue = task.catch(() => {});
    return task;
  }
  private async stopOwned() {
    const id = this.id;
    if (!id) return;
    try {
      let state = await this.api.job(id, true);
      for (let attempt = 0; state.running && attempt < 50; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 20));
        state = await this.api.job(id);
      }
      if (state.running)
        throw Error(
          "The previous search is still stopping. Try again shortly.",
        );
      this.id = null;
    } catch (error) {
      if (
        typeof error !== "object" ||
        !error ||
        !("code" in error) ||
        error.code !== "stale"
      )
        throw error;
      this.id = null;
    }
  }
  replace(args: SearchArguments, current: () => boolean) {
    return this.enqueue(async () => {
      await this.stopOwned();
      if (!current()) return null;
      const id = await this.api.scan(...args);
      this.id = id;
      if (!current()) {
        await this.stopOwned();
        return null;
      }
      return id;
    });
  }
  stop() {
    return this.enqueue(() => this.stopOwned());
  }
}
