import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Scope } from "@vestra-player/sdk";
import { FORMATS, quality } from "./model.ts";
import type { Format } from "./model.ts";
import { normalTemplate } from "./names.ts";
import type { TemplateKind } from "./names.ts";

/** A burst's values (Options ▸ Video ▸ Snapshot); Burst (Ctrl+G) starts at the current position with them. */
export interface BurstSettings {
  count: number;
  step: number;
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
  burst: BurstSettings;
  /** File-name templates (proposal §3.7). */
  templates: Record<TemplateKind, string>;
  /** Save / Copy Current Frame include the video effects. */
  snapshotEffects: boolean;
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

export const DEFAULT_BURST: BurstSettings = { count: 10, step: 1, processed: false, subtitles: false };

/** A burst's frame count as typed: an integer 2–20, or null. */
export function burstCount(text: unknown): number | null {
  const n = Number(String(text ?? "").trim());
  return String(text ?? "").trim() !== "" && Number.isInteger(n) && n >= 2 && n <= 20 ? n : null;
}
/** A burst's interval in seconds as typed: 0.1–10, or null. */
export function burstStep(text: unknown): number | null {
  const n = Number(String(text ?? "").trim());
  return String(text ?? "").trim() !== "" && Number.isFinite(n) && n >= 0.1 && n <= 10 ? n : null;
}

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
    burst: {
      count: Number.isInteger(burst.count) ? inRange(burst.count, 2, 20, DEFAULT_BURST.count) : DEFAULT_BURST.count,
      step: inRange(burst.step, 0.1, 10, DEFAULT_BURST.step),
      processed: flag(burst.processed, DEFAULT_BURST.processed),
      subtitles: flag(burst.subtitles, DEFAULT_BURST.subtitles),
    },
    templates: {
      image: normalTemplate("image", templates.image),
      burst: normalTemplate("burst", templates.burst),
    },
    snapshotEffects: flag(s.snapshotEffects, false),
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
