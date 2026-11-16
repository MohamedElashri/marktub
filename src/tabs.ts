// Session records contain real documents only. Keep selection tied to the
// visible tab even when New tab or recovery tabs appear before it.
export function sessionSelection(documents: (string | null)[], active: number) {
  const opened = documents.flatMap((id, index) =>
    id && id !== "recovery" ? [{ id, index }] : [],
  );
  return {
    ids: opened.map((tab) => tab.id),
    active: Math.max(0, opened.filter((tab) => tab.index <= active).length - 1),
  };
}

export function moveTab<T extends { key: number }>(
  tabs: T[],
  active: number,
  key: number,
  destination: number,
) {
  const from = tabs.findIndex((tab) => tab.key === key);
  if (from < 0 || !Number.isInteger(destination)) return { tabs, active };
  const to = Math.max(0, Math.min(destination, tabs.length - 1));
  if (from === to) return { tabs, active };
  const selected = tabs[active]?.key;
  const reordered = [...tabs];
  const [moving] = reordered.splice(from, 1);
  reordered.splice(to, 0, moving);
  return {
    tabs: reordered,
    active: Math.max(
      0,
      reordered.findIndex((tab) => tab.key === selected),
    ),
  };
}
