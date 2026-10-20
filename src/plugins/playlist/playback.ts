import { realpath } from "node:fs/promises";
import type {
  Media,
  PlayerState,
  PluginError,
  Scope,
} from "../../sdk/plugin/index.ts";
import type { Binding, Queue } from "./queue.ts";

/**
 * Opening rows and following the media they became: row status, the failed
 * navigation anchor, and advancing once after a natural end.
 */
export function playback(scope: Scope, queue: Queue, settled: () => void = () => {}) {
  const { player } = scope;
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

  const observe = (state: PlayerState) => {
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
  player.onChange(observe);

  // Only when nobody changed the source after it ended: otherwise "stale".
  const advance = (binding: Binding, ended: PlayerState): Promise<void> | undefined => {
    if (busy() || queue.binding !== binding) return;
    if (!queue.row(queue.binding.rowId)) return;
    if (queue.repeat === "one") return replay(ended);
    const next = queue.neighbor(1);
    if (!next) queue.notice("列表已播完");
    else return play(next.rowId, ended);
  };

  /** Repeat one: replay the same media; the row stays bound and its end listener stays armed. */
  async function replay(ended: PlayerState) {
    try {
      await player.restart({ ifCurrent: ended });
    } catch (error) {
      if ((error as PluginError).code === "stale") queue.detach("自动推进已过期，保留较新的来源");
      else queue.notice(`无法重播：${(error as PluginError).message}`);
    }
  }

  /** Open a row; `ifCurrent` makes it an automatic advance from that ended state; `quiet` keeps the current notice. */
  async function play(rowId: string, ifCurrent?: PlayerState, options: { quiet?: boolean } = {}): Promise<void> {
    const row = queue.row(rowId);
    if (busy() || !row) return;
    const request = { rowId, epoch: queue.begin(rowId) };
    opening = request;
    const valid = () => opening === request && request.epoch === queue.epoch && !!queue.row(rowId);
    try {
      // Only local paths take part in filesystem identity; a URL row keeps the entered URL.
      if (row.kind === "file") {
        try {
          const key = await realpath(row.source);
          if (!valid()) return;
          const duplicate = queue.rows.find(
            (other) => other.rowId !== rowId && other.kind === "file" && other.key === key,
          );
          if (duplicate) {
            queue.removeRow(rowId);
            queue.selected = duplicate.rowId;
            queue.notice("文件规范路径与已有项重复，已选择已有项");
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
        if (code === "stale") queue.detach("自动推进已过期，保留较新的来源");
        // Accepted, then superseded by another source before it loaded: not a failure of this row.
        else if (code === "replaced") queue.detach("已切换到列表外来源");
        else {
          queue.fail(rowId, message);
          failedSource = player.state.sourceId;
        }
        return;
      }
      if (opening !== request) return;
      opening = null;
      if (request.epoch !== queue.epoch || !queue.row(rowId)) {
        queue.message = "当前媒体已移出列表";
        return;
      }
      if (media.current) own.add(player.state.sourceId);
      const binding: Binding = { rowId, media };
      binding.end = media.onEnd((ended) => advance(binding, ended));
      queue.bind(binding);
      queue.played(rowId);
      if (!options.quiet) queue.say(row.title);
      if (media.current) loading = { rowId, sourceId: player.state.sourceId };
      observe(player.state);
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
    queue.detach("已接管");
    queue.failed = null;
    adoptedEnded = state.phase === "ended";
    queue.bind({ rowId, media: null, sourceId: state.sourceId });
    queue.played(rowId);
    observe(state);
  }

  /** An open is unsettled: no second open until it is. */
  function busy() {
    return !!opening || !!loading;
  }
  return {
    play,
    busy,
    adopt,
    /** Whether this plugin opened the source `sourceId`. */
    owns: (sourceId: number) => own.has(sourceId),
    /** The row whose open is unsettled. */
    get pendingRow() {
      return opening?.rowId ?? loading?.rowId ?? null;
    },
  };
}
