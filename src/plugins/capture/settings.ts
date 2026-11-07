import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Scope } from "@vestra-player/sdk";
import { encodeOf, lastSeconds, METHODS } from "./clip.ts";
import type { Encode, Method } from "./clip.ts";
import { FORMATS, quality } from "./model.ts";
import type { Format } from "./model.ts";
import { normalTemplate } from "./names.ts";
import type { TemplateKind } from "./names.ts";

/** The burst row's values (proposal §3.2 ②), kept between runs. */
export interface BurstSettings {
  count: number;
  step: number;
  /** Start at the current position (else at `start`, typed in the row). */
  fromNow: boolean;
  processed: boolean;
  subtitles: boolean;
}

/** Everything kept in `settings.json`. */
export interface Settings {
  folder: string;
  /** F07: frame and burst format; the clipboard is always PNG. */
  format: Format;
  /** F07: JPEG quality 1–100. */
  quality: number;
  /** F04: the rendered frame includes the subtitles. */
  screenSubtitles: boolean;
  /** F07/F09: save into the media's folder when possible. */
  besideMedia: boolean;
  /** The clip method the user picked; Fast by default (it applies once the player can copy streams, R1). */
  method: Method;
  /** A Precise cut's options (R6), High quality by default. */
  encode: Encode;
  burst: BurstSettings;
  /** File-name templates (proposal §3.7). */
  templates: Record<TemplateKind, string>;
  /** "Last N s" length. */
  lastSeconds: number;
  /** Rows of the tab the user opened: the burst row, Advanced, the settings section. */
  burstOpen: boolean;
  advancedOpen: boolean;
  settingsOpen: boolean;
  /** Seconds of work per second of media of the last Precise export on this machine (the time estimate); null before one. */
  preciseSpeed: number | null;
}

export const defaultFolder = () => {
  const pictures = join(homedir(), "Pictures");
  return existsSync(pictures) ? pictures : homedir();
};

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const isFolder = (path: unknown) => {
  try {
    return typeof path === "string" && !!path && statSync(path).isDirectory();
  } catch {
    return false;
  }
};
const flag = (value: unknown, fallback: boolean) => (typeof value === "boolean" ? value : fallback);
const inRange = (value: unknown, low: number, high: number, fallback: number) => {
  const n = Number(value);
  return typeof value === "number" && Number.isFinite(n) && n >= low && n <= high ? n : fallback;
};

export const DEFAULT_BURST: BurstSettings = { count: 10, step: 1, fromNow: true, processed: false, subtitles: false };

/** Settings from stored data, field by field: an invalid field falls back alone. */
export function settingsOf(stored: unknown, folder = defaultFolder()): Settings {
  const s = isObject(stored) ? stored : {};
  const burst = isObject(s.burst) ? s.burst : {};
  const templates = isObject(s.templates) ? s.templates : {};
  return {
    folder: isFolder(s.folder) ? (s.folder as string) : folder,
    format: FORMATS.includes(s.format as Format) ? (s.format as Format) : "png",
    quality: s.quality === undefined ? 90 : quality(s.quality),
    screenSubtitles: flag(s.screenSubtitles, true),
    besideMedia: flag(s.besideMedia, false),
    method: METHODS.includes(s.method as Method) ? (s.method as Method) : "fast",
    encode: encodeOf(s.encode),
    burst: {
      count: Number.isInteger(burst.count) ? inRange(burst.count, 2, 20, DEFAULT_BURST.count) : DEFAULT_BURST.count,
      step: inRange(burst.step, 0.1, 10, DEFAULT_BURST.step),
      fromNow: flag(burst.fromNow, DEFAULT_BURST.fromNow),
      processed: flag(burst.processed, DEFAULT_BURST.processed),
      subtitles: flag(burst.subtitles, DEFAULT_BURST.subtitles),
    },
    templates: {
      image: normalTemplate("image", templates.image),
      clip: normalTemplate("clip", templates.clip),
      burst: normalTemplate("burst", templates.burst),
    },
    lastSeconds: lastSeconds(s.lastSeconds),
    burstOpen: flag(s.burstOpen, false),
    advancedOpen: flag(s.advancedOpen, false),
    settingsOpen: flag(s.settingsOpen, false),
    preciseSpeed: s.preciseSpeed === null || s.preciseSpeed === undefined ? null : inRange(s.preciseSpeed, 0.01, 1000, 0) || null,
  };
}

/**
 * The settings in `<dataDirectory>/settings.json` (`plugin.storage`): a damaged
 * file is set aside by the SDK; a partly invalid one falls back field by field,
 * and a folder that no longer exists falls back to Pictures. `save()` writes the
 * current values (debounced, and on deactivation). Without a data directory
 * they are kept for this session only.
 */
export function openSettings(plugin: Scope) {
  let store: { value: Record<string, unknown>; save(next: Record<string, unknown>): void } | null = null;
  try {
    store = plugin.storage.json<Record<string, unknown>>("settings", { defaults: {}, validate: isObject });
  } catch {
    // no data directory
  }
  const settings = settingsOf(store?.value);
  return { settings, save: () => store?.save(structuredClone(settings) as unknown as Record<string, unknown>) };
}
