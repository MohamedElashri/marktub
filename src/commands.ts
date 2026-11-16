import type { Values } from "./preferences";
export type CommandId =
  | "power"
  | "review_records"
  | "review"
  | "git"
  | "open"
  | "save"
  | "save_as"
  | "preferences"
  | "palette"
  | "quick_open"
  | "search_workspaces"
  | "browser"
  | "read"
  | "source"
  | "split"
  | "visual"
  | "focus"
  | "outline"
  | "find"
  | "reload"
  | "compare"
  | "example"
  | "move_tab_left"
  | "move_tab_right"
  | "new_tab"
  | "close_tab"
  | "next_tab"
  | "previous_tab"
  | "back"
  | "forward";
export interface Command {
  id: CommandId;
  label: string;
  group: string;
  default: string;
  enabled: boolean;
  reason?: string;
  run: () => void | Promise<void>;
}
export const definitions: [CommandId, string, string, string][] = [
  ["power", "Export PDF and local jobs", "Document", "none"],
  ["review_records", "Manage stored reviews", "Review", "none"],
  ["review", "Toggle document review", "Review", "none"],
  ["git", "Git workbench", "Files", "none"],
  ["open", "Open file", "Files", "Mod+O"],
  ["save", "Save", "Files", "Mod+S"],
  ["save_as", "Save As", "Files", "Mod+Shift+S"],
  ["preferences", "Preferences", "Application", "Mod+Comma"],
  ["palette", "Command palette", "Application", "Mod+Shift+P"],
  ["quick_open", "Quick open", "Files", "Mod+P"],
  ["search_workspaces", "Search workspaces", "Files", "Mod+Shift+F"],
  ["browser", "Toggle file browser", "Files", "Mod+Alt+B"],
  ["read", "Read mode", "Document", "Mod+Shift+R"],
  ["source", "Source mode", "Document", "Mod+Shift+E"],
  ["split", "Split mode", "Document", "none"],
  ["visual", "Visual mode", "Document", "none"],
  ["focus", "Toggle document focus", "Document", "Mod+Alt+F"],
  ["outline", "Outline and headings", "Reading", "Mod+Shift+O"],
  ["find", "Find in document", "Reading", "Mod+F"],
  ["reload", "Reload document", "Document", "Mod+Alt+R"],
  ["compare", "Compare disk changes", "Document", "Mod+Shift+D"],
  ["example", "Open feature guide", "Help", "none"],
  ["move_tab_left", "Move tab left", "Tabs", "Alt+Shift+ArrowLeft"],
  ["move_tab_right", "Move tab right", "Tabs", "Alt+Shift+ArrowRight"],
  ["new_tab", "New tab", "Tabs", "Mod+T"],
  ["close_tab", "Close tab", "Tabs", "Mod+Shift+X"],
  ["next_tab", "Next tab", "Tabs", "Ctrl+Tab"],
  ["previous_tab", "Previous tab", "Tabs", "Ctrl+Shift+Tab"],
  ["back", "Back in file history", "Files", "Alt+ArrowLeft"],
  ["forward", "Forward in file history", "Files", "Alt+ArrowRight"],
];
export function binding(command: Command, values: Values) {
  return String(values[`shortcut.${command.id}`]?.value ?? command.default);
}
// Display only: canonical settings and event matching keep their literal form.
export function shortcutLabel(
  value: string,
  mac = navigator.platform.startsWith("Mac"),
) {
  if (!value || value === "none")
    return { visual: "", spoken: "No shortcut assigned" };
  const parts = value.split("+");
  const key = parts.pop() ?? "";
  const modifiers = new Set(
    parts.map((part) => (part === "Mod" ? (mac ? "Meta" : "Ctrl") : part)),
  );
  const keys: Record<string, string> = {
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backslash: "\\",
    Minus: "−",
    Equal: "=",
    ArrowLeft: "←",
    ArrowRight: "→",
    ArrowUp: "↑",
    ArrowDown: "↓",
    Space: "Space",
    Escape: "Esc",
    Enter: mac ? "↩" : "Enter",
    Tab: mac ? "⇥" : "Tab",
  };
  const names: Record<string, string> = {
    Ctrl: "Control",
    Alt: mac ? "Option" : "Alt",
    Shift: "Shift",
    Meta: mac ? "Command" : "Super",
  };
  const symbols: Record<string, string> = {
    Ctrl: mac ? "⌃" : "Ctrl",
    Alt: mac ? "⌥" : "Alt",
    Shift: mac ? "⇧" : "Shift",
    Meta: mac ? "⌘" : "Super",
  };
  const ordered = ["Ctrl", "Alt", "Shift", "Meta"].filter((part) =>
    modifiers.has(part),
  );
  return {
    visual: [...ordered.map((part) => symbols[part]), keys[key] ?? key].join(
      mac ? "" : "+",
    ),
    spoken: [
      ...ordered.map((part) => names[part]),
      key.replace(/^Arrow/, "Arrow "),
    ].join(" plus "),
  };
}
export function chord(event: KeyboardEvent) {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.getModifierState?.("AltGraph")
  )
    return "";
  let key = event.code.startsWith("Key")
    ? event.code.slice(3)
    : event.code.startsWith("Digit")
      ? event.code.slice(5)
      : event.key.length === 1
        ? event.key.toUpperCase()
        : event.key;
  if (key === ",") key = "Comma";
  return [
    event.altKey ? "Alt" : "",
    event.ctrlKey ? "Ctrl" : "",
    event.metaKey ? "Meta" : "",
    event.shiftKey ? "Shift" : "",
    key,
  ]
    .filter(Boolean)
    .join("+");
}
export function matches(
  event: KeyboardEvent,
  binding: string,
  mac = navigator.platform.startsWith("Mac"),
) {
  if (binding === "none") return false;
  const parts = binding.split("+");
  const key = parts.pop();
  const mods = parts
    .map((m) => (m === "Mod" ? (mac ? "Meta" : "Ctrl") : m))
    .sort();
  return chord(event) === [...mods, key].join("+");
}
export function score(text: string, query: string) {
  text = text.toLocaleLowerCase();
  query = query.trim().toLocaleLowerCase();
  if (!query) return 0;
  const direct = text.indexOf(query);
  if (direct >= 0) return direct;
  let at = 0;
  for (const c of query) {
    at = text.indexOf(c, at);
    if (at < 0) return Infinity;
    at++;
  }
  return 100 + at;
}
