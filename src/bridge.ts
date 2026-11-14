import { invoke } from "@tauri-apps/api/core";

export interface Descriptor {
  id: string;
  name: string;
  byteLength: number;
  revision: string;
  watching: boolean;
  location?: string;
}

export interface SourceRange {
  start: number;
  end: number;
  total: number;
  text: string;
}

export const takeOsOpen = () =>
  invoke<{ document: Descriptor | null; error: string | null }[]>(
    "take_os_open",
  );

export const chooseDocument = () =>
  invoke<Descriptor | null>("choose_document");
export const openExample = () => invoke<Descriptor>("open_example");
export const readRange = (id: string, start = 0) =>
  invoke<SourceRange>("read_range", { id, start, limit: 64 * 1024 });
export const releaseDocument = (id: string) =>
  invoke<void>("release_document", { id });
export const documentStatus = (id: string) =>
  invoke<"unchanged" | "changed" | "missing">("document_status", { id });

export function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return typeof error === "string"
    ? error
    : "The operation failed. Try opening the file again.";
}

import type {
  Applied,
  Batch,
  EditDescriptor,
  EditState,
  Saved,
  Recovery,
  Comparison,
} from "./document/protocol";
export const beginEdit = (id: string, start?: number) =>
  invoke<EditDescriptor>("begin_edit", { id, start: start ?? null });
export const applyEdits = (
  id: string,
  generation: string,
  batches: Batch[],
  retained: string[],
) => invoke<Applied>("apply_edits", { id, generation, batches, retained });
export const flushEdits = (id: string, generation: string) =>
  invoke<EditState>("flush_edits", { id, generation });
export const saveDocument = (
  id: string,
  generation: string,
  revision: number,
) => invoke<Saved>("save_document", { id, generation, revision });
export const saveAsDocument = (
  id: string,
  generation: string,
  revision: number,
  pending: Batch[],
) =>
  invoke<{
    saved: Saved;
    document: Descriptor | null;
    refreshWarning: string | null;
  } | null>("save_as_document", { id, generation, revision, pending });
export const editingStatus = (id: string, generation: string) =>
  invoke<"unchanged" | "changed" | "missing">("editing_status", {
    id,
    generation,
  });
export const compareDocument = (id: string, generation: string) =>
  invoke<Comparison>("compare_document", { id, generation });
export const reopenDocument = (id: string) =>
  invoke<Descriptor>("reopen_document", { id });
export const releaseEditor = (
  id: string,
  generation: string,
  discard: boolean,
) => invoke<void>("release_editor", { id, generation, discard });
export const listRecoveries = (includeSaved = false) =>
  invoke<Recovery[]>("list_recoveries", { includeSaved });
export const restoreRecovery = (id: string) =>
  invoke<EditDescriptor>("restore_recovery", { id });
export const discardRecovery = (id: string) =>
  invoke<void>("discard_recovery", { id });
export type SettingValue = {
  type: "number" | "boolean" | "text" | "enum";
  value: string | number | boolean;
};
export interface SettingsResult {
  text: string;
  resolved: Record<string, SettingValue>;
  diagnostics: { line: number; message: string }[];
}
export const getSettings = (id: string | null, scope: string) =>
  invoke<SettingsResult>("get_settings", { id, scope });
export const updateSettings = (
  id: string | null,
  scope: string,
  text: string,
) => invoke<SettingsResult>("update_settings", { id, scope, text });
export const importSettings = () => invoke<string | null>("import_settings");
export const exportSettings = (text: string) =>
  invoke<boolean>("export_settings", { text });
export const documentPosition = (id: string) =>
  invoke<{ byte: number; caret: number; anchor: string | null }>(
    "document_position",
    { id },
  );
export const rememberPosition = (
  id: string,
  byte: number,
  caret: number,
  anchor: string | null = null,
) =>
  invoke<void>("remember_position", { id, position: { byte, caret, anchor } });
export const storageWarning = () => invoke<string | null>("storage_warning");
export const closeApplication = () => invoke<void>("close_application");

export const resetSettings = () => invoke<void>("reset_settings");

export const openDocumentLink = (id: string, link: string) =>
  invoke<Descriptor | null>("open_document_link", { id, link });
export const chooseImage = (id: string, link: string) =>
  invoke<{ dataUrl: string; width: number; height: number } | null>(
    "choose_image",
    { id, link },
  );
export const rememberDocument = (id: string) =>
  invoke<void>("library_remember_document", { id });

export const changeEditWindow = (
  id: string,
  generation: string,
  revision: number,
  start: number,
  fromByte = false,
) =>
  invoke<EditDescriptor>("change_edit_window", {
    id,
    generation,
    revision,
    start,
    fromByte,
  });

export const importAuthorImage = (
  id: string,
  copy: boolean,
  offer?: string,
  bytes?: number[],
) =>
  invoke<{
    path: string;
    dataUrl: string;
    width?: number;
    height?: number;
  } | null>("import_author_image", {
    id,
    copy,
    offer: offer ?? null,
    bytes: bytes ?? null,
  });

export const captureEditRead = (
  id: string,
  generation: string,
  revision: number,
) => invoke<string>("capture_edit_read", { id, generation, revision });
export const readEditSnapshot = (
  id: string,
  generation: string,
  token: string,
  start: number,
) =>
  invoke<SourceRange>("read_edit_snapshot", { id, generation, token, start });
