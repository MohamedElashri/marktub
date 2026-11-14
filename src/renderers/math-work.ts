import type { MathExpression, MathSpan } from "../reader/model";
import { MathPolicyError } from "./math-policy";
export type MathRequest =
  | { type: "document"; expressions: MathExpression[] }
  | { type: "author"; text: string };
export interface MathResult {
  results: string[];
  spans?: MathSpan[];
  local?: boolean;
}

export class MathWork {
  readonly result: Promise<MathResult>;
  private stop?: (error: Error) => void;
  constructor(request: MathRequest) {
    this.result = new Promise((resolve, reject) => {
      const worker = new Worker(new URL("./math.worker.ts", import.meta.url), {
        type: "module",
      });
      const retire = () => {
        clearTimeout(timer);
        worker.terminate();
        worker.onmessage = null;
        worker.onerror = null;
        this.stop = undefined;
      };
      this.stop = (error) => {
        retire();
        reject(error);
      };
      const timer = setTimeout(
        () =>
          this.stop?.(
            new MathPolicyError(
              "Document math exceeded its eight-second worker budget. Source preserved.",
            ),
          ),
        8000,
      );
      worker.onerror = () =>
        this.stop?.(
          new MathPolicyError("Document math worker failed. Source preserved."),
        );
      worker.onmessage = ({ data }) => {
        if (data?.error) {
          this.stop?.(new MathPolicyError(String(data.error)));
          return;
        }
        let cost = 0;
        if (
          !Array.isArray(data?.results) ||
          data.results.length > 128 ||
          data.results.some((value: unknown) => {
            if (typeof value !== "string") return true;
            cost += value.length * 2;
            return value.length > 1024 * 1024 || cost > 8 * 1024 * 1024;
          }) ||
          (data.spans !== undefined &&
            (!Array.isArray(data.spans) ||
              data.spans.length > 128 ||
              data.spans.some(
                (span: MathSpan) =>
                  !span ||
                  !Number.isInteger(span.from) ||
                  !Number.isInteger(span.to) ||
                  span.from < 0 ||
                  span.to < span.from ||
                  span.to > 2 * 1024 * 1024 + 8192 ||
                  !Number.isInteger(span.index) ||
                  span.index < 0 ||
                  span.index >= data.results.length ||
                  typeof span.source !== "string" ||
                  span.source.length > 8192 ||
                  typeof span.display !== "boolean",
              )))
        ) {
          this.stop?.(
            new MathPolicyError(
              "Invalid or oversized document math result. Source preserved.",
            ),
          );
          return;
        }
        retire();
        resolve(data);
      };
      worker.postMessage(request);
    });
  }
  destroy() {
    this.stop?.(new DOMException("Document math view closed.", "AbortError"));
  }
}
