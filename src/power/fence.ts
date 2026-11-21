import type { Block, Result } from "./bridge";

export type FenceKind = "shell" | "sqlite";
export interface Fence {
  kind: FenceKind;
  line: number;
  indent: number;
  code: string;
}
export interface InlineTarget extends Fence {
  document: string;
}
export interface InlineRun {
  pending?: boolean;
  error?: string;
  result?: Result;
}

export function fenceKind(info: string): FenceKind | undefined {
  if (["sh", "bash", "shell", "zsh"].includes(info)) return "shell";
  if (["sql", "sqlite"].includes(info)) return "sqlite";
}

export function closedFence(open: string, close: string) {
  const marker = /^( {0,3})(`{3,}|~{3,})/.exec(open);
  if (!marker) return;
  const end = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(close);
  if (end && end[1][0] === marker[2][0] && end[1].length >= marker[2].length)
    return marker[1].length;
}

export function matchesFence(block: Block, fence: Fence) {
  if (block.kind !== fence.kind || block.line !== fence.line) return false;
  const normalized = block.code.replace(/\r\n?/g, "\n");
  const displayed = fence.indent
    ? normalized.replace(new RegExp(`^ {1,${fence.indent}}`, "gm"), "")
    : normalized;
  return displayed === fence.code;
}
