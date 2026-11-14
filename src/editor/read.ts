import type { Text } from "@codemirror/state";
import type { Read } from "../reader/model";
export function readEditor(doc: Text): Read {
  const encoder = new TextEncoder();
  const checkpoints = new Map<number, number>([[0, 0]]);
  let total = 0;
  for (const chunk of doc.iter()) total += encoder.encode(chunk).length;
  return async (start) => {
    let byte = 0,
      position = 0;
    for (const [offset, utf16] of checkpoints)
      if (offset <= start && offset >= byte) {
        byte = offset;
        position = utf16;
      }
    while (byte < start) {
      const cp = doc.sliceString(position, position + 2).codePointAt(0);
      if (cp === undefined) break;
      const character = String.fromCodePoint(cp);
      byte += encoder.encode(character).length;
      position += character.length;
    }
    let end = Math.min(doc.length, position + 16000);
    if (
      end < doc.length &&
      /[\uD800-\uDBFF]/.test(doc.sliceString(end - 1, end))
    )
      end--;
    const text = doc.sliceString(position, end);
    const next = byte + encoder.encode(text).length;
    checkpoints.set(next, end);
    return { start: byte, end: next, text, total };
  };
}
