import { realpath } from "node:fs/promises";
import { basename, resolve } from "node:path";
import type { Disposable, Media, PlayerState } from "../../sdk/plugin/index.ts";

export type Row = {
  rowId: string;
  source: string;
  key: string;
  title: string;
  kind: "file" | "url";
  status: string;
  error: string;
  /** Seconds, once the player reported it while this row played. */
  duration: number | null;
  /** Seconds read by ffprobe before playing; `duration` overrides it. */
  probed: number | null;
  /** ffprobe state: null = not asked (URL rows stay so), failed = shown as `--:--`. */
  probe: null | "pending" | "done" | "failed";
  /** Started playing at least once (drawn grey when no longer current). */
  played: boolean;
};
/** The media this list opened for a row; `end` is its natural-end listener. */
export type Binding = { rowId: string; media: Media; end?: Disposable };
export type Repeat = "none" | "all" | "one";
export type Placement = "top" | "up" | "down" | "bottom";
export type SortKey = "title" | "path" | "reverse";

const collator = new Intl.Collator("zh", { numeric: true, sensitivity: "base" });

/** All ordering, selection and source association belong to this instance. */
export class Queue {
  rows: Row[] = [];
  /** Rows marked by click, Ctrl/Shift-click or Ctrl+A. */
  selection = new Set<string>();
  binding: Binding | null = null;
  failed: string | null = null;
  epoch = 0;
  /** Last status text; only notices (see `notice`) reach the user, as on-screen text. */
  message = "";
  /** Shows a notice to the user; the plugin wires this to `osd.show`. */
  say: (text: string) => void = () => {};
  /** Called with each row that leaves the list (removal, dedup or clear). */
  removed: (rowId: string) => void = () => {};
  repeat: Repeat = "none";
  shuffle = false;
  /** Rows played in the current shuffle round. */
  shuffled = new Set<string>();
  private cursor: string | null = null;
  private anchor: string | null = null;
  private nextId = 0;

  /** The row keyboard actions and "play" use; setting it selects only that row. */
  get selected() {
    return this.cursor;
  }
  set selected(id: string | null) {
    this.cursor = id;
    this.anchor = id;
    this.selection = new Set(id ? [id] : []);
  }
  notice(text: string) {
    this.message = text;
    this.say(text);
  }
  row(id: string | null) {
    return this.rows.find((row) => row.rowId === id);
  }
  private indexOf(id: string | null) {
    return this.rows.findIndex((row) => row.rowId === id);
  }
  select(id: string) {
    if (this.row(id)) this.selected = id;
  }
  /** Ctrl/Cmd-click: add or remove one row, keeping the others. */
  toggle(id: string) {
    if (!this.row(id)) return;
    if (this.selection.has(id)) this.selection.delete(id);
    else this.selection.add(id);
    this.cursor = id;
    this.anchor = id;
  }
  /** Shift-click or Shift+arrow: the range from the anchor to `id`. */
  extend(id: string) {
    const to = this.indexOf(id);
    if (to < 0) return;
    const from = this.indexOf(this.anchor);
    if (from < 0) return this.select(id);
    const [low, high] = from < to ? [from, to] : [to, from];
    this.selection = new Set(this.rows.slice(low, high + 1).map((row) => row.rowId));
    this.cursor = id;
  }
  selectAll() {
    this.selection = new Set(this.rows.map((row) => row.rowId));
    this.cursor ??= this.rows[0]?.rowId ?? null;
    this.anchor ??= this.cursor;
  }
  /** Move the cursor by `delta` (clamped); `extend` grows the range like Shift+arrow. */
  selectNeighbor(delta: number, extend = false) {
    const index = this.indexOf(this.cursor);
    const next = this.rows[index < 0 ? 0 : Math.max(0, Math.min(this.rows.length - 1, index + delta))];
    if (!next) return;
    if (extend) this.extend(next.rowId);
    else this.selected = next.rowId;
  }
  /** The rows marked for DEL / move, in list order. */
  marked() {
    return this.rows.filter((row) => this.selection.has(row.rowId));
  }

  /** Manual previous / next from the failed row, the playing row or the selection. */
  neighbor(delta: number): Row | undefined {
    const anchor = this.failed ?? this.binding?.rowId ?? this.selected;
    const index = this.indexOf(anchor);
    if (index < 0) return undefined;
    if (delta > 0 && this.shuffle) return this.pick(anchor);
    const target = index + delta;
    if (this.repeat === "all" && this.rows.length > 0)
      return this.rows[(target + this.rows.length) % this.rows.length];
    return this.rows[target];
  }
  /** Rows a shuffled "next" may choose from `anchor`; a new round under repeat-all. */
  candidates(anchor: string | null) {
    const fresh = this.rows.filter((row) => row.rowId !== anchor && !this.shuffled.has(row.rowId));
    if (fresh.length || this.repeat !== "all") return fresh;
    return this.rows.filter((row) => row.rowId !== anchor);
  }
  private pick(anchor: string | null) {
    const rows = this.candidates(anchor);
    return rows[Math.floor(Math.random() * rows.length)];
  }
  /** Called when a row starts opening: it counts as played in the shuffle round. */
  played(rowId: string) {
    if (this.shuffle) this.shuffled.add(rowId);
  }
  setShuffle(on: boolean) {
    this.shuffle = on;
    this.shuffled = new Set(on && this.binding ? [this.binding.rowId] : []);
  }

  async addFiles(paths: string[]) {
    let added = 0,
      duplicate = 0,
      invalid = 0;
    const wasEmpty = this.rows.length === 0;
    let first: string | null = null;
    for (const path of paths) {
      const source = resolve(path);
      let key = source,
        error = "";
      try {
        key = await realpath(source);
      } catch {
        error = "文件不存在或暂不可读取，可重试";
      }
      if (this.rows.some((row) => row.kind === "file" && row.key === key)) {
        duplicate++;
        continue;
      }
      const row = this.append({ source, key, title: basename(source), kind: "file", error });
      first ??= row.rowId;
      added++;
      if (error) invalid++;
    }
    if (wasEmpty && first) this.selected = first;
    this.notice(
      `已添加 ${added} 项${duplicate ? `，跳过 ${duplicate} 个重复` : ""}${invalid ? `，${invalid} 项暂不可读` : ""}`,
    );
  }
  addUrl(input: string) {
    const source = input.trim();
    let url: URL;
    try {
      url = new URL(source);
    } catch {
      throw new Error("请输入完整 HTTP(S) 链接");
    }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password)
      throw new Error("只接受不含账户密码的 HTTP(S) 链接");
    const key = url.href;
    const existing = this.rows.find((row) => row.kind === "url" && row.key === key);
    const row = existing ?? this.append({ source, key, title: source, kind: "url", error: "" });
    this.selected = row.rowId;
    this.notice(existing ? "已选择已有链接" : "已添加链接");
  }
  private append(input: Pick<Row, "source" | "key" | "title" | "kind" | "error">) {
    const row: Row = {
      ...input,
      rowId: `row-${++this.nextId}`,
      status: input.error ? "待重试" : "待播放",
      duration: null,
      probed: null,
      probe: null,
      played: false,
    };
    this.rows.push(row);
    return row;
  }

  /** Whether `place` would move any marked row. */
  canPlace(place: Placement) {
    const marks = this.rows.map((row) => this.selection.has(row.rowId));
    if (place === "top" || place === "up") return marks.some((mark, i) => mark && i > 0 && !marks[i - 1]);
    return marks.some((mark, i) => mark && i < marks.length - 1 && !marks[i + 1]);
  }
  /** Move the marked rows one step or to an end, keeping their relative order. */
  place(place: Placement) {
    const marked = (row: Row) => this.selection.has(row.rowId);
    if (place === "top") this.rows = [...this.rows.filter(marked), ...this.rows.filter((row) => !marked(row))];
    else if (place === "bottom")
      this.rows = [...this.rows.filter((row) => !marked(row)), ...this.rows.filter(marked)];
    else if (place === "up") {
      for (let i = 1; i < this.rows.length; i++)
        if (marked(this.rows[i]) && !marked(this.rows[i - 1]))
          [this.rows[i - 1], this.rows[i]] = [this.rows[i], this.rows[i - 1]];
    } else {
      for (let i = this.rows.length - 2; i >= 0; i--)
        if (marked(this.rows[i]) && !marked(this.rows[i + 1]))
          [this.rows[i], this.rows[i + 1]] = [this.rows[i + 1], this.rows[i]];
    }
  }
  /** Old single-row step, kept for keyboard compatibility: moves the selection. */
  move(delta: number) {
    this.place(delta < 0 ? "up" : "down");
  }
  /**
   * Drag reorder: drop `rowId` (with the rest of the selection when it is part of
   * it) before the row at `index` in the current order; `index = rows.length` is the end.
   */
  drop(rowId: string, index: number) {
    if (!this.row(rowId)) return;
    if (!this.selection.has(rowId)) this.selected = rowId;
    const moving = this.marked();
    const before = this.rows.slice(0, Math.max(0, Math.min(this.rows.length, index)));
    const kept = this.rows.filter((row) => !this.selection.has(row.rowId));
    const at = before.filter((row) => !this.selection.has(row.rowId)).length;
    this.rows = [...kept.slice(0, at), ...moving, ...kept.slice(at)];
  }
  sort(key: SortKey) {
    if (key === "reverse") this.rows.reverse();
    else this.rows.sort((a, b) => collator.compare(key === "title" ? a.title : a.source, key === "title" ? b.title : b.source));
  }

  /** Remove the marked rows; the cursor lands on the row after them. */
  removeSelected() {
    const marked = this.marked();
    if (!marked.length) return;
    const first = this.indexOf(marked[0].rowId);
    for (const row of marked) this.removeRow(row.rowId);
    this.selected = this.rows[Math.min(first, this.rows.length - 1)]?.rowId ?? null;
  }
  /** Remove one row without touching the selection of the others. */
  removeRow(rowId: string) {
    const index = this.indexOf(rowId);
    if (index < 0) return;
    const [removed] = this.rows.splice(index, 1);
    this.removed(rowId);
    this.selection.delete(rowId);
    this.shuffled.delete(rowId);
    if (this.cursor === rowId) this.cursor = null;
    if (removed.rowId === this.binding?.rowId) this.detach("当前媒体已移出列表", true);
    if (removed.rowId === this.failed) this.failed = null;
  }
  clear() {
    for (const row of this.rows) this.removed(row.rowId);
    this.rows = [];
    this.selected = null;
    this.failed = null;
    this.shuffled.clear();
    this.detach("列表已清空，当前媒体继续", true);
  }
  /** Stop following the bound media; `loud` shows `message` on screen. */
  detach(message: string, loud = false) {
    const row = this.row(this.binding?.rowId ?? null);
    if (row) row.status = "已切换";
    this.unbind();
    this.epoch++;
    if (loud) this.notice(message);
    else this.message = message;
  }
  begin(rowId: string) {
    this.detach("正在打开…");
    this.failed = null;
    const row = this.row(rowId);
    if (row) {
      row.status = "正在打开";
      row.error = "";
    }
    return this.epoch;
  }
  fail(rowId: string, message: string) {
    const row = this.row(rowId);
    if (!row) return;
    row.status = "失败";
    row.error = message;
    this.failed = rowId;
    this.unbind();
    this.notice(`无法打开 ${row.title}：${message}`);
  }
  bind(binding: Binding) {
    this.unbind();
    this.binding = binding;
  }
  private unbind() {
    void this.binding?.end?.dispose();
    this.binding = null;
  }
  /** Project the player state onto the bound row; natural ends arrive via `media.onEnd`. */
  observe(state: PlayerState) {
    const binding = this.binding;
    if (!binding) return;
    const row = this.row(binding.rowId);
    if (!row) return this.detach("当前媒体已移出列表");
    // Another open (even of the same path) is a new source; a restart is not.
    // UI-PL-20: switching to an outside source is not announced.
    if (!binding.media.current || state.kind !== "media") {
      this.failed = null;
      return this.detach("已切换到列表外来源");
    }
    if (state.duration !== null && state.duration > 0) row.duration = state.duration;
    if (state.phase === "error") {
      this.fail(row.rowId, state.error ?? "无法打开此媒体");
    } else if (state.phase === "ready") {
      row.status = state.paused ? "已暂停" : "播放中";
      row.error = "";
      row.played = true;
      this.message = row.status;
    } else if (state.phase === "loading") {
      row.status = "正在打开";
    } else if (state.phase === "ended") {
      row.status = "已结束";
      row.played = true;
    } else if (state.phase === "empty") {
      this.detach("当前来源已关闭");
    }
  }
}

/** A row's best known length: the player's once it played, else ffprobe's. */
export const lengthOf = (row: Row) => row.duration ?? row.probed;

/** `MM:SS`, or `H:MM:SS` from one hour (U-08). */
export function clock(seconds: number) {
  const total = Math.max(0, Math.round(seconds));
  const h = Math.floor(total / 3600),
    m = Math.floor((total % 3600) / 60),
    s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}
