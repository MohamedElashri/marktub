export type Insert =
  { kind: "text"; text: string } | { kind: "restore"; token: string };
export interface Change {
  from: number;
  to: number;
  insert: Insert;
}
export interface Batch {
  schema: 1;
  generation: string;
  operationId: string;
  baseRevision: number;
  newRevision: number;
  changes: Change[];
}
export interface Ack {
  generation: string;
  appliedRevision: number;
  receivedRevision: number;
  durableRevision: number;
  inverse: Change[];
}
export interface EditState {
  receivedRevision: number;
  durableRevision: number;
  savedRevision: number;
  dirty: boolean;
}
export interface EditWindow {
  from: number;
  to: number;
  total: number;
}
export interface EditDescriptor {
  range?: EditWindow | null;
  id: string;
  generation: string;
  journalGeneration: string;
  text: string;
  state: EditState;
  name: string;
  caret: number;
  needsSaveAs: boolean;
}
export interface Applied {
  acknowledgements: Ack[];
  state: EditState;
}
export interface Saved {
  revision: number;
  noop: boolean;
  rescue: boolean;
  name: string;
  state: EditState;
}
export interface Recovery {
  id: string;
  name: string;
  revision: number;
  updated: number;
  damaged: boolean;
  clean: boolean;
}
export interface Comparison {
  base: string;
  disk: string;
  clipped: boolean;
  missing: boolean;
}
export interface Transport {
  apply(batches: Batch[], retained: string[]): Promise<Applied>;
  flush(): Promise<EditState>;
}
