import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Scope } from "../../sdk/plugin/index.ts";
import { FORMATS, quality } from "./model.ts";
import type { Format } from "./model.ts";

/** Everything kept in `settings.json`; every field applies to all image outputs. */
export interface Settings {
  folder: string;
  /** F07: frame and burst format; the clipboard is always PNG. */
  format: Format;
  /** F07: JPEG quality 1–100. */
  quality: number;
  /** F04: 实际画面包含字幕. */
  screenSubtitles: boolean;
  /** F07/F09: 条件允许时储存到播放文件夹. */
  besideMedia: boolean;
}

export const defaultFolder = () => {
  const pictures = join(homedir(), "Pictures");
  return existsSync(pictures) ? pictures : homedir();
};

const isObject = (value: unknown): value is Partial<Settings> => typeof value === "object" && value !== null && !Array.isArray(value);
const isFolder = (path: unknown) => {
  try {
    return typeof path === "string" && !!path && statSync(path).isDirectory();
  } catch {
    return false;
  }
};

/**
 * The settings in `<dataDirectory>/settings.json` (`plugin.storage`): a damaged
 * file is set aside by the SDK; a partly invalid one falls back field by field,
 * and a folder that no longer exists falls back to Pictures. `save()` writes the
 * current values (debounced, and on deactivation). Without a data directory
 * they are kept for this session only.
 */
export function openSettings(plugin: Scope) {
  const fallback = { folder: defaultFolder(), format: "png" as Format, quality: 90, screenSubtitles: true, besideMedia: false };
  let store: { value: Partial<Settings>; save(next: Settings): void } | null = null;
  try {
    store = plugin.storage.json<Partial<Settings>>("settings", { defaults: {}, validate: isObject });
  } catch {
    // no data directory
  }
  const { folder, format, quality: q, screenSubtitles, besideMedia } = store?.value ?? {};
  const settings: Settings = {
    folder: isFolder(folder) ? folder! : fallback.folder,
    format: FORMATS.includes(format as Format) ? format! : fallback.format,
    quality: q === undefined ? fallback.quality : quality(q),
    screenSubtitles: typeof screenSubtitles === "boolean" ? screenSubtitles : fallback.screenSubtitles,
    besideMedia: typeof besideMedia === "boolean" ? besideMedia : fallback.besideMedia,
  };
  return { settings, save: () => store?.save({ ...settings }) };
}
