import { invoke } from "@tauri-apps/api/core";
export type Kind = "shell" | "sqlite" | "tex";
export type Role = "shell" | "pandoc" | "typst" | "tex";
export type Mode =
  "pdf" | "pdf_tex" | "shell" | "sqlite_read" | "sqlite_write" | "tex";
export interface Block {
  id: string;
  kind: Kind;
  start: number;
  end: number;
  line: number;
  code: string;
}
export interface Capture {
  id: string;
  name: string;
  revision: string;
  origin: string;
  blocks: Block[];
  text: string;
  executable: boolean;
}
export interface Tool {
  id: string;
  role: Role;
  name: string;
  version: string;
}
export interface State {
  shell: boolean;
  sqlite: boolean;
  tex: boolean;
  tools: Tool[];
  roots: { id: string; kind: Kind; name: string }[];
  databases: { id: string; name: string }[];
}
export interface Result {
  status: string;
  message: string;
  output: { stdout: string; stderr: string; code: number | null } | null;
  table: {
    columns: string[];
    rows: string[][];
    truncated: boolean;
    changes: number;
  } | null;
  pdf: boolean;
  warnings: string[];
}
export interface Request {
  capture: string;
  block: string | null;
  root: string | null;
  database: string | null;
  tool: string | null;
  pandoc: string | null;
  mode: Mode;
  toc: boolean;
}
export interface Input {
  document: string | null;
  editor: string | null;
  generation: string | null;
  revision: number | null;
  saved: boolean;
}
export const capture = (input: Input) =>
  invoke<Capture>("power_capture", { input });
export const state = () => invoke<State>("power_state");
export const enable = (kind: Kind, on: boolean) =>
  invoke<State>("power_enable", { kind, on });
export const chooseTool = (role: Role) =>
  invoke<Tool | null>("power_choose_tool", { role });
export const trust = (kind: Kind) => invoke("power_trust", { kind });
export const database = () => invoke("power_choose_database");
export const image = (capture: string, reference: string) =>
  invoke<boolean>("power_image", { capture, reference });
export const diagram = (capture: string, source: string, png: string) =>
  invoke("power_diagram", { capture, source, png });
export const prepare = (request: Request) =>
  invoke<{ token: string; summary: string }>("power_prepare", { request });
export const run = (token: string) =>
  invoke<string | null>("power_run", { token });
export const poll = (job: string) => invoke<Result>("power_poll", { job });
export const cancel = (job: string) => invoke("power_cancel", { job });
export const release = (capture: string) =>
  invoke("power_release", { capture });
export const clear = () => invoke("power_clear");
export const save = (job: string) =>
  invoke<string | null>("power_save_pdf", { job });
export const open = (exportId: string) =>
  invoke("power_open_pdf", { export: exportId });
