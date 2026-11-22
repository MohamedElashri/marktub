import { join, resolve } from "node:path";

export const macosTarget = "universal-apple-darwin";
export const desktopTargetArgs =
  process.platform === "darwin" ? ["--target", macosTarget] : [];
export const macosTestArchitecture =
  process.arch === "arm64" ? "arm64" : "x86_64";
const macosTestTarget =
  macosTestArchitecture === "arm64"
    ? "aarch64-apple-darwin"
    : "x86_64-apple-darwin";
export const desktopTestTargetArgs =
  process.platform === "darwin" ? ["--target", macosTestTarget] : [];
export function desktopBinary(
  profile,
  platform = process.platform,
  target = process.env.CARGO_TARGET_DIR ?? "src-tauri/target",
) {
  return resolve(
    platform === "darwin"
      ? join(
          target,
          profile === "debug" ? macosTestTarget : macosTarget,
          profile,
          "bundle/macos/Marktub.app/Contents/MacOS/marktub",
        )
      : join(target, profile, "marktub"),
  );
}
