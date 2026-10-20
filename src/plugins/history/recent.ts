import { access } from "node:fs/promises";
import { join } from "node:path";
import type { PlayerState } from "../../sdk/plugin/index.ts";
import { isLocal, keyOf } from "./identity.ts";
import { JsonFile, readJson, timing } from "./store.ts";

export type RecentItem = { key: string; source: string; kind: "file" | "url"; openedAt: string };

/** Records kept in recent.json; the menu shows the first MENU_ITEMS. */
export const CAPACITY = 100;
export const MENU_ITEMS = 20;

/**
 * A12 最近播放历史: every media source (whoever opened it) the first time it
 * becomes ready, newest first, in `<dataDirectory>/recent.json`.
 */
export class Recent {
  enabled = true;
  items: RecentItem[] = [];
  readonly #file: JsonFile;
  readonly #listeners: (() => void)[] = [];
  #recorded: number | null = null;

  constructor(directory: string | null) {
    const file = directory ? join(directory, "recent.json") : null;
    this.#file = new JsonFile(file, "merge", () => timing.recentWrite, () => ({
      version: 1,
      enabled: this.enabled,
      items: this.items,
    }));
    const saved = readJson(file) as { enabled?: unknown; items?: unknown } | null;
    if (!saved || typeof saved !== "object") return;
    if (typeof saved.enabled === "boolean") this.enabled = saved.enabled;
    if (Array.isArray(saved.items))
      this.items = saved.items
        .filter(
          (item): item is RecentItem =>
            !!item &&
            typeof item.key === "string" &&
            typeof item.source === "string" &&
            item.source !== "" &&
            (item.kind === "file" || item.kind === "url") &&
            typeof item.openedAt === "string",
        )
        .slice(0, CAPACITY);
  }

  /** Called on every change of the records or the 记录播放历史 switch. */
  onChange(listener: () => void) {
    this.#listeners.push(listener);
  }

  /** Record a media source the first time its `sourceId` is ready; replays of the same source are not recorded again. */
  observe(state: PlayerState) {
    if (state.kind !== "media" || !state.source || state.phase !== "ready") return;
    if (this.#recorded === state.sourceId) return;
    this.#recorded = state.sourceId;
    if (!this.enabled) return;
    const key = keyOf(state.source);
    const item: RecentItem = {
      key,
      source: state.source,
      kind: isLocal(state.source) ? "file" : "url",
      openedAt: new Date().toISOString(),
    };
    this.items = [item, ...this.items.filter((other) => other.key !== key)].slice(0, CAPACITY);
    this.#changed();
  }

  setEnabled(enabled: boolean) {
    if (enabled === this.enabled) return;
    this.enabled = enabled;
    this.#changed();
  }

  clear() {
    this.items = [];
    this.#changed();
  }

  /** Drop local records whose file no longer exists (checked concurrently); links stay. Returns how many went. */
  async prune(): Promise<number> {
    const checked = await Promise.all(
      this.items.map(async (item) => ({ item, gone: item.kind === "file" && !(await exists(item.source)) })),
    );
    const gone = new Set(checked.filter((c) => c.gone).map((c) => c.item.key));
    if (!gone.size) return 0;
    this.items = this.items.filter((item) => !gone.has(item.key));
    this.#changed();
    return gone.size;
  }

  flush() {
    this.#file.flush();
  }

  #changed() {
    this.#file.touch();
    for (const listener of this.#listeners) listener();
  }
}

export async function exists(path: string) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
