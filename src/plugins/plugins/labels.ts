import type { PluginInfo } from "@vestra-player/sdk";

/**
 * The page's words live in ui/plugins.slint (`@tr`); TypeScript hands it
 * codes and data. These helpers pick the codes.
 */

/**
 * The list's narrow source column, as a code the page turns into a word:
 * a `PackageSource` kind (`repository`, `link`, `file`, `workshop`,
 * `bundled`; `bundled` for every built-in plugin), else the origin
 * (`installed` from an older player without sources, `development`).
 */
export function sourceCode(info: Pick<PluginInfo, "origin" | "builtin" | "source">): string {
  if (info.builtin) return "bundled";
  return info.source?.kind ?? info.origin;
}

/** The address shown after a repository or link source (detail 「Source」); "" otherwise. */
export function sourceUrl(info: Pick<PluginInfo, "source">): string {
  const source = info.source;
  return source && "url" in source ? source.url : "";
}

/** `12 KB`, `3.4 MB`; `—` when unknown. Units are symbols, not translated. */
export function bytes(size: number | null): string {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * A refused package call, as the code the page words (`self`,
 * `development`, `builtin`, `missing`, `busy`) or `error` with the
 * player's own message.
 */
export function failure(error: unknown, info?: PluginInfo): { code: string; text: string } {
  const code = (error as { code?: string }).code;
  if (code === "invalid") return { code: "self", text: "" };
  if (code === "unsupported") {
    if (info?.origin === "development") return { code: "development", text: "" };
    // An installed plugin refused: built in (the list may not have said so yet).
    return { code: "builtin", text: "" };
  }
  if (code === "missing" || code === "busy") return { code, text: "" };
  return { code: "error", text: (error as Error).message || String(error) };
}
