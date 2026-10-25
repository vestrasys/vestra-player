import type { I18n, JsonStore, Scope, Text } from "@vestra-player/sdk";
import { fileSize, mediaKey, round1 } from "./format.ts";
import { jsonStore, timing } from "./store.ts";

/**
 * A bookmark of the current media; `id` lives for this run only (list rows,
 * selection). `title` is the user's own; it is "" until the user names the
 * bookmark, which meanwhile is shown as "Bookmark <number>" in the interface
 * language (`bookmarkName`).
 */
export interface Bookmark {
  readonly id: number;
  readonly time: number;
  readonly title: string;
  readonly number: number;
  readonly created: string;
}
/** Saved: an untitled bookmark has `title: ""` and its `number`. */
type SavedItem = { time: number; title: string; number?: number; created: string };
type Entry = { size: number | null; modified: string; items: SavedItem[] };
type Saved = { version: 1; media: Record<string, Entry> };

/** Bookmarks per file; files remembered (least recently modified dropped first). */
export const PER_FILE = 500;
export const FILES = 2000;
export const TITLE_MAX = 100;
/** Adding within this many seconds of an existing bookmark is refused. */
const NEAR = 0.5;

const validItem = (item: unknown): item is SavedItem => {
  const it = item as SavedItem;
  return (
    !!it &&
    typeof it.time === "number" &&
    Number.isFinite(it.time) &&
    it.time >= 0 &&
    typeof it.title === "string" &&
    ([...it.title.trim()].length >= 1 || (it.title === "" && Number.isInteger(it.number) && it.number! >= 1)) &&
    [...it.title].length <= TITLE_MAX &&
    typeof it.created === "string"
  );
};

/** The name a bookmark is shown by: its title, or "Bookmark <number>" (a text the player translates). */
export const bookmarkName = (text: I18n["text"], bookmark: { title: string; number: number }): string | Text =>
  bookmark.title || text("Bookmark {}", [bookmark.number]);

/** A title as typed: trimmed, 1–100 characters; `null` when empty. */
export function cleanTitle(text: string): string | null {
  const title = text.replace(/\s+/g, " ").trim();
  if (!title) return null;
  return [...title].slice(0, TITLE_MAX).join("");
}

export type AddResult =
  | { ok: true; bookmark: Bookmark }
  | {
      ok: false;
      reason: "Can't add bookmarks to this source" | "A bookmark already exists here" | "Bookmarks full ({})";
      args?: [number];
    };

/**
 * B21 bookmarks: the bookmarks of the current media, kept in
 * `<dataDirectory>/bookmarks.json` (`{ version: 1, media: { <key>: { size,
 * modified, items } } }`), written `timing.bookmarksWrite` after each change.
 * The key is the real path of a local file, the source text of a URL.
 */
export class Bookmarks {
  readonly #store: JsonStore<Saved>;
  readonly #listeners: (() => void)[] = [];
  #source: string | null = null;
  #sourceId: number | null = null;
  #key: string | null = null;
  #size: number | null = null;
  #savedSize: number | null = null;
  #items: Bookmark[] = [];
  #nextId = 1;
  /** Bumped on every change (media switch included), for cheap change checks. */
  version = 0;

  constructor(plugin: Scope) {
    this.#store = jsonStore<Saved>(plugin, "bookmarks", { version: 1, media: {} }, timing.bookmarksWrite);
  }

  onChange(listener: () => void) {
    this.#listeners.push(listener);
  }
  #changed() {
    this.version++;
    for (const listener of this.#listeners) listener();
  }

  /**
   * The media the bookmarks belong to (`null` = none), once per source
   * (`sourceId`: reopening the same file checks its size again). The same
   * media keeps its bookmarks.
   */
  open(source: string | null, sourceId: number) {
    // Called on every state update: the real path is looked up once per source.
    if (source === this.#source && sourceId === this.#sourceId) return;
    this.#source = source;
    this.#sourceId = sourceId;
    const key = source === null ? null : mediaKey(source);
    const size = source === null ? null : fileSize(source);
    if (key === this.#key) {
      if (size === this.#size) return;
      this.#size = size;
      return this.#changed();
    }
    this.#key = key;
    this.#size = size;
    const entry = key === null ? undefined : this.#media()[key];
    this.#savedSize = entry && typeof entry.size === "number" ? entry.size : null;
    const items = entry && Array.isArray(entry.items) ? entry.items.filter(validItem).slice(0, PER_FILE) : [];
    this.#items = items
      .map((item) => ({
        id: this.#nextId++,
        time: round1(item.time),
        title: item.title.trim(),
        number: item.title ? 0 : item.number!,
        created: item.created,
      }))
      .sort((a, b) => a.time - b.time);
    this.#changed();
  }

  get media(): string | null {
    return this.#key;
  }
  get items(): readonly Bookmark[] {
    return this.#items;
  }
  get(id: number): Bookmark | undefined {
    return this.#items.find((item) => item.id === id);
  }
  /** The file is not the size it had when its bookmarks were saved: times may be off. */
  get fileChanged(): boolean {
    return this.#size !== null && this.#savedSize !== null && this.#size !== this.#savedSize;
  }

  add(time: number): AddResult {
    if (this.#key === null) return { ok: false, reason: "Can't add bookmarks to this source" };
    const at = round1(Math.max(0, time));
    if (this.#items.some((item) => Math.abs(item.time - at) <= NEAR)) return { ok: false, reason: "A bookmark already exists here" };
    if (this.#items.length >= PER_FILE) return { ok: false, reason: "Bookmarks full ({})", args: [PER_FILE] };
    // No title until the user gives it one: it is shown as "Bookmark <number>".
    const bookmark = { id: this.#nextId++, time: at, title: "", number: this.#items.length + 1, created: new Date().toISOString() };
    this.#set([...this.#items, bookmark]);
    return { ok: true, bookmark };
  }
  remove(id: number): boolean {
    if (!this.get(id)) return false;
    this.#set(this.#items.filter((item) => item.id !== id));
    return true;
  }
  rename(id: number, text: string): boolean {
    const title = cleanTitle(text);
    const item = this.get(id);
    if (!item || title === null) return false;
    if (title !== item.title) this.#set(this.#items.map((it) => (it.id === id ? { ...it, title } : it)));
    return true;
  }
  /** Move a bookmark (seconds, already clamped by the caller). */
  retime(id: number, time: number): boolean {
    const item = this.get(id);
    if (!item || !Number.isFinite(time) || time < 0) return false;
    const at = round1(time);
    if (at !== item.time) this.#set(this.#items.map((it) => (it.id === id ? { ...it, time: at } : it)));
    return true;
  }
  clear() {
    if (this.#items.length) this.#set([]);
  }

  #media(): Record<string, Entry> {
    const media = (this.#store.value as { media?: unknown }).media;
    return media && typeof media === "object" && !Array.isArray(media) ? (media as Record<string, Entry>) : {};
  }

  /** Replace the current media's bookmarks and save; an edit adopts the file's current size. */
  #set(items: Bookmark[]) {
    if (this.#key === null) return;
    this.#items = [...items].sort((a, b) => a.time - b.time);
    this.#savedSize = this.#size;
    const media = { ...this.#media() };
    delete media[this.#key];
    if (this.#items.length)
      media[this.#key] = {
        size: this.#size,
        modified: new Date().toISOString(),
        items: this.#items.map(({ time, title, number, created }) => (title ? { time, title, created } : { time, title, number, created })),
      };
    const keys = Object.keys(media);
    if (keys.length > FILES) {
      const age = (key: string) => {
        const t = Date.parse(String(media[key]?.modified));
        return Number.isFinite(t) ? t : 0;
      };
      keys.sort((a, b) => age(a) - age(b));
      for (const key of keys.slice(0, keys.length - FILES)) delete media[key];
    }
    this.#store.save({ version: 1, media });
    this.#changed();
  }
}
