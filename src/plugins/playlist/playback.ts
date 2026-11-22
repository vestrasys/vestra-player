import { realpath, stat } from "node:fs/promises";
import type {
  Media,
  PlayerState,
  PluginError,
  Scope,
} from "@vestra-player/sdk";
import type { Binding, Queue, Row } from "./queue.ts";

/**
 * B26: the row a natural end will open, worked out while the current one plays:
 * its realpath and whether it still exists, so the end only has to `open`.
 */
type Plan = {
  rowId: string;
  /** realpath; null when it could not be read (the advance then takes the usual path). */
  key: string | null;
  /** The file is gone: the advance skips the row. */
  missing: boolean;
  /** The file checks finished. */
  ready: boolean;
};

/** `VESTRA_PLAYLIST_TIMING=1`: print each automatic advance's timings to stderr (GUI measurement). */
const timing = !!process.env.VESTRA_PLAYLIST_TIMING;

/**
 * Opening rows and following the media they became: row status, the failed
 * navigation anchor, and advancing once after a natural end.
 */
export function playback(scope: Scope, queue: Queue, settled: () => void = () => {}) {
  const { player } = scope;
  // B26 "Fast Advance to Next" is on; `plan` is the row prepared for the next natural end.
  let fast = false;
  let plan: Plan | null = null;
  // GUI measurement of the last automatic advance (see `timing`).
  let advancing: { at: number; title: string; opened?: number; sourceId?: number } | null = null;
  // Sources this plugin opened (A13 only adopts sources other plugins opened).
  const own = new Set<number>();
  // An adopted row's natural end was handled for the current playthrough.
  let adoptedEnded = false;
  // Between the click and the host accepting the open.
  let opening: { rowId: string; epoch: number } | null = null;
  // Accepted, but the media is still loading: opening stays blocked.
  let loading: { rowId: string; sourceId: number } | null = null;
  // The source that was current when the failed anchor was set.
  let failedSource: number | null = null;

  // §11: a resolution cancelled by a close (F4, `player.close()`) fails `backend` with the
  // source empty; a resolver's own failure (or an expired resolution) leaves it in `error`.
  const cancelled = (code: string) => code === "backend" && player.state.phase === "empty";

  /** Follow one state of the player (main.ts hands over each change it hears). */
  const observe = (state: PlayerState) => {
    if (advancing?.sourceId === state.sourceId && state.phase !== "loading") {
      const now = performance.now();
      console.error(
        `[playlist] advance to ${advancing.title}: end→open accepted ${(advancing.opened! - advancing.at).toFixed(1)} ms, ` +
          `end→${state.phase} ${(now - advancing.at).toFixed(1)} ms`,
      );
      advancing = null;
    }
    if (queue.failed && failedSource !== null && state.sourceId !== failedSource) {
      queue.failed = null;
      failedSource = null;
    }
    if (loading && (state.sourceId !== loading.sourceId || state.phase !== "loading")) loading = null;
    queue.observe(state);
    if (queue.failed) failedSource = state.sourceId;
    // An adopted row (A13) has no Media: its natural end is seen in the state.
    const binding = queue.binding;
    if (binding && !binding.media && state.sourceId === binding.sourceId) {
      const ended = state.phase === "ended" && state.endReason === "natural";
      if (ended && !adoptedEnded) {
        adoptedEnded = true;
        void advance(binding, state);
      } else if (!ended) adoptedEnded = false;
    }
  };

  // Only when nobody changed the source after it ended: otherwise "stale".
  const advance = (binding: Binding, ended: PlayerState): Promise<void> | undefined => {
    if (busy() || queue.binding !== binding) return;
    if (!queue.row(queue.binding.rowId)) return;
    // Stop After Current: playback stops at this end; the row and the position stay as at the
    // end of the list under Off (shuffle has nothing to pick: nothing advances).
    if (queue.repeat === "once") return;
    if (timing) advancing = { at: performance.now(), title: "" };
    if (queue.repeat === "one") return replay(ended);
    const prepared = planned();
    plan = null;
    if (prepared?.plan.missing) {
      // B26: the next file is gone; say so and go on to the one after it.
      queue.fail(prepared.row.rowId, { kind: "missing" });
      failedSource = player.state.sourceId;
      const after = queue.neighbor(1);
      return after && after.rowId !== prepared.row.rowId ? play(after.rowId, ended) : undefined;
    }
    const next = prepared?.row ?? queue.neighbor(1);
    if (!next) queue.notice("End of the list");
    else return play(next.rowId, ended, { key: prepared?.plan.key ?? undefined });
  };

  /** The anchor `queue.neighbor` steps from. */
  const anchor = () => queue.failed ?? queue.binding?.rowId ?? queue.selected;
  /** The row a natural end would open now (shuffle: the planned pick while it is still allowed). */
  function upcoming(): Row | undefined {
    if (!queue.binding || queue.repeat === "one" || queue.repeat === "once") return undefined;
    if (!queue.shuffle) return queue.neighbor(1);
    const kept = plan && queue.candidates(anchor()).find((row) => row.rowId === plan!.rowId);
    return kept || queue.neighbor(1);
  }
  /** The prepared plan when it is still what a natural end would open. */
  function planned(): { row: Row; plan: Plan } | undefined {
    if (!fast || !plan?.ready) return undefined;
    const row = upcoming();
    return row && row.rowId === plan.rowId ? { row, plan } : undefined;
  }
  /**
   * B26: keep the plan in step with the list (called every update round and on
   * each bind): reorder, removal, repeat or shuffle changes prepare the new next row.
   */
  function prepare() {
    if (!fast) return void (plan = null);
    const row = upcoming();
    if (!row) return void (plan = null);
    if (plan?.rowId === row.rowId) return;
    const mine: Plan = { rowId: row.rowId, key: null, missing: false, ready: row.kind === "url" };
    plan = mine;
    if (row.kind !== "file") return;
    void (async () => {
      try {
        const key = await realpath(row.source);
        if ((await stat(key)).isFile()) mine.key = key;
      } catch (error) {
        const code = (error as NodeJS.ErrnoException).code;
        mine.missing = code === "ENOENT" || code === "ENOTDIR";
      }
      mine.ready = true;
    })();
  }

  /** Repeat one: replay the same media; the row stays bound and its end listener stays armed. */
  async function replay(ended: PlayerState) {
    try {
      await player.restart({ ifCurrent: ended });
    } catch (error) {
      if ((error as PluginError).code === "stale") queue.detach("The automatic advance expired; the newer source stays");
      else queue.notice("Cannot replay: {}", [(error as PluginError).message]);
    }
  }

  /**
   * Open a row; `ifCurrent` makes it an automatic advance from that ended state;
   * `quiet` keeps the current notice; `key` is a realpath read ahead (B26): the
   * open is then sent at once, with no file access first.
   */
  async function play(
    rowId: string,
    ifCurrent?: PlayerState,
    options: { quiet?: boolean; key?: string } = {},
  ): Promise<void> {
    const row = queue.row(rowId);
    if (busy() || !row) return;
    const request = { rowId, epoch: queue.begin(rowId) };
    opening = request;
    const valid = () => opening === request && request.epoch === queue.epoch && !!queue.row(rowId);
    try {
      // Only local paths take part in filesystem identity; a URL row keeps the entered URL.
      if (row.kind === "file") {
        try {
          const key = options.key ?? (await realpath(row.source));
          if (!valid()) return;
          const duplicate = queue.rows.find(
            (other) => other.rowId !== rowId && other.kind === "file" && other.key === key,
          );
          if (duplicate) {
            queue.removeRow(rowId);
            queue.selected = duplicate.rowId;
            queue.notice("The file's canonical path matches an existing item; that item is selected");
            // After an await (also from an advance nobody awaits): show it now (SDK 1.5.45).
            scope.update();
            return;
          }
          row.key = key;
        } catch {
          /* Keep inaccessible entries; the player reports the real open error. */
        }
        if (!valid()) return;
      }
      let media: Media;
      try {
        // URL rows are resolved by whichever site plugin registered a resolver.
        media = await player.open(row.source, ifCurrent ? { ifCurrent } : {});
      } catch (error) {
        if (opening !== request) return;
        opening = null;
        if (request.epoch !== queue.epoch || !queue.row(rowId)) return;
        const { code, message } = error as PluginError;
        // A failed load of this row is still this plugin's source.
        if (player.state.source === row.source) own.add(player.state.sourceId);
        if (code === "stale") queue.detach("The automatic advance expired; the newer source stays");
        // Accepted, then superseded by another source before it loaded: not a failure of this row.
        else if (code === "replaced") queue.detach("Switched to a source outside the list");
        // A file resolution (§11) cancelled by a close (F4, player.close()): not a failure of this row.
        else if (cancelled(code)) queue.detach("The current source was closed");
        else {
          queue.fail(rowId, message);
          failedSource = player.state.sourceId;
        }
        return;
      }
      if (opening !== request) return;
      opening = null;
      if (advancing && ifCurrent) {
        advancing.opened = performance.now();
        advancing.title = row.title;
        advancing.sourceId = player.state.sourceId;
      }
      if (request.epoch !== queue.epoch || !queue.row(rowId)) {
        queue.message = "The current media was removed from the list";
        return;
      }
      if (media.current) own.add(player.state.sourceId);
      const binding: Binding = { rowId, media };
      binding.end = media.onEnd((ended) => advance(binding, ended));
      queue.bind(binding);
      queue.played(rowId);
      if (!options.quiet) queue.sayTitle(row.title);
      if (media.current) loading = { rowId, sourceId: player.state.sourceId };
      observe(player.state);
      prepare();
      // It may already have ended before the reply arrived.
      const now = player.state;
      if (media.current && now.phase === "ended" && now.endReason === "natural") return advance(binding, now);
    } finally {
      if (opening === request) opening = null;
      settled();
    }
  }

  /** A13: follow `rowId` as the row of a source another plugin opened. */
  function adopt(rowId: string, state: PlayerState) {
    if (!queue.row(rowId) || busy()) return;
    queue.detach("Taken over");
    queue.failed = null;
    adoptedEnded = state.phase === "ended";
    queue.bind({ rowId, media: null, sourceId: state.sourceId });
    queue.played(rowId);
    observe(state);
    prepare();
  }

  /** An open is unsettled: no second open until it is. */
  function busy() {
    return !!opening || !!loading;
  }
  return {
    observe,
    /**
     * A natural end heard as `source.ended`, which comes once per playthrough: the
     * adopted row's end is armed again first (a restart after the end sends no event).
     */
    endedNaturally(state: PlayerState) {
      adoptedEnded = false;
      observe(state);
    },
    play,
    busy,
    adopt,
    prepare,
    /** B26: turn "Fast Advance to Next" on or off (off drops the plan). */
    setFast(on: boolean) {
      fast = on;
      prepare();
    },
    /** B26 (tests): the row prepared for the next natural end, once its checks finished. */
    get planned() {
      return planned()?.row.rowId ?? null;
    },
    /** Whether this plugin opened the source `sourceId`. */
    owns: (sourceId: number) => own.has(sourceId),
    /** The row whose open is unsettled. */
    get pendingRow() {
      return opening?.rowId ?? loading?.rowId ?? null;
    },
  };
}
