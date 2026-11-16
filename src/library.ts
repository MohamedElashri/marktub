import { invoke } from "@tauri-apps/api/core";
import type { Descriptor } from "./bridge";
export interface Folder {
  id: string;
  name: string;
  root: string;
  label?: string;
}
export interface Item {
  id: string;
  name: string;
  kind: string;
  bytes: number;
  modified: number;
  directory: string | null;
  parent: Folder;
}
export interface Page {
  items: Item[];
  total: number;
  scanned: number;
  complete: boolean;
  revision: number;
}
export interface Hint {
  id: string;
  name: string;
  context: string;
  pinned: boolean;
  authorized: boolean;
}
export interface LibraryState {
  roots: Folder[];
  rootHints: Hint[];
  recent: Hint[];
  sidebar: boolean;
  view: string;
  warning: string | null;
  session: Hint[];
  active: number;
  display: Display;
  job: string | null;
  lastOperation: Progress["outcomes"];
}
export interface Review {
  id: string;
  kind: string;
  sources: string[];
  destination: string;
  warning: string;
}
export interface Progress {
  id: string;
  running: boolean;
  scanned: number;
  skipped: number;
  bytes: number;
  truncated: boolean;
  canceled: boolean;
  results: { item: Item; context: string }[];
  selection: Item[];
  outcomes: { name: string; status: string; message: string }[];
  error: string | null;
}
export interface Operation {
  kind: string;
  entries: string[];
  destination: string | null;
  name: string | null;
}
export const state = () => invoke<LibraryState>("library_state");
export const chooseRoot = () => invoke<Folder | null>("library_choose_root");
export const page = (
  id: string,
  offset: number,
  sort: string,
  hidden: boolean,
  filter: string,
  refresh: boolean,
  ignored: boolean,
) =>
  invoke<Page>("library_page", {
    id,
    offset,
    sort,
    hidden,
    filter,
    refresh,
    ignored,
  });
export const open = (id: string, recent = false) =>
  invoke<Descriptor>("library_open", { id, recent });
export const preview = (id: string) =>
  invoke<string>("library_preview", { id });
export const scan = (
  query: string,
  content: boolean,
  hidden: boolean,
  ignored: boolean,
) => invoke<string>("library_scan", { query, content, hidden, ignored });
export const job = (id: string, cancel = false) =>
  invoke<Progress>("library_job", { id, cancel });
export const prepare = (request: Operation) =>
  invoke<Review>("library_prepare", { request });
export const commit = (id: string) => invoke<string>("library_commit", { id });
export const configure = (sidebar: boolean, view: string) =>
  invoke<void>("library_configure", { sidebar, view });
export const pin = (id: string) => invoke<void>("library_pin", { id });
export const removeRoot = (id: string) =>
  invoke<void>("library_remove_root", { id });
export interface DropOffer {
  id: string;
  name: string;
  kind: string;
}
export const acceptDrop = (id: string, action: "open" | "folder") =>
  invoke<{ document: Descriptor | null; folder: Folder | null }>(
    "library_accept_drop",
    { id, action },
  );

export const session = (ids: string[], active: number) =>
  invoke<void>("library_session", { ids, active });

export interface Display {
  offset: number;
  sort: string;
  hidden: boolean;
  ignored: boolean;
  filter: string;
}
export const location = (id: string, display: Display) =>
  invoke<void>("library_location", { id, display });

export const gitBadges = (entries: string[]) =>
  invoke<Record<string, string>>("git_badges", { entries });

export interface NoteCapture {
  item: Item;
  path: string;
  text: string | null;
}
export interface NoteCatalogue {
  root: Folder;
  active: string;
  notes: NoteCapture[];
  complete: boolean;
  skipped: number;
}
export const noteCatalogue = (id: string) =>
  invoke<NoteCatalogue>("note_catalogue", { id });
