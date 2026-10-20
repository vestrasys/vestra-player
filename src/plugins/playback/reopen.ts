import { basename, join } from "node:path";
import type { PlayerState, Scope } from "../../sdk/plugin/index.ts";
import { readJson, writeJson } from "./settings.ts";

/** A local path shows its file name; a link is shown whole. */
export const displayName = (source: string) => (source.startsWith("/") ? basename(source) : source);

/**
 * A10 重开当前/最后文件: reload the current source (back to where it was), or
 * open the last source played, remembered across launches in `last.json`.
 */
export class Reopener {
  /** An open is in flight; presses until it returns are ignored. */
  busy = false;
  last: string | null = null;
  readonly #scope: Scope;
  readonly #file: string | null;
  readonly #osd: (text: string) => void;
  #recorded: number | null = null;

  constructor(scope: Scope, directory: () => string, osd: (text: string) => void) {
    this.#scope = scope;
    this.#osd = osd;
    let file: string | null = null;
    try {
      file = join(directory(), "last.json");
    } catch {
      // No data directory: only the current source can be reopened.
    }
    this.#file = file;
    const saved = (file ? readJson(file) : null) as { source?: unknown } | null;
    if (typeof saved?.source === "string" && saved.source) this.last = saved.source;
  }

  /** Remember a media source (whoever opened it) the first time it becomes ready. */
  observe(state: PlayerState) {
    if (state.kind !== "media" || !state.source || state.phase !== "ready") return;
    if (this.#recorded === state.sourceId) return;
    this.#recorded = state.sourceId;
    this.last = state.source;
    if (!this.#file) return;
    try {
      writeJson(this.#file, { source: state.source, openedAt: new Date().toISOString() });
    } catch {
      // Unwritable: still remembered for this run.
    }
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
        this.#osd(`重新打开: ${displayName(source!)}`);
        // Back to where it was, e.g. a file still being written now plays further.
        media.ready().then(
          async () => {
            const duration = player.state.duration;
            if (position !== null && position > 0 && duration !== null && position < duration && media.current)
              await player.seek(position).catch((error: Error & { code?: string }) => {
                // Someone acted on the new source first: their change stands.
                if (error?.code !== "stale") this.#osd(`来源操作失败：${error.message}`);
              });
          },
          () => {}, // a failed load is announced by the phase change
        );
      } else {
        const source = this.last!;
        await player.open(source);
        this.#osd(`重开最后文件: ${displayName(source)}`);
      }
    } catch (error) {
      throw new Error(`来源操作失败：${error instanceof Error ? error.message : String(error)}`);
    } finally {
      this.busy = false;
    }
  }
}
