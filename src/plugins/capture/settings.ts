import { existsSync } from "node:fs";
import { readFile, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { FORMATS, message, quality } from "./model.ts";
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

const defaults = (): Settings => ({ folder: defaultFolder(), format: "png", quality: 90, screenSubtitles: true, besideMedia: false });

/** The saved settings; a missing, damaged or partly invalid file falls back field by field. */
export async function loadSettings(dataDirectory: string | null): Promise<Settings> {
  const settings = defaults();
  if (!dataDirectory) return settings;
  let saved: Record<string, unknown>;
  try {
    saved = JSON.parse(await readFile(join(dataDirectory, "settings.json"), "utf8")) ?? {};
  } catch {
    return settings; // first start or damaged file
  }
  const { folder, format, quality: q, screenSubtitles, besideMedia } = saved;
  if (typeof folder === "string" && folder && (await stat(folder).catch(() => null))?.isDirectory()) settings.folder = folder;
  if (FORMATS.includes(format as Format)) settings.format = format as Format;
  if (q !== undefined) settings.quality = quality(q);
  if (typeof screenSubtitles === "boolean") settings.screenSubtitles = screenSubtitles;
  if (typeof besideMedia === "boolean") settings.besideMedia = besideMedia;
  return settings;
}

/**
 * Saves run one after another; each writes a temporary file, then renames it
 * over `settings.json`, so a crash never leaves half a file.
 */
export function saver(dataDirectory: string | null) {
  let chain = Promise.resolve();
  return (settings: Settings) => {
    if (!dataDirectory) return chain; // no data directory: kept for this session only
    const text = `${JSON.stringify(settings, null, 2)}\n`;
    chain = chain.then(async () => {
      const file = join(dataDirectory, "settings.json");
      const temporary = `${file}.${process.pid}.tmp`;
      try {
        await writeFile(temporary, text);
        await rename(temporary, file);
      } catch (error) {
        console.error(`[capture] cannot save settings: ${message(error)}`);
      }
    });
    return chain;
  };
}
