type Token = ReturnType<ReturnType<typeof grammar>["parse"]>[number];
import type { MathExpression, MathPlan, MathSpan } from "./model";
import { mathContextIssue } from "../renderers/math-policy";
import { grammar, emptyContext } from "./grammar";

export const MATH_EXPRESSIONS = 128;
export const MATH_SOURCE = 128 * 1024;
export function mathExpressions(tokens: Token[]): MathExpression[] {
  const found: MathExpression[] = [];
  for (const token of tokens) {
    if (token.type === "math" || token.type === "math_block")
      found.push({
        source: token.content,
        display: token.type === "math_block" || !!token.meta?.display,
      });
    // Image alt text, code and definition bodies are not displayed formulas.
    if (token.type === "inline" && token.children)
      found.push(...mathExpressions(token.children));
  }
  return found;
}
export function appendMath(plan: MathPlan, expressions: MathExpression[]) {
  for (const expression of expressions) {
    plan.needsContext ||=
      !!mathContextIssue(expression.source) ||
      /\\(?:def|let|newcommand|renewcommand|providecommand|newenvironment|renewenvironment|DeclareMathOperator|DeclarePairedDelimiter|DeclarePairedDelimiterX|DeclarePairedDelimiterXPP|Newextarrow)\b/.test(
        expression.source,
      );
    if (plan.issue) continue;
    plan.sourceUnits += expression.source.length;
    if (
      plan.expressions.length >= MATH_EXPRESSIONS ||
      plan.sourceUnits > MATH_SOURCE ||
      expression.source.length > 8192
    ) {
      plan.issue =
        "Document math exceeds 128 formulas / 128 KiB source / 8,192 characters per formula. Context-dependent math remains source.";
      plan.expressions.length = 0;
    } else plan.expressions.push(expression);
  }
}
export const emptyMathPlan = (): MathPlan => ({
  expressions: [],
  sourceUnits: 0,
  needsContext: false,
});
export function mathPlanForText(text: string) {
  const md = grammar();
  const env = { ...emptyContext(), beginning: true };
  const plan = emptyMathPlan();
  appendMath(plan, mathExpressions(md.parse(text, env)));
  for (const [id, note] of Object.entries(env.notes))
    appendMath(
      plan,
      mathExpressions(
        md.parse(`### Footnote ${id}\n\n${note}`, { ...env, beginning: false }),
      ),
    );
  return plan;
}
export function authorMathSpans(text: string, plan: MathPlan): MathSpan[] {
  const offsets = [0];
  for (let i = 0; i < text.length; i++)
    if (text[i] === "\n") offsets.push(i + 1);
  const tokens = grammar().parse(text, { ...emptyContext(), beginning: true });
  const spans: MathSpan[] = [];
  let index = 0;
  for (const token of tokens) {
    if (token.type === "math_block") {
      index++;
      continue;
    }
    if (token.type !== "inline") continue;
    const start = offsets[token.map?.[0] ?? 0];
    const end = offsets[token.map?.[1] ?? offsets.length] ?? text.length;
    const raw = text.slice(start, end),
      at = raw.indexOf(token.content);
    // Only exact, unambiguous inline spellings receive author controls.
    const exact = at >= 0 && raw.indexOf(token.content, at + 1) < 0;
    for (const child of token.children ?? []) {
      if (child.type !== "math") continue;
      const from = child.meta?.from,
        to = child.meta?.to;
      const expression = plan.expressions[index];
      if (
        exact &&
        typeof from === "number" &&
        Number.isInteger(from) &&
        typeof to === "number" &&
        Number.isInteger(to) &&
        expression?.source === child.content &&
        expression.display === !!child.meta?.display
      )
        spans.push({
          from: start + at + from,
          to: start + at + to,
          ...expression,
          index,
        });
      index++;
    }
  }
  return spans;
}
