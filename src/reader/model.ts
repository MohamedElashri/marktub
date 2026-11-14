export interface Section {
  start: number;
  end: number;
  fallback?: string;
  note?: string;
}
export interface Heading {
  title: string;
  id: string;
  section: number;
  level: number;
}
export interface Context {
  references: Record<string, { href: string; title: string }>;
  notes: Record<string, string>;
}
export interface Index {
  sections: Section[];
  headings: Heading[];
  context: Context;
  complete: boolean;
  scanned: number;
  total: number;
}
export interface Rich {
  kind: "math" | "mermaid";
  source: string;
  display: boolean;
}
export interface Page {
  html: string;
  rich: Rich[];
  provisional: boolean;
  fallback?: string;
}
export type Read = (
  start: number,
) => Promise<{ start: number; end: number; text: string; total: number }>;
