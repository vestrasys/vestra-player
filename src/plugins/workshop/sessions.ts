// Conversations on disk, in the plugin's data folder (no hidden folders, never .pi/):
//
//   sessions/<sid>/meta.json      title, times, pi's JSONL name, whether a run was cut off
//   sessions/<sid>/<pi>.jsonl     pi's session record (written by the worker; no key)
//   sessions/<sid>/work/          the agent's only folder
import { randomBytes } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

export interface Meta {
  id: string;
  title: string;
  created: number;
  updated: number;
  /** pi's JSONL file name inside the session folder, once it exists. */
  file: string | null;
  /** A run was in progress when the plugin stopped. */
  running: boolean;
}

const ID = /^s-[0-9a-z]+-[0-9a-f]{6}$/;

export class Sessions {
  readonly root: string;
  constructor(dataDirectory: string) {
    this.root = path.join(dataDirectory, "sessions");
  }

  directory(id: string): string {
    if (!ID.test(id)) throw new Error(`Not a session id: ${id}`);
    return path.join(this.root, id);
  }

  work(id: string): string {
    return path.join(this.directory(id), "work");
  }

  async create(title: string, now = Date.now()): Promise<Meta> {
    const meta: Meta = { id: `s-${now.toString(36)}-${randomBytes(3).toString("hex")}`, title, created: now, updated: now, file: null, running: false };
    await fs.mkdir(this.work(meta.id), { recursive: true });
    await this.save(meta);
    return meta;
  }

  private writes = new Map<string, Promise<void>>();

  /**
   * Atomic (a crash leaves the old or the new meta.json, never half of one)
   * and in call order per session: the last save is what stays on disk.
   */
  save(meta: Meta): Promise<void> {
    const file = path.join(this.directory(meta.id), "meta.json");
    const text = `${JSON.stringify(meta, null, 2)}\n`;
    const write = async () => {
      const temporary = `${file}.${randomBytes(4).toString("hex")}.tmp`;
      await fs.writeFile(temporary, text);
      await fs.rename(temporary, file);
    };
    const next = (this.writes.get(meta.id) ?? Promise.resolve()).catch(() => {}).then(write);
    this.writes.set(meta.id, next);
    return next;
  }

  /** Every readable session, most recently used first. */
  async list(): Promise<Meta[]> {
    let names: string[];
    try {
      names = await fs.readdir(this.root);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }
    const metas: Meta[] = [];
    for (const name of names.filter((n) => ID.test(n))) {
      try {
        const meta = JSON.parse(await fs.readFile(path.join(this.root, name, "meta.json"), "utf8")) as Meta;
        if (meta.id === name && typeof meta.title === "string") metas.push(meta);
      } catch {
        // unreadable: left alone, not listed
      }
    }
    return metas.sort((a, b) => b.updated - a.updated);
  }

  /** pi's JSONL of the session, or "" before the first reply. */
  async transcript(meta: Meta): Promise<string> {
    if (!meta.file || path.basename(meta.file) !== meta.file) return "";
    try {
      return await fs.readFile(path.join(this.directory(meta.id), meta.file), "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return "";
      throw error;
    }
  }
}

/** The sidebar group of a session: "today" or "earlier" (ai.slint names them). */
export function group(time: number, now = Date.now()): "today" | "earlier" {
  const day = (t: number) => new Date(t).toDateString();
  return day(time) === day(now) ? "today" : "earlier";
}

/** A title from the first message: one line, at most 24 characters ("" shows as New Chat). */
export function titleFrom(text: string): string {
  const line = text.replace(/\s+/g, " ").trim();
  const chars = [...line];
  return chars.length > 24 ? `${chars.slice(0, 23).join("")}…` : line;
}
