import {
  readdirSync,
  lstatSync,
  readSync,
  realpathSync,
  openSync,
  closeSync,
  fstatSync,
  constants,
} from "node:fs";
import { resolve, relative, isAbsolute, sep } from "node:path";
import { createHash } from "node:crypto";

const noticeName =
  /^(?:licen[sc]e|copying|notice|copyright|authors)(?:[.\-_].*)?$/i;

const sourceExtension =
  /\.(?:rs|c|cc|cpp|h|hpp|m|mm|js|cjs|mjs|ts|tsx|jsx|py|rb|go|swift)$/i;
const isNotice = (name) => noticeName.test(name) && !sourceExtension.test(name);

// Registry archives can carry separately licensed vendored code below the root.
// Never follow their links or a declared license path outside the package.
export function licenseFiles(directory, declared, limits = {}) {
  const root = realpathSync(directory);
  const { entries = 25000, depth = 16, files = 256 } = limits;
  const candidates = new Set();
  let visited = 0;
  function walk(folder, level) {
    if (realpathSync(folder) !== folder)
      throw Error("Package notice directory changed or is a symlink.");
    if (level > depth) throw Error("Package notice scan exceeds depth budget.");
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      if (++visited > entries)
        throw Error("Package notice scan exceeds entry budget.");
      const path = resolve(folder, entry.name);
      if (entry.isSymbolicLink()) {
        if (isNotice(entry.name)) throw Error("Package notice is a symlink.");
        continue;
      }
      if (entry.isDirectory()) {
        if (entry.name !== "node_modules" && entry.name !== ".git")
          walk(path, level + 1);
      } else if (entry.isFile() && isNotice(entry.name)) candidates.add(path);
    }
  }
  walk(root, 0);
  if (declared) {
    const path = resolve(root, declared),
      local = relative(root, path);
    if (
      !local ||
      local === ".." ||
      local.startsWith(".." + sep) ||
      isAbsolute(local)
    )
      throw Error("Declared license escapes its package.");
    if (realpathSync(path) !== path || !lstatSync(path).isFile())
      throw Error("Declared license must be an ordinary package file.");
    candidates.add(path);
  }
  if (candidates.size > files)
    throw Error("Package notice scan exceeds file budget.");
  return [...candidates]
    .sort()
    .map((path) => {
      if (realpathSync(path) !== path)
        throw Error("Package notice changed or is a symlink.");
      const fd = openSync(
        path,
        constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
      );
      try {
        const before = fstatSync(fd, { bigint: true });
        if (!before.isFile() || before.size > 1024n * 1024n)
          throw Error("Package notice must be bounded UTF-8 text.");
        const buffer = Buffer.alloc(Number(before.size) + 1);
        let length = 0,
          n;
        while (
          length < buffer.length &&
          (n = readSync(fd, buffer, length, buffer.length - length, null))
        )
          length += n;
        const bytes = buffer.subarray(0, length),
          text = bytes.toString("utf8"),
          after = fstatSync(fd, { bigint: true });
        if (
          BigInt(length) !== before.size ||
          before.size !== after.size ||
          before.mtimeNs !== after.mtimeNs ||
          before.ctimeNs !== after.ctimeNs
        )
          throw Error("Package notice changed during capture.");
        if (bytes.includes(0) || !Buffer.from(text).equals(bytes))
          throw Error("Package notice must be bounded UTF-8 text.");
        return {
          path: relative(root, path).split(sep).join("/"),
          text,
          sha256: createHash("sha256").update(bytes).digest("hex"),
        };
      } finally {
        closeSync(fd);
      }
    })
    .filter((notice) => notice.text.trim());
}

export function renderNotices(records) {
  const groups = new Map();
  for (const { component, text } of records) {
    const hash = createHash("sha256").update(text).digest("hex");
    if (!groups.has(hash)) groups.set(hash, { text, components: new Set() });
    groups.get(hash).components.add(component);
  }
  return (
    "Dependency attribution. Identical notice texts are retained once for the listed components.\n\n" +
    [...groups.values()]
      .map(
        ({ text, components }) =>
          [...components].sort().join("\n\n") + "\n\n" + text,
      )
      .join(
        "\n\n------------------------------------------------------------\n\n",
      )
  );
}
