import { describe, expect, it, vi } from "vitest";
import { SearchJobs } from "../../src/search";
import type { Progress } from "../../src/library";
const finished = { running: false } as Progress;
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
describe("owned workspace searches", () => {
  it("cancels a stale in-flight start before starting the replacement query", async () => {
    const first = deferred<string>();
    const api = {
      scan: vi
        .fn()
        .mockReturnValueOnce(first.promise)
        .mockResolvedValueOnce("new"),
      job: vi.fn().mockResolvedValue(finished),
    };
    const jobs = new SearchJobs(api);
    let current = true;
    const old = jobs.replace(["old", false, false, false], () => current);
    await Promise.resolve();
    await Promise.resolve();
    expect(api.scan).toHaveBeenCalledTimes(1);
    current = false;
    const replacement = jobs.replace(["new", true, false, false], () => true);
    first.resolve("old");
    expect(await old).toBeNull();
    expect(await replacement).toBe("new");
    expect(api.job).toHaveBeenCalledWith("old", true);
    expect(api.scan).toHaveBeenLastCalledWith("new", true, false, false);
    await jobs.stop();
    expect(api.job).toHaveBeenLastCalledWith("new", true);
  });
  it("stops a pending search on close and never cancels an unrelated native job", async () => {
    const pending = deferred<string>();
    const api = {
      scan: vi.fn().mockReturnValue(pending.promise),
      job: vi.fn().mockResolvedValue(finished),
    };
    const jobs = new SearchJobs(api);
    const start = jobs.replace(["note", false, false, false], () => true);
    const close = jobs.stop();
    pending.resolve("owned");
    await start;
    await close;
    await jobs.stop();
    expect(api.job.mock.calls).toEqual([["owned", true]]);
    expect(api.job).not.toHaveBeenCalledWith("foreign", true);
  });
  it("can search again after an expired handle or rejected start without hiding other failures", async () => {
    const api = {
      scan: vi
        .fn()
        .mockResolvedValueOnce("old")
        .mockRejectedValueOnce(Error("busy"))
        .mockResolvedValueOnce("fresh"),
      job: vi
        .fn()
        .mockRejectedValueOnce({ code: "stale" })
        .mockResolvedValue(finished),
    };
    const jobs = new SearchJobs(api);
    await jobs.replace(["old", false, false, false], () => true);
    await expect(
      jobs.replace(["retry", false, false, false], () => true),
    ).rejects.toThrow("busy");
    expect(await jobs.replace(["fresh", false, false, false], () => true)).toBe(
      "fresh",
    );
    await jobs.stop();
    expect(api.job.mock.calls).toEqual([
      ["old", true],
      ["fresh", true],
    ]);
  });
});
