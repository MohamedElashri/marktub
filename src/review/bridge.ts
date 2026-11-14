import { invoke } from "@tauri-apps/api/core";
import type { Hunk } from "../git/bridge";
export interface Anchor {
  revision: string;
  start: number;
  end: number;
  lineStart: number;
  lineEnd: number;
  quoteHash: string;
  quote: string;
  prefix: string;
  suffix: string;
  headingStart: number | null;
  headingEnd: number | null;
  headingHash: string | null;
}
export interface Reply {
  id: string;
  text: string;
}
export interface Comment {
  id: string;
  anchor: Anchor;
  previous: Anchor[];
  text: string;
  replies: Reply[];
  resolved: boolean;
}
export interface CommentView {
  comment: Comment;
  status: string;
  start: number | null;
  end: number | null;
  lineStart: number | null;
}
export interface View {
  record: string;
  token: string;
  version: number;
  currentHash: string;
  text: string;
  reviewedHash: string | null;
  reviewedText: string | null;
  viewedHash: string | null;
  viewedText: string | null;
  initial: boolean;
  decision: string;
  decisionRevision: string | null;
  comments: CommentView[];
  snapshots: number;
  renamedTo: string | null;
}
export interface Comparison {
  left: string;
  right: string;
  hunks: Hunk[];
}
export interface Entry {
  id: string;
  key: string;
  name: string;
}
export const begin = (id: string) => invoke<View>("review_begin", { id });
export const sync = (token: string, id: string, disk = false) =>
  invoke<View>("review_sync", { token, id, disk });
export const displayed = (token: string, id: string) =>
  invoke<void>("review_displayed", { token, id });
export const end = (token: string) => invoke<void>("review_end", { token });
export const update = (
  token: string,
  id: string,
  version: number,
  revision: string,
  action: Record<string, unknown>,
) => invoke<View>("review_update", { token, id, version, revision, action });
export const compare = (token: string, id: string, baseline: string) =>
  invoke<Comparison>("review_compare", { token, id, baseline });
export const report = (
  token: string,
  id: string,
  includePath: boolean,
  snippets: boolean,
) => invoke<string>("review_export", { token, id, includePath, snippets });
export const save = (
  token: string,
  id: string,
  includePath: boolean,
  snippets: boolean,
  format: string,
) =>
  invoke<boolean>("review_save", { token, id, includePath, snippets, format });
export const list = () => invoke<Entry[]>("review_list");
export const remove = (record: string) =>
  invoke<void>("review_delete", { record });
export const archiveReport = (record: string, snippets: boolean) =>
  invoke<string>("review_archive_export", { record, snippets });
export const archiveSave = (
  record: string,
  snippets: boolean,
  format: string,
) => invoke<boolean>("review_archive_save", { record, snippets, format });
export function markdownReport(json: string) {
  const fence = "`".repeat(
    Math.max(3, ...(json.match(/`+/g) ?? []).map((s) => s.length + 1)),
  );
  return `# Document review feedback\n\nVerifiable saved-byte anchors; author attribution is not inferred.\n\n${fence}json\n${json}\n${fence}\n`;
}
export function copyPrepared(text: string, field: HTMLTextAreaElement) {
  field.value = text;
  field.focus();
  field.select();
  if (!document.execCommand("copy"))
    throw Error(
      "Use the system Copy command on the selected feedback, or export a file.",
    );
}
