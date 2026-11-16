export const librarySize = { default: 280, min: 220, max: 600 };

export function libraryWidth(width: number, viewport = Infinity) {
  const maximum = Math.max(
    librarySize.min,
    Math.min(librarySize.max, viewport - 360),
  );
  return Math.round(
    Math.min(
      maximum,
      Math.max(
        librarySize.min,
        Number.isFinite(width) ? width : librarySize.default,
      ),
    ),
  );
}
