import { invoke } from "@tauri-apps/api/core";
import type { Descriptor } from "../bridge";
export interface Change {
  id: string;
  path: string;
  staged: string;
  working: string;
  warning: string | null;
}
export interface Commit {
  id: string;
  short: string;
  summary: string;
  time: number;
}
export interface State {
  id: string;
  name: string;
  branch: string;
  head: string | null;
  files: Change[];
  total: number;
  history: Commit[];
  warning: string | null;
}
export interface Hunk {
  id: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  unifiedRow: number;
  splitRow: number;
  unifiedFullRow: number;
  splitFullRow: number;
}
export interface Diff {
  id: string;
  path: string;
  before: string;
  after: string;
  hunks: Hunk[];
  unifiedRows: number;
  splitRows: number;
  unifiedFullRows: number;
  splitFullRows: number;
  mutation: string | null;
  conversion: string;
}
export interface Row {
  oldNumber: number | null;
  newNumber: number | null;
  oldText: string | null;
  newText: string | null;
  kind: string;
  hunk: string | null;
  hidden: number;
  clipped: boolean;
  oldEnding: string;
  newEnding: string;
  sourceLine: number | null;
}
export interface Backup {
  id: string;
  path: string;
  created: number;
}
export const choose = () => invoke<State | null>("git_choose");
export const refresh = (id: string, start = 0) =>
  invoke<State>("git_refresh", { id, start });
export const revoke = (id: string) => invoke<void>("git_revoke", { id });
export const compare = (
  id: string,
  file: string,
  before: string,
  after: string,
) => invoke<Diff>("git_compare", { id, file, before, after });
export const page = (
  id: string,
  preview: string,
  start: number,
  split: boolean,
  expanded: boolean,
) => invoke<Row[]>("git_page", { id, preview, start, split, expanded });
export const mutate = (
  id: string,
  preview: string,
  action: string,
  hunks: string[],
) =>
  invoke<{ message: string; backup: string | null }>("git_mutate", {
    id,
    preview,
    action,
    hunks,
  });
export const open = (id: string, file: string) =>
  invoke<Descriptor>("git_open", { id, file });
export const backups = (id: string) => invoke<Backup[]>("git_backups", { id });
export const exportBackup = (id: string, backup: string) =>
  invoke<boolean>("git_export_backup", { id, backup });
export const forgetBackup = (id: string, backup: string) =>
  invoke<void>("git_forget_backup", { id, backup });

export const historyFiles = (id: string, commit: string, start = 0) =>
  invoke<{ files: Change[]; total: number }>("git_history_files", {
    id,
    commit,
    start,
  });
