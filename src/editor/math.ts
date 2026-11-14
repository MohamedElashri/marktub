import { grammar, emptyContext } from "../reader/grammar";
import { MathWork } from "../renderers/math-work";
import { renderRich } from "../renderers/rich";
import { MathPolicyError } from "../renderers/math-policy";
// Read-only rendering projection. Full document parsing/compilation stays in an
// owned worker; no preview dispatches a transaction or supplies saved Markdown.
export class AuthorMath {
  private work?: MathWork;
  private dead = false;
  private contextual: boolean;
  constructor(private text: string) {
    if (text.length > 2 * 1024 * 1024 + 8192)
      throw new MathPolicyError(
        "Full-document author math admits ordinary documents up to 2 MiB. Use Read for large documents.",
      );
    // A conservative hint only: the worker's shared grammar decides actual context.
    this.contextual =
      /\\(?:label|ref|eqref|autoref|nameref|pageref|gdef|xdef|global|globaldefs|csname|catcode|numberwithin|refstepcounter|setcounter|addtocounter|def|let|newcommand|renewcommand|providecommand|newenvironment|renewenvironment|DeclareMathOperator|DeclarePairedDelimiter\w*|Newextarrow|begin)\b/.test(
        text,
      );
  }
  async render(
    from: number,
    to: number,
    source: string,
    current: () => boolean,
  ): Promise<string> {
    const live = () => !this.dead && current();
    if (!live())
      throw new DOMException("Author math view closed.", "AbortError");
    if (this.contextual) {
      this.work ??= new MathWork({ type: "author", text: this.text });
      const result = await this.work.result;
      if (!live())
        throw new DOMException("Author math view closed.", "AbortError");
      if (!result.local) {
        const match = result.spans?.find(
          (span) =>
            span.from <= from && span.to >= to && span.source === source,
        );
        if (!match)
          throw new MathPolicyError(
            "Math preview has no verified full-document span. Source preserved; use Read.",
          );
        return renderRich(
          {
            kind: "math",
            source,
            display: match.display,
            mathIndex: match.index,
          },
          "auto",
          live,
          { render: async () => result.results[match.index] },
        );
      }
    }
    let start = from;
    if (this.text.startsWith("$$" + source + "$$", from)) start += 2;
    else if (this.text.startsWith("$" + source + "$", from)) start += 1;
    const width =
      this.text.slice(start - 2, start) === "$$" &&
      this.text.slice(start + source.length, start + source.length + 2) === "$$"
        ? 2
        : 1;
    const spelling = this.text.slice(
      start - width,
      start + source.length + width,
    );
    const tokens = grammar().parseInline(spelling, { ...emptyContext() })[0]
      ?.children;
    if (
      source.length > 8192 ||
      to > start + source.length + width ||
      tokens?.length !== 1 ||
      tokens[0].type !== "math" ||
      tokens[0].content !== source
    )
      throw new MathPolicyError(
        "Math preview has no verified source span. Source preserved.",
      );
    return renderRich(
      { kind: "math", source, display: !!tokens[0].meta?.display },
      "auto",
      live,
    );
  }
  destroy() {
    this.dead = true;
    this.work?.destroy();
    this.work = undefined;
    this.text = "";
  }
}
