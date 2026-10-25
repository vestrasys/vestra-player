import { basename } from "node:path";
import type { JsonStore, PlayerState, Scope } from "@vestra-player/sdk";
import { openStore } from "./settings.ts";
import { type Osd, sourceFailure } from "./texts.ts";

type Last = { source: string | null; openedAt?: string };
const isLast = (value: unknown): value is Last =>
  typeof value === "object" && value !== null && typeof (value as Last).source === "string";

/** A local path shows its file name; a link is shown whole. */
export const displayName = (source: string) => (source.startsWith("/") ? basename(source) : source);

/**
 * A10 Reopen Current/Last File: reload the current source (back to where it was), or
 * open the last source played, remembered across launches in `last.json`.
 */
export class Reopener {
  /** An open is in flight; presses until it returns are ignored. */
  busy = false;
  last: string | null = null;
  readonly #scope: Scope;
  readonly #store: JsonStore<Last>;
  readonly #osd: Osd;
  #recorded: number | null = null;

  constructor(scope: Scope, osd: Osd) {
    this.#scope = scope;
    this.#osd = osd;
    // `last.json`; without a data directory only the current source can be reopened.
    this.#store = openStore<Last>(scope, "last", { defaults: { source: null }, validate: isLast });
    if (this.#store.value.source) this.last = this.#store.value.source;
  }

  /** Remember a media source (whoever opened it) the first time it becomes ready. */
  observe(state: PlayerState) {
    if (state.kind !== "media" || !state.source || state.phase !== "ready") return;
    if (this.#recorded === state.sourceId) return;
    this.#recorded = state.sourceId;
    this.last = state.source;
    // Written now rather than debounced: the player may be quit right after.
    // An unwritable directory still remembers it for this run.
    this.#store.save({ source: state.source, openedAt: new Date().toISOString() });
    this.#store.flush().catch(() => {});
  }

  /** The current source when it can be reloaded (a plugin stream cannot). */
  current(): PlayerState | null {
    const state = this.#scope.player.state;
    return state.phase !== "empty" && state.kind === "media" && state.source ? state : null;
  }
  available() {
    return this.current() !== null || this.last !== null;
  }

  async run() {
    if (this.busy || !this.available()) return;
    const { player } = this.#scope;
    const current = this.current();
    this.busy = true;
    try {
      if (current) {
        const { source, position, paused } = current;
        const media = await player.open(source!, { paused });
        this.#osd("Reopened: {}", [displayName(source!)]);
        // Back to where it was, e.g. a file still being written now plays further.
        media.ready().then(
          async () => {
            const duration = player.state.duration;
            if (position !== null && position > 0 && duration !== null && position < duration && media.current)
              await player.seek(position).catch((error: Error & { code?: string }) => {
                // Someone acted on the new source first: their change stands.
                if (error?.code !== "stale") sourceFailure(error).show(this.#osd);
              });
          },
          () => {}, // a failed load is announced by the phase change
        );
      } else {
        const source = this.last!;
        await player.open(source);
        this.#osd("Reopened the last file: {}", [displayName(source)]);
      }
    } catch (error) {
      throw sourceFailure(error);
    } finally {
      this.busy = false;
    }
  }
}
