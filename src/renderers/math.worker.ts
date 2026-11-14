import { mathjaxDocument } from "./mathjax";
import { authorMathSpans, mathPlanForText } from "../reader/math-context";
import type { MathRequest } from "./math-work";
self.onmessage = ({ data }: MessageEvent<MathRequest>) => {
  try {
    if (data.type === "author") {
      if (data.text.length > 2 * 1024 * 1024 + 8192)
        throw Error("Author math input exceeds its 2 MiB budget.");
      const plan = mathPlanForText(data.text);
      if (!plan.needsContext) {
        self.postMessage({ results: [], local: true });
        return;
      }
      if (plan.issue) throw Error(plan.issue);
      self.postMessage({
        results: mathjaxDocument(plan.expressions),
        spans: authorMathSpans(data.text, plan),
      });
    } else if (data.type === "document")
      self.postMessage({ results: mathjaxDocument(data.expressions) });
    else throw Error("Unknown document math request.");
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};
