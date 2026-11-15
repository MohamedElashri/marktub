import { join, resolve } from "node:path";

export const macosTarget = "universal-apple-darwin";
export const desktopTargetArgs =
  process.platform === "darwin" ? ["--target", macosTarget] : [];
export function desktopBinary(
  profile,
  platform = process.platform,
  target = process.env.CARGO_TARGET_DIR ?? "src-tauri/target",
) {
  return resolve(
    platform === "darwin"
      ? join(
          target,
          macosTarget,
          profile,
          "bundle/macos/Marktub.app/Contents/MacOS/marktub",
        )
      : join(target, profile, "marktub"),
  );
}
