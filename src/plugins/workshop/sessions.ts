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
  /** Plugin ids installed from this session (by the agent or the card), for the delete dialog. */
  installed?: string[];
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
  private removed = new Set<string>();

  /**
   * Deletes a session: its folder sessions/<sid>/ with the transcript, the
   * work folder (the plugin projects) and dist/ (packed .vxp files). Files
   * the user exported elsewhere are not touched. The caller stops the
   * session's worker first; a file a closing process still holds (Windows)
   * is retried for a moment.
   */
  /**
   * Marks the session as being deleted, synchronously: from here on saves are
   * dropped and isRemoved() is true, so a send still starting its worker bails.
   */
  markRemoved(id: string) {
    this.directory(id); // a session id
    this.removed.add(id);
  }

  async remove(id: string): Promise<void> {
    const directory = this.directory(id);
    this.markRemoved(id);
    await (this.writes.get(id) ?? Promise.resolve()).catch(() => {});
    this.writes.delete(id);
    await fs.rm(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }

  /** remove() was called for the session (late writers check it). */
  isRemoved(id: string): boolean {
    return this.removed.has(id);
  }

  /**
   * After a remove() that failed: true when the session's meta.json is still
   * there, so it stays a session (and can be saved again); false when only
   * leftovers remain.
   */
  async restore(meta: Meta): Promise<boolean> {
    try {
      await fs.access(path.join(this.directory(meta.id), "meta.json"));
    } catch {
      return false;
    }
    this.removed.delete(meta.id);
    return true;
  }

  /**
   * Atomic (a crash leaves the old or the new meta.json, never half of one)
   * and in call order per session: the last save is what stays on disk.
   */
  save(meta: Meta): Promise<void> {
    // A late save (a run that ended) must not bring a deleted session back.
    if (this.removed.has(meta.id)) return Promise.resolve();
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
