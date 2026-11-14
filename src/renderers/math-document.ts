import type { MathPlan } from "../reader/model";
import { MathPolicyError } from "./math-policy";
import { MathWork } from "./math-work";

export class DocumentMath {
  private pending?: Promise<string[]>;
  private dead = false;
  private work?: MathWork;
  constructor(private plan: MathPlan) {}
  async render(
    index: number,
    source: string,
    display: boolean,
    current = () => true,
  ) {
    const check = () => {
      if (this.dead || !current())
        throw new DOMException(
          "Document math view closed or changed.",
          "AbortError",
        );
    };
    check();
    if (this.plan.issue) throw new MathPolicyError(this.plan.issue);
    const expected = this.plan.expressions[index];
    if (!expected || expected.source !== source || expected.display !== display)
      throw new MathPolicyError(
        "Math no longer matches the finalized document context. Source preserved.",
      );
    if (!this.pending) {
      const count = this.plan.expressions.length;
      this.work = new MathWork({
        type: "document",
        expressions: this.plan.expressions,
      });
      this.pending = this.work.result.then(({ results }) => {
        if (results.length !== count)
          throw new MathPolicyError(
            "Invalid document math result. Source preserved.",
          );
        return results;
      });
    }
    const results = await this.pending;
    check();
    return results[index];
  }
  destroy() {
    this.dead = true;
    this.work?.destroy();
    this.work = undefined;
    this.pending = undefined;
    this.plan = { expressions: [], sourceUnits: 0, needsContext: false };
  }
}
