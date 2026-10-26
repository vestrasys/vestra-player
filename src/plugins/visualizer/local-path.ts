import { posix, win32 } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Which path rules apply. The plugin uses the rules of the OS it runs on;
 * tests pass the other one to check both on any machine.
 */
export type PathRules = "windows" | "posix";
export const HOST_RULES: PathRules = process.platform === "win32" ? "windows" : "posix";

const pathOf = (rules: PathRules) => (rules === "windows" ? win32 : posix);

/**
 * An absolute local path under `rules`: `/x`; on Windows also `C:\x`, `C:/x`,
 * `\\server\share\x` and `\x`. Checked before the link test, so `C://x` is a
 * path on Windows, not a link with the scheme `c`.
 */
export const isLocalPath = (source: string, rules = HOST_RULES) => pathOf(rules).isAbsolute(source);

/** A link (`scheme://…`), not a local path. `file:` URLs are links too; see {@link fileUrlPath}. */
export const isUrl = (source: string, rules = HOST_RULES) => !isLocalPath(source, rules) && /^[a-z][a-z0-9+.-]*:\/\//i.test(source);

/**
 * The local path a `file:` URL names under `rules`, or `null` (not a `file:`
 * URL, or one that names no local file here: `file:///x` has no drive on
 * Windows, and a host other than `localhost` is refused on POSIX).
 */
export function fileUrlPath(source: string, rules = HOST_RULES): string | null {
  if (!/^file:\/\//i.test(source)) return null;
  try {
    return fileURLToPath(source, { windows: rules === "windows" });
  } catch {
    return null;
  }
}

/** The local file `source` names: a path as it is, a `file:` URL converted, `null` for any other link. */
export function localFile(source: string, rules = HOST_RULES): string | null {
  if (!isUrl(source, rules)) return source;
  return fileUrlPath(source, rules);
}

/** The last segment of a path under `rules` (`\` and `/` both separate on Windows); the path itself when it has none. */
export const baseName = (path: string, rules = HOST_RULES) => pathOf(rules).basename(path) || path;
