import { msg, type Msg } from "./text.ts";

/**
 * "How to open" (Store v3, design store-v3 §6; owner ruling 2026-11-15: no
 * manifest field, a convention in `description`). A description whose last
 * line starts with `→ ` (U+2192 and a space) says there where the plugin is
 * found once installed, in at most 48 characters after the marker:
 *
 *   Saves, saves as or copies the current frame …
 *   → Menu: Tools ▸ Snapshot
 *
 * The card shows that line and the description is shown without it. The
 * arrow is not a word, so translations keep it and the line works in every
 * language the description is translated into. Anything else (no marker on
 * the last line, nothing after it, more than 48 characters) leaves the
 * description as it is and the card's line empty. A description that is only
 * the line is shown as none.
 */
export const OPEN_MARKER = "→ ";
/** Characters after the marker (code points; the card elides the line by pixel width anyway). */
export const OPEN_LIMIT = 48;

export function splitDescription(description: string | null): { description: string | null; open: string | null } {
  if (description === null) return { description, open: null };
  const text = description.trimEnd();
  const at = text.lastIndexOf("\n");
  const last = text.slice(at + 1);
  if (!last.startsWith(OPEN_MARKER)) return { description, open: null };
  const open = last.slice(OPEN_MARKER.length).trim();
  if (!open || [...open].length > OPEN_LIMIT) return { description, open: null };
  const rest = at < 0 ? "" : text.slice(0, at).trimEnd();
  return { description: rest || null, open };
}

/** One row of `plugin.commands.list()` as far as the fallback reads it. */
export interface ListedCommand {
  id: string;
  path: string[];
  /** "Category: Title" as the palette shows it. */
  label: string;
}

/**
 * The fallback for an installed plugin whose description has no line: its
 * own menu entry from the palette's table (`commands.list`, built-in plugins
 * only; the Store is one), "Menu: Tools ▸ Snapshot"; without one its palette
 * row, "Command palette: Window: Always on Top". The shortest text wins, then
 * the first by text, so the line does not change between reads; a trailing
 * "…" is dropped as the convention writes menu names. None: `null` (the line
 * stays empty).
 */
export function commandLine(commands: readonly ListedCommand[], pluginId: string): Msg | null {
  const own = commands.filter((each) => each.id.startsWith(`${pluginId}/`));
  const shortest = (texts: string[]) => texts.sort((a, b) => a.length - b.length || a.localeCompare(b))[0];
  const menu = shortest(own.filter((each) => each.path.length > 0).map((each) => each.path.join(" ▸ ").replace(/…$/, "")));
  if (menu) return msg("Menu: {}", menu);
  const palette = shortest(own.map((each) => each.label.replace(/…$/, "")).filter((label) => label !== ""));
  return palette ? msg("Command palette: {}", palette) : null;
}

/**
 * The Category drop-down (design §9 question 3, owner ruling 2026-11-15): a
 * fixed list of goals, each matched by a plugin's tags (English or
 * translated, any case). A plugin can be in more than one; one that no goal
 * matches is in Other. Index 0 of the drop-down is "Category: All".
 */
export const GOALS = [
  {
    name: "Picture",
    tags: ["video", "picture", "image", "color", "colour", "filter", "filters", "shader", "shaders", "upscaling", "upscale", "hdr", "motion", "interpolation"],
  },
  { name: "Sound", tags: ["audio", "sound", "music", "equalizer", "volume", "audio track", "output device", "loudness"] },
  { name: "Subtitles", tags: ["subtitle", "subtitles", "caption", "captions", "lyrics"] },
  { name: "Online video", tags: ["online", "online video", "streaming", "stream", "youtube", "bilibili", "web", "iptv"] },
  { name: "Library and playlists", tags: ["playlist", "playlists", "library", "history", "bookmarks", "chapters", "resume", "media library"] },
  {
    name: "Tools",
    tags: ["tools", "tool", "snapshot", "screenshot", "capture", "recording", "info", "media info", "settings", "shortcuts", "hotkeys", "plugins", "store", "ai", "plugin development", "window"],
  },
  { name: "Games", tags: ["game", "games", "emulator", "retro"] },
] as const;

export type GoalName = (typeof GOALS)[number]["name"] | "Other";
/** The drop-down's entries after "Category: All", in order. */
export const GOAL_NAMES: readonly GoalName[] = [...GOALS.map((goal) => goal.name), "Other"];

/** The goals a plugin's tags put it in; none matched: Other. */
export function goalsOf(tags: readonly string[]): GoalName[] {
  const lower = new Set(tags.map((tag) => tag.toLowerCase()));
  const found: GoalName[] = GOALS.filter((goal) => goal.tags.some((tag) => lower.has(tag))).map((goal) => goal.name);
  return found.length ? found : ["Other"];
}
