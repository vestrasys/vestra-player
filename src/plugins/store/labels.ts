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
  "media.sources": { text: "Remember videos you add and export from them later" },
  "source.resolve": { text: "Resolve online video addresses" },
  "timeline.marks": { text: "Show marks on the progress bar" },
  "window.control": { text: "Adjust the main window (size, on top, picture-in-picture)" },
  clipboard: { text: "Read and write the clipboard", risky: true },
  "bindings.manage": { text: "Change all shortcuts", risky: true },
  secrets: { text: "Store sign-in details in the system keychain", risky: true },
  "plugins.manage": { text: "Install, uninstall and disable other plugins", risky: true },
  "rendering.manage": { text: "Choose GPU or software rendering and restart the player", risky: true },
  "video.upscaling": { text: "Upscale the video with the GPU's video upscaler" },
  "video.motion": { text: "Smooth motion with the GPU's frame-rate converter" },
  "commands.run": { text: "Run any player or plugin command", risky: true },
  "palette.lead": { text: "Add the first row of the command palette" },
  mpv: { text: "Control the playback engine (mpv) directly: send its commands and change its options", risky: true },
} as const satisfies Record<string, { text: string; risky?: true }>;

/**
 * The categories of the SDK's capability scan (`vestra-plugin scan`, its
 * `uses`) in plain words, in the scan's order. A repository's index stores
 * them per version (`scan.uses`); the confirmations then show them instead
 * of the declared capabilities.
 */
export const CATEGORIES = {
  player: "Change your player settings (shortcuts, theme, language, rendering) or control the playback engine directly",
  plugins: "Install, uninstall, disable or reload other plugins, or run any command",
  clipboard: "Read and write the clipboard",
  credentials: "Store sign-in details in the system keychain",
  camera: "Use the camera",
  microphone: "Record from the microphone",
  screen: "Capture the screen or other windows",
  notifications: "Show system notifications",
} as const satisfies Record<string, string>;

/** The words of a capability or of a scanned category (a template of text.ts). */
export type CapabilityText =
  | (typeof CAPABILITIES)[keyof typeof CAPABILITIES]["text"]
  | (typeof CATEGORIES)[keyof typeof CATEGORIES];

const KNOWN: Partial<Record<string, { text: CapabilityText; risky?: true }>> = CAPABILITIES;
const CATEGORY: Partial<Record<string, CapabilityText>> = CATEGORIES;

/** The declared capabilities a confirmation names: the risky ones, and any the store does not know. */
export const riskyOnly = (capabilities: readonly string[]): string[] =>
  capabilities.filter((name) => KNOWN[name]?.risky === true || !Object.hasOwn(CAPABILITIES, name));

/**
 * Scanned categories in plain words: each once, in the scan's order; a word
 * the store does not know is named, after the others.
 */
export function categories(uses: readonly string[]): Msg[] {
  const given = new Set(uses);
  return [
    ...Object.keys(CATEGORIES)
      .filter((word) => given.has(word))
      .map((word) => msg(CATEGORY[word]!)),
    ...[...given].filter((word) => !Object.hasOwn(CATEGORIES, word)).map((word) => msg("Unknown permission: {}", word)),
  ];
}

/**
 * What a newer version adds to the installed one, in plain words (empty:
 * nothing): by the scanned categories when the index has them for both the
 * installed version and the new one, else by the declared capabilities. The
 * update confirmation and the window's notice both go by this.
 */
export function addedBy(
  versions: readonly { version: string; scan: { uses: string[] } | null }[],
  installed: { version: string | null; capabilities: readonly string[] },
  next: { capabilities: readonly string[]; scan: { uses: string[] } | null },
): Msg[] {
  const before = versions.find((v) => v.version === installed.version)?.scan ?? null;
  if (before && next.scan) return categories(next.scan.uses.filter((word) => !before.uses.includes(word)));
  return permissions(next.capabilities.filter((c) => !installed.capabilities.includes(c))).map((line) => line.text);
}

/** `lines` as a bullet list for a confirmation, at most 20 lines. */
export function bulletList(lines: readonly Msg[]): Arg {
  const shown: Arg[] = lines.slice(0, 20).map((line) => msg("· {}", line));
  if (lines.length > 20) shown.push(msg("{} in total", lines.length));
  return list(shown, "lines");
}

/** Bullet list of scanned categories for a confirmation; `uses` is not empty. */
export const categoryList = (uses: readonly string[]): Arg =>
  list(
    categories(uses).map((line) => msg("· {}", line)),
    "lines",
  );

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
