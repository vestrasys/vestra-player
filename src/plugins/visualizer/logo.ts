import { readFileSync } from "node:fs";
import { readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { extname, join } from "node:path";
import type { ImageRef, Plugin } from "@vestra-player/sdk";
import { extensionOf, imageInfo, imageProblem, MAX_IMAGE_BYTES } from "./images.ts";
import type { SettingsStore } from "./settings.ts";

export const TOO_LARGE = "Image too large (max 4096×4096, 16 MB)";
export const NOT_IMAGE = "Only PNG or JPEG images can be used";
export const UNREADABLE = "The logo image cannot be read; using the default logo";
const CANNOT_READ = "Cannot read the selected image";
const CUSTOM = "Using the custom logo";
const RESTORED = "Default logo restored";

/**
 * The notices of this file, as a union of literals: `vestra-plugin i18n
 * extract` reads the texts from this type where `osd.show` is called.
 */
export type LogoNotice = typeof TOO_LARGE | typeof NOT_IMAGE | typeof UNREADABLE | typeof CANNOT_READ | typeof CUSTOM | typeof RESTORED;

/**
 * H12 custom logo: a PNG or JPEG chosen with `dialogs.openFile`, checked like
 * the player checks it, copied to `<dataDirectory>/logo.<ext>` (so the
 * original may move or go) and shown with `plugin.ui.image`. A logo that
 * cannot be read falls back to the brand logo with one notice.
 */
export class Logo {
  readonly #plugin: Plugin;
  readonly #settings: SettingsStore;
  readonly #osd: (text: LogoNotice) => void;
  /** The custom logo now shown; null for the brand logo. */
  image: ImageRef | null = null;

  constructor(plugin: Plugin, settings: SettingsStore, osd: (text: LogoNotice) => void) {
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

  /** Settings ▸ Appearance ▸ Choose Logo Image…; true when the logo changed. */
  async choose(): Promise<boolean> {
    const picked = await this.#plugin.dialogs.openFile({ title: "Choose Logo Image" });
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
      this.#osd(CANNOT_READ);
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
    this.#osd(CUSTOM);
    return true;
  }

  /** Reset ▸ Idle Logo. */
  async reset() {
    const name = this.#settings.value.idle.logo;
    this.image = null;
    this.#settings.update((s) => void (s.idle.logo = null));
    if (name) await unlink(join(this.#plugin.dataDirectory, name)).catch(() => {});
    this.#osd(RESTORED);
  }
}
