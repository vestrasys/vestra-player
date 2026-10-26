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
 * A source the player reports is a local file, not a link: an absolute path
 * under `rules` (`/x`; on Windows also `C:\x`, `C:/x`, `\\server\share\x`, `\x`).
 */
export const isLocalPath = (source: string, rules = HOST_RULES) => pathOf(rules).isAbsolute(source);

export const fileName = (source: string, rules = HOST_RULES) => pathOf(rules).basename(source);
export const folderOf = (source: string, rules = HOST_RULES) => pathOf(rules).dirname(source);

/**
 * A typed local path that names one file wherever the player runs, or `null`.
 * POSIX: starts with `/`, kept as typed. Windows: a drive path (`C:\x`, `C:/x`)
 * or a UNC path (`\\server\share\x`, `//server/share/x`), with its separators
 * made `\`; `/x` and `\x` are refused because their drive is the player's
 * current one, which the user cannot see, and `C:x` is relative.
 */
export function typedPath(text: string, rules = HOST_RULES): string | null {
  if (rules === "posix") return text.startsWith("/") ? text : null;
  const { root } = win32.parse(text);
  return win32.isAbsolute(text) && root.length > 1 ? win32.normalize(text) : null;
}

/**
 * The path a `file:` URL names under `rules`, or `null`. POSIX: `file:///x` is
 * `/x`; a host other than `localhost` is refused. Windows: `file:///C:/x` is
 * `C:\x` and `file://server/share/x` is `\\server\share\x`; `file:///x` has no
 * drive and is refused.
 */
export function fileUrlPath(url: URL, rules = HOST_RULES): string | null {
  try {
    return fileURLToPath(url, { windows: rules === "windows" });
  } catch {
    return null;
  }
}
