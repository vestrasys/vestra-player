import { readFileSync } from "node:fs";
import { readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { extname, join } from "node:path";
import type { ImageRef, Plugin } from "@vestra-player/sdk";
import { extensionOf, imageInfo, imageProblem, MAX_IMAGE_BYTES } from "./images.ts";
import type { SettingsStore } from "./settings.ts";

export const TOO_LARGE = "图片过大（最大 4096×4096、16 MB）";
export const NOT_IMAGE = "只能使用 PNG 或 JPEG 图片";
export const UNREADABLE = "徽标图片无法读取，已使用默认徽标";

/**
 * H12 custom logo: a PNG or JPEG chosen with `dialogs.openFile`, checked like
 * the player checks it, copied to `<dataDirectory>/logo.<ext>` (so the
 * original may move or go) and shown with `plugin.ui.image`. A logo that
 * cannot be read falls back to the brand logo with one notice.
 */
export class Logo {
  readonly #plugin: Plugin;
  readonly #settings: SettingsStore;
  readonly #osd: (text: string) => void;
  /** The custom logo now shown; null for the brand logo. */
  image: ImageRef | null = null;

  constructor(plugin: Plugin, settings: SettingsStore, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#settings = settings;
    this.#osd = osd;
    const name = settings.value.idle.logo;
    if (!name) return;
    try {
      const bytes = readFileSync(join(plugin.dataDirectory, name));
      if (imageProblem(bytes)) throw new Error("unusable");
      this.image = plugin.ui.image(join(plugin.dataDirectory, name));
    } catch {
      this.fallBack();
    }
  }

  get custom() {
    return this.image !== null;
  }

  /** The custom logo could not be read (here or by the player): brand logo, one notice. */
  fallBack() {
    if (this.#settings.value.idle.logo === null && this.image === null) return;
    this.image = null;
    this.#settings.update((s) => void (s.idle.logo = null));
    this.#osd(UNREADABLE);
  }

  /** 设置 ▶ 外观 ▶ 选择徽标图片…; true when the logo changed. */
  async choose(): Promise<boolean> {
    const picked = await this.#plugin.dialogs.openFile({ title: "选择徽标图片" });
    if (!picked) return false;
    if (![".png", ".jpg", ".jpeg"].includes(extname(picked).toLowerCase())) {
      this.#osd(NOT_IMAGE);
      return false;
    }
    let bytes: Buffer;
    try {
      if ((await stat(picked)).size > MAX_IMAGE_BYTES) {
        this.#osd(TOO_LARGE);
        return false;
      }
      bytes = await readFile(picked);
    } catch {
      this.#osd("无法读取所选图片");
      return false;
    }
    const problem = imageProblem(bytes);
    if (problem) {
      this.#osd(problem === "size" ? TOO_LARGE : NOT_IMAGE);
      return false;
    }
    const name = `logo.${extensionOf(imageInfo(bytes)!)}`;
    const directory = this.#plugin.dataDirectory;
    const target = join(directory, name);
    const temporary = `${target}.${process.pid}.tmp`;
    await writeFile(temporary, bytes);
    await rename(temporary, target);
    // The other extension's copy is stale now.
    for (const other of ["logo.png", "logo.jpg"]) if (other !== name) await unlink(join(directory, other)).catch(() => {});
    this.#settings.update((s) => void (s.idle.logo = name));
    // A new reference even for the same path: the file's content changed.
    this.image = this.#plugin.ui.image(target);
    this.#osd("已使用自定义徽标");
    return true;
  }

  /** 设置 ▶ 外观 ▶ 恢复默认徽标. */
  async reset() {
    const name = this.#settings.value.idle.logo;
    this.image = null;
    this.#settings.update((s) => void (s.idle.logo = null));
    if (name) await unlink(join(this.#plugin.dataDirectory, name)).catch(() => {});
    this.#osd("已恢复默认徽标");
  }
}
