import type { PackageSource } from "@vestra-player/sdk";
import { IndexError } from "./index-format.ts";
import { NetError } from "./net.ts";
import { msg, raw, type Msg } from "./text.ts";

/**
 * `vestra` and `vestra.<name>` (any case) belong to the player's built-in
 * plugins (ecosystem ruling 2026-10-24): the player refuses to install them
 * (`reserved`), whichever repository lists them.
 */
export function reservedId(id: string): boolean {
  const lower = id.toLowerCase();
  return lower === "vestra" || lower.startsWith("vestra.");
}

/** The player's own wording for `reserved`. */
export const reservedText = (id: string | undefined): Msg =>
  id === undefined
    ? msg(
        "This plugin: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed or loaded for testing",
      )
    : msg(
        "“{}”: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed or loaded for testing",
        id,
      );

/** Where a sideloaded or other-source plugin came from. */
export function sourceName(source: PackageSource | null): Msg {
  switch (source?.kind) {
    case "bundled":
      return msg("Built-in");
    case "file":
      return msg("Local file");
    case "link":
      return msg("Link");
    case "workshop":
      return msg("Plugin Workshop");
    case "repository":
      return msg("Repository {}", hostOf(source.url));
    default:
      return msg("Unknown source");
  }
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `10:21` today, `10-21 18:02` otherwise (local time). */
export function clock(time: number, now: number): string {
  const at = new Date(time);
  const today = new Date(now);
  const hm = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  return at.toDateString() === today.toDateString() ? hm : `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${hm}`;
}

/** `2026-10-21` from an ISO date. */
export function day(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/** Why something failed, for a sentence: the store's own reasons, else the error's message. */
export function reasonOf(error: unknown): Msg {
  if (error instanceof NetError || error instanceof IndexError) return error.reason;
  return raw((error as Error).message);
}
