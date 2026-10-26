import { access } from "node:fs/promises";
import type { JsonStore, PlayerState, Scope } from "@vestra-player/sdk";
import { isLocal, keyOf, legacyKeyOf, localPath } from "./identity.ts";
import { jsonStore, timing } from "./store.ts";

export type RecentItem = { key: string; source: string; kind: "file" | "url"; openedAt: string };
type Saved = { version: 1; enabled: boolean; items: RecentItem[] };

/** Records kept in recent.json; the menu shows the first MENU_ITEMS. */
export const CAPACITY = 100;
export const MENU_ITEMS = 20;

/**
 * A12 recent history: every media source (whoever opened it) the first time it
 * becomes ready, newest first, in `<dataDirectory>/recent.json`.
 */
export class Recent {
  enabled = true;
  items: RecentItem[] = [];
  readonly #store: JsonStore<Saved>;
  readonly #listeners: (() => void)[] = [];
  #recorded: number | null = null;

  constructor(plugin: Scope) {
    this.#store = jsonStore<Saved>(plugin, "recent", { version: 1, enabled: true, items: [] }, timing.recentWrite);
    const saved = this.#store.value as { enabled?: unknown; items?: unknown };
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
        // Derived from the source: 0.1.3 and earlier stored Windows paths as "url".
        .map((item) => ({ ...item, kind: isLocal(item.source) ? ("file" as const) : ("url" as const) }))
        .slice(0, CAPACITY);
  }

  /** Called on every change of the records or the Record Playback History switch. */
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
    // A record saved under the 0.1.3 key of the same source is this one.
    const legacy = legacyKeyOf(state.source);
    const item: RecentItem = {
      key,
      source: state.source,
      kind: isLocal(state.source) ? "file" : "url",
      openedAt: new Date().toISOString(),
    };
    this.items = [item, ...this.items.filter((other) => other.key !== key && other.key !== legacy)].slice(0, CAPACITY);
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
      this.items.map(async (item) => ({ item, gone: !(await present(item.source)) })),
    );
    const gone = new Set(checked.filter((c) => c.gone).map((c) => c.item.key));
    if (!gone.size) return 0;
    this.items = this.items.filter((item) => !gone.has(item.key));
    this.#changed();
    return gone.size;
  }

  #changed() {
    this.#store.save({ version: 1, enabled: this.enabled, items: this.items });
    for (const listener of this.#listeners) listener();
  }
}

/** Whether a local source's file exists; a link always counts as present. */
export async function present(source: string) {
  const path = localPath(source);
  if (path === null) return true;
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
