import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export type Settings = {
  openUrl: { pasteFromClipboard: boolean };
  preventSleep: boolean;
  hoverTime: boolean;
};

const defaults = (): Settings => ({ openUrl: { pasteFromClipboard: true }, preventSleep: true, hoverTime: true });

/** Read `file` as JSON; `null` when missing or unreadable. */
export function readJson(file: string): unknown {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

/** Write through a temporary file and a rename, so a crash never leaves half a file. */
export function writeJson(file: string, value: unknown) {
  const temporary = `${file}.${process.pid}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(temporary, file);
}

/** `settings.json` in the plugin's data directory; bad or missing fields fall back to the defaults. */
export class SettingsStore {
  readonly value: Settings;
  readonly #file: string | null;

  constructor(directory: () => string) {
    let file: string | null = null;
    try {
      file = join(directory(), "settings.json");
    } catch {
      // No data directory: settings live for this run only.
    }
    this.#file = file;
    this.value = defaults();
    const raw = (file ? readJson(file) : null) as Record<string, unknown> | null;
    if (!raw || typeof raw !== "object") return;
    const openUrl = raw.openUrl as Record<string, unknown> | undefined;
    if (typeof openUrl?.pasteFromClipboard === "boolean")
      this.value.openUrl.pasteFromClipboard = openUrl.pasteFromClipboard;
    if (typeof raw.preventSleep === "boolean") this.value.preventSleep = raw.preventSleep;
    if (typeof raw.hoverTime === "boolean") this.value.hoverTime = raw.hoverTime;
  }

  /** Apply `change` and save; a failed write keeps the value for this run. */
  update(change: (settings: Settings) => void) {
    change(this.value);
    if (!this.#file) return;
    try {
      writeJson(this.#file, this.value);
    } catch {
      // Unwritable directory: the choice still holds until the plugin stops.
    }
  }
}
