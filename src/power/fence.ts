import type { Block, Result } from "./bridge";

export type FenceKind = "shell" | "sqlite";
export interface Fence {
  kind: FenceKind;
  line: number;
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
  const marker = /^ {0,3}(`{3,}|~{3,})/.exec(open);
  if (!marker) return false;
  const end = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(close);
  return (
    !!end && end[1][0] === marker[1][0] && end[1].length >= marker[1].length
  );
}

export function matchesFence(block: Block, fence: Fence) {
  if (block.kind !== fence.kind || block.line !== fence.line) return false;
  return block.code.replace(/\r\n?/g, "\n") === fence.code;
}
