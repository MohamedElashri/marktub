export const librarySize = { default: 280, min: 220, max: 600 };

export function libraryWidth(
  width: number,
  viewport = Infinity,
  minimumDocumentWidth = 360,
) {
  const maximum = Math.max(
    librarySize.min,
    Math.min(librarySize.max, viewport - minimumDocumentWidth),
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

export const folderRailWidth = 160;

export function showNoteColumns(viewport: number, mode: string | undefined) {
  return viewport >= 1000 && mode !== "split";
}
