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
