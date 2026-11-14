export class MathPolicyError extends Error {}
// Expression-local TeX only. Inspect control sequences rather than raw substring
// matches so comments and escaped backslashes don't invent document context.
export function mathContextIssue(source: string): string | undefined {
  const global = new Set([
    "label",
    "ref",
    "eqref",
    "autoref",
    "nameref",
    "pageref",
    "newlabel",
    "gdef",
    "xdef",
    "global",
    "globaldefs",
    "csname",
    "catcode",
    "numberwithin",
    "refstepcounter",
    "setcounter",
    "addtocounter",
  ]);
  for (let i = 0; i < source.length; i++) {
    if (source[i] === "%") {
      while (i < source.length && source[i] !== "\n" && source[i] !== "\r") i++;
    } else if (source[i] === "\\") {
      const start = ++i;
      while (i < source.length && /[a-zA-Z]/.test(source[i])) i++;
      const command = source.slice(start, i);
      if (global.has(command)) return `\\${command}`;
      if (command === "begin") {
        const environment =
          /^\s*\{\s*(equation|align|alignat|gather|multline|eqnarray)\s*\}/.exec(
            source.slice(i),
          );
        if (environment) return `numbered ${environment[1]} environment`;
      }
      // Non-letter control symbols consume their following character (\\, \%).
      if (command) i--;
    }
  }
}
export function requireLocalMath(source: string) {
  const issue = mathContextIssue(source);
  if (issue)
    throw new MathPolicyError(
      `Document-wide math context is unsupported (${issue}). Source preserved; use expression-local definitions, an explicit \\tag, or an unnumbered environment.`,
    );
}
