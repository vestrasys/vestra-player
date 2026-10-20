import { chmodSync, readFileSync, renameSync, writeFileSync } from "node:fs";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** A12: changes within this window are written together. */
  recentWrite: 1000,
  /** A12: menu rebuilds within this window are merged. */
  menuRebuild: 500,
  /** B25: positions.json is written at most this often. */
  positionsWrite: 5000,
  /** B25: the playing position is remembered this often. */
  positionTick: 5000,
  /** B25: the 每次询问 window closes itself after this long. */
  askTimeout: 10000,
};

/** Read `file` as JSON; `null` when missing or unreadable. */
export function readJson(file: string | null): unknown {
  if (!file) return null;
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

/** Write through a temporary file and a rename (owner read/write only), so a crash never leaves half a file. */
export function writeJson(file: string, value: unknown) {
  const temporary = `${file}.${process.pid}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(value)}\n`, { mode: 0o600 });
  chmodSync(temporary, 0o600); // an old temporary file keeps its mode
  renameSync(temporary, file);
}

/**
 * One JSON file in the data directory, written some time after a change.
 * `touch()` schedules a write: `merge` waits `interval` so that changes in
 * that window are written together; `throttle` writes at once unless the last
 * write was less than `interval` ago. `flush()` writes now. Without a file (no data directory) nothing is written
 * and the values live for this run only.
 */
export class JsonFile {
  readonly file: string | null;
  readonly #mode: "merge" | "throttle";
  readonly #interval: () => number;
  readonly #value: () => unknown;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #dirty = false;
  #written = 0;

  constructor(file: string | null, mode: "merge" | "throttle", interval: () => number, value: () => unknown) {
    this.file = file;
    this.#mode = mode;
    this.#interval = interval;
    this.#value = value;
  }

  touch() {
    this.#dirty = true;
    if (this.#timer || !this.file) return;
    const wait =
      this.#mode === "merge" ? this.#interval() : Math.max(0, this.#written + this.#interval() - Date.now());
    this.#timer = setTimeout(() => this.flush(), wait);
  }

  flush() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    if (!this.#dirty || !this.file) return;
    this.#dirty = false;
    this.#written = Date.now();
    try {
      writeJson(this.file, this.#value());
    } catch {
      // Unwritable directory: the values still hold until the plugin stops.
    }
  }
}
