// Plain source spans; untrusted source is never inserted as HTML by this adapter.
export function intraline(left: string, right: string) {
  const a = Array.from(left),
    b = Array.from(right);
  let from = 0,
    end = 0;
  while (from < Math.min(a.length, b.length) && a[from] === b[from]) from++;
  while (
    end < Math.min(a.length, b.length) - from &&
    a[a.length - 1 - end] === b[b.length - 1 - end]
  )
    end++;
  return {
    left: [
      a.slice(0, from).join(""),
      a.slice(from, a.length - end).join(""),
      a.slice(a.length - end).join(""),
    ],
    right: [
      b.slice(0, from).join(""),
      b.slice(from, b.length - end).join(""),
      b.slice(b.length - end).join(""),
    ],
  };
}
export function windowStart(scroll: number, height = 24) {
  return Math.max(0, Math.floor(Math.max(0, scroll / height - 8) / 100) * 100);
}
