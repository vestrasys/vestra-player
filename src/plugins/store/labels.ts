import type { PackageSource } from "@vestra-player/sdk";
import { IndexError } from "./index-format.ts";
import { NetError } from "./net.ts";
import { list, msg, raw, type Arg, type Msg } from "./text.ts";

/** Capabilities in plain words (design §2.4); `risky` ones are drawn amber. */
export const CAPABILITIES = {
  commands: { text: "Add menu commands and shortcuts" },
  "ui.window": { text: "Open tool windows" },
  "ui.mount": { text: "Show panels in the bottom bar, the sidebar or behind the video" },
  "ui.dialog": { text: "Show file pickers and confirmations" },
  "ui.osd": { text: "Show messages on the video" },
  "ui.drop": { text: "Receive files dropped on the player" },
  "ui.theme": { text: "Change the player's palette" },
  "ui.settings": { text: "Add pages to the Options window" },
  "playback.read": { text: "Read what is playing and the position" },
  events: { text: "Receive playback changes as they happen" },
  "playback.control": { text: "Open media, control playback, keep the computer awake" },
  "producer.av": { text: "Provide audio and video it generates itself" },
  "processing.video": { text: "Process the picture (filters, shaders)" },
  "processing.audio": { text: "Process the sound" },
  "subtitles.read": { text: "Read subtitles" },
  "subtitles.control": { text: "Load and change subtitles" },
  "media.read": { text: "Take snapshots and export clips" },
  "media.control": { text: "Take snapshots and export clips" },
  "source.resolve": { text: "Resolve online video addresses" },
  "timeline.marks": { text: "Show marks on the progress bar" },
  "window.control": { text: "Adjust the main window (size, on top, picture-in-picture)" },
  clipboard: { text: "Read and write the clipboard", risky: true },
  "bindings.manage": { text: "Change all shortcuts", risky: true },
  secrets: { text: "Store sign-in details in the system keychain", risky: true },
  "plugins.manage": { text: "Install, uninstall and disable other plugins", risky: true },
  "rendering.manage": { text: "Choose GPU or software rendering and restart the player", risky: true },
  "video.upscaling": { text: "Upscale the video with the GPU's video upscaler" },
} as const satisfies Record<string, { text: string; risky?: true }>;

/** The words of a capability (a template of text.ts). */
export type CapabilityText = (typeof CAPABILITIES)[keyof typeof CAPABILITIES]["text"];

const KNOWN: Partial<Record<string, { text: CapabilityText; risky?: true }>> = CAPABILITIES;

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

/**
 * One line per distinct description (media.read and media.control share
 * one): the capability shown (`id`, the first of those sharing a text), its
 * words and whether it is risky.
 */
export function permissions(capabilities: readonly string[]): { id: string; text: Msg; risky: boolean }[] {
  const seen = new Set<string>();
  const lines: { id: string; text: Msg; risky: boolean }[] = [];
  for (const name of capabilities) {
    const known = KNOWN[name];
    const key = known?.text ?? `?${name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    lines.push({
      id: name,
      text: known ? msg(known.text) : msg("Unknown permission: {}", name),
      risky: known?.risky === true,
    });
  }
  return lines;
}

/** Bullet list for a confirmation, at most 20 lines. */
export function permissionList(capabilities: readonly string[]): Arg {
  const lines = permissions(capabilities);
  if (!lines.length) return msg("· Uses none of the player's features");
  const shown: Arg[] = lines.slice(0, 20).map((line) => msg("· {}", line.text));
  if (lines.length > 20) shown.push(msg("{} in total", lines.length));
  return list(shown, "lines");
}

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
