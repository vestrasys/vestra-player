import { realpath } from "node:fs/promises";
import { basename, dirname, extname, resolve } from "node:path";
import type { Disposable, Media, PlayerState } from "../../sdk/plugin/index.ts";
import { collator, compareByFolder } from "./media.ts";

export type Row = {
  rowId: string;
  source: string;
  key: string;
  /** Display title: `named`, else the file name (a link shows its URL). */
  title: string;
  /** Title given by a list file (C18), kept across restarts (C17). */
  named: string | null;
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
/**
 * The source a row follows: media this list opened (`end` is its natural-end
 * listener), or an adopted source another plugin opened (A13: `media` null, followed
 * by `sourceId`).
 */
export type Binding = { rowId: string; media: Media | null; sourceId?: number; end?: Disposable };
export type Repeat = "none" | "all" | "one";
export type Placement = "top" | "up" | "down" | "bottom";
/** C11 keys; `shuffle` and `reverse` reorder without being a key the direction re-applies. */
export type SortKey =
  | "title"
  | "filename"
  | "extension"
  | "size"
  | "duration"
  | "date"
  | "shuffle"
  | "folder"
  | "path"
  | "reverse";
/** One row to add: local file or link; `key` is the realpath when already known. */
export type Entry = {
  kind: "file" | "url";
  source: string;
  key?: string;
  named?: string | null;
  duration?: number | null;
  /** Error shown when the file cannot be read (instead of the default). */
  missing?: string;
};
export type Added = {
  /** Per entry: its new or existing row (undefined beyond the size limit). */
  rows: (Row | undefined)[];
  /** The rows this batch created, in order. */
  fresh: Row[];
  added: number;
  duplicate: number;
  invalid: number;
  /** Entries refused because the list is full. */
  refused: number;
};

/** C17: the list holds at most this many rows. */
export const ROW_LIMIT = 20_000;
const UNREADABLE = "文件不存在或暂不可读取，可重试";

/** C21: NFKC, lower case. */
export const fold = (text: string) => text.normalize("NFKC").toLowerCase();
/** A link's last path segment (decoded), else its URL. */
export function fileNameOf(row: Pick<Row, "kind" | "source">) {
  if (row.kind === "file") return basename(row.source);
  try {
    const last = new URL(row.source).pathname.split("/").filter(Boolean).at(-1);
    return last ? decodeURIComponent(last) : row.source;
  } catch {
    return row.source;
  }
}

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
  /** C21 search terms (folded); empty = no filter. */
  terms: string[] = [];
  /** C11 last sort key and direction. */
  sortKey: Exclude<SortKey, "shuffle" | "reverse"> | null = null;
  descending = false;
  private cursor: string | null = null;
  private anchor: string | null = null;
  private nextId = 0;

  get filtering() {
    return this.terms.length > 0;
  }
  matches(row: Row) {
    if (!this.terms.length) return true;
    // Title, file name and (for links) the URL; a local file's folders do not match.
    const text = fold(`${row.title}\n${fileNameOf(row)}${row.kind === "url" ? `\n${row.source}` : ""}`);
    return this.terms.every((term) => text.includes(term));
  }
  /** The rows shown (all of them unless a search filters). */
  visible() {
    return this.terms.length ? this.rows.filter((row) => this.matches(row)) : this.rows;
  }
  /** C21: filter by the words of `query`; hidden rows lose their selection. */
  setFilter(query: string) {
    this.terms = fold(query).split(/\s+/).filter(Boolean);
    if (!this.terms.length) return;
    for (const row of this.rows) if (!this.matches(row)) this.selection.delete(row.rowId);
    if (this.cursor && !this.selection.has(this.cursor) && !this.matches(this.row(this.cursor)!)) this.cursor = null;
    if (this.anchor && !this.row(this.anchor)) this.anchor = null;
  }

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
  indexOf(id: string | null) {
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
    const rows = this.visible();
    const to = rows.findIndex((row) => row.rowId === id);
    if (to < 0) return;
    const from = rows.findIndex((row) => row.rowId === this.anchor);
    if (from < 0) return this.select(id);
    const [low, high] = from < to ? [from, to] : [to, from];
    this.selection = new Set(rows.slice(low, high + 1).map((row) => row.rowId));
    this.cursor = id;
  }
  /** Ctrl+A: every shown row. */
  selectAll() {
    const rows = this.visible();
    this.selection = new Set(rows.map((row) => row.rowId));
    if (!this.cursor || !this.selection.has(this.cursor)) this.cursor = rows[0]?.rowId ?? null;
    this.anchor ??= this.cursor;
  }
  /** Move the cursor by `delta` among the shown rows (clamped); `extend` grows the range like Shift+arrow. */
  selectNeighbor(delta: number, extend = false) {
    const rows = this.visible();
    const index = rows.findIndex((row) => row.rowId === this.cursor);
    const next = rows[index < 0 ? 0 : Math.max(0, Math.min(rows.length - 1, index + delta))];
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

  /** Whether the list can take more rows; announces the limit when it cannot. */
  get full() {
    return this.rows.length >= ROW_LIMIT;
  }
  /**
   * Append entries, skipping duplicates (realpath for files, the text for links).
   * Unreadable files are kept as retryable rows. Beyond `ROW_LIMIT` entries are refused.
   * `before` (A15 drop on the list): the new rows go before the row at that index
   * of the order before adding (`rows.length` or more = the end); duplicates stay put.
   */
  async addEntries(entries: Entry[], options: { before?: number } = {}): Promise<Added> {
    const result: Added = { rows: [], fresh: [], added: 0, duplicate: 0, invalid: 0, refused: 0 };
    const files = new Map<string, Row>(),
      links = new Map<string, Row>();
    for (const row of this.rows) (row.kind === "file" ? files : links).set(row.key, row);
    for (const entry of entries) {
      let key = entry.key ?? (entry.kind === "url" ? this.urlKey(entry.source) : entry.source),
        error = "";
      const source = entry.kind === "file" ? resolve(entry.source) : entry.source;
      if (entry.kind === "file" && entry.key === undefined) {
        try {
          key = await realpath(source);
        } catch {
          key = source;
          error = entry.missing ?? UNREADABLE;
        }
      }
      const known = (entry.kind === "file" ? files : links).get(key);
      if (known) {
        result.duplicate++;
        result.rows.push(known);
        continue;
      }
      if (this.full) {
        result.refused++;
        result.rows.push(undefined);
        continue;
      }
      const row = this.append({ source, key, named: entry.named ?? null, kind: entry.kind, error });
      if (entry.duration) row.probed = entry.duration;
      (entry.kind === "file" ? files : links).set(key, row);
      result.rows.push(row);
      result.fresh.push(row);
      result.added++;
      if (error) result.invalid++;
    }
    if (options.before !== undefined && result.fresh.length) {
      const fresh = new Set(result.fresh);
      const others = this.rows.filter((row) => !fresh.has(row));
      const at = Math.max(0, Math.min(others.length, Math.floor(options.before)));
      this.rows = [...others.slice(0, at), ...result.fresh, ...others.slice(at)];
    }
    return result;
  }
  /** The notice after adding: `已添加 N 项，跳过 M 个重复…`, then the limit when reached. */
  summary(result: Added, extra = "") {
    const parts = [`已添加 ${result.added} 项`];
    if (result.duplicate) parts.push(`跳过 ${result.duplicate} 个重复`);
    if (result.invalid) parts.push(`${result.invalid} 项暂不可读`);
    if (extra) parts.push(extra);
    if (result.refused) parts.push(`列表已达上限 ${ROW_LIMIT} 项`);
    return parts.join("，");
  }
  /** The first row a batch created is selected when the list was empty (as "添加文件"). */
  selectFirstAdded(result: Added, wasEmpty: boolean) {
    const first = result.fresh[0];
    if (wasEmpty && first) this.selected = first.rowId;
  }
  async addFiles(paths: string[]) {
    const wasEmpty = this.rows.length === 0;
    const result = await this.addEntries(paths.map((source) => ({ kind: "file" as const, source })));
    this.selectFirstAdded(result, wasEmpty);
    this.notice(this.summary(result));
    return result;
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
    if (!existing && this.full) throw new Error(`列表已达上限 ${ROW_LIMIT} 项`);
    const row = existing ?? this.append({ source, key, named: null, kind: "url", error: "" });
    this.selected = row.rowId;
    this.notice(existing ? "已选择已有链接" : "已添加链接");
  }
  /** C17: a saved row, trusted as is (its realpath is learnt when probed). */
  restoreRow(entry: Required<Pick<Entry, "kind" | "source" | "named" | "duration">>) {
    const key = entry.kind === "url" ? this.urlKey(entry.source) : entry.source;
    const row = this.append({ source: entry.source, key, named: entry.named, kind: entry.kind, error: "" });
    row.probed = entry.duration;
    return row;
  }
  /** Links are compared by their normalized HTTP(S) URL, other schemes by their text. */
  urlKey(source: string) {
    try {
      const url = new URL(source);
      return ["http:", "https:"].includes(url.protocol) ? url.href : source;
    } catch {
      return source;
    }
  }
  private append(input: Pick<Row, "source" | "key" | "named" | "kind" | "error">) {
    const row: Row = {
      ...input,
      title: input.named ?? (input.kind === "file" ? basename(input.source) : input.source),
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
    if (this.filtering) return false;
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
    if (!this.row(rowId) || this.filtering) return;
    if (!this.selection.has(rowId)) this.selected = rowId;
    const moving = this.marked();
    const before = this.rows.slice(0, Math.max(0, Math.min(this.rows.length, index)));
    const kept = this.rows.filter((row) => !this.selection.has(row.rowId));
    const at = before.filter((row) => !this.selection.has(row.rowId)).length;
    this.rows = [...kept.slice(0, at), ...moving, ...kept.slice(at)];
  }
  /**
   * C11: stable sort by `key` in the current direction; rows without a value stay last
   * in both directions. `values` gives size / date per row (missing = no value);
   * `shuffle` ignores the direction; `reverse` only reverses the current order.
   */
  sort(key: SortKey, options: { values?: Map<string, number>; random?: () => number } = {}) {
    if (key === "reverse") return void this.rows.reverse();
    if (key === "shuffle") {
      const random = options.random ?? Math.random;
      for (let i = this.rows.length - 1; i > 0; i--) {
        const j = Math.floor(random() * (i + 1));
        [this.rows[i], this.rows[j]] = [this.rows[j]!, this.rows[i]!];
      }
      return;
    }
    this.sortKey = key;
    const direction = this.descending ? -1 : 1;
    const name = (row: Row) => fileNameOf(row);
    const value = (row: Row): string | number | null => {
      switch (key) {
        case "title":
          return row.title;
        case "filename":
          return name(row);
        case "extension":
          return extname(name(row)).slice(1).toLowerCase();
        case "size":
        case "date":
          return options.values?.get(row.rowId) ?? null;
        case "duration":
          return lengthOf(row);
        case "folder":
          return row.kind === "file" ? row.source : null;
        case "path":
          return row.source;
      }
    };
    const compare = (a: Row, b: Row) => {
      const x = value(a),
        y = value(b);
      if (x === null || y === null) return x === y ? 0 : x === null ? 1 : -1;
      let order: number;
      if (key === "folder") order = compareByFolder(x as string, y as string);
      else if (typeof x === "number") order = x - (y as number);
      else order = collator.compare(x, y as string);
      if (!order && key === "extension") order = collator.compare(name(a), name(b));
      return order * direction;
    };
    this.rows.sort(compare);
  }

  /** Remove the marked rows; the cursor lands on the row after them. */
  removeSelected() {
    const marked = this.marked();
    if (!marked.length) return;
    const first = this.indexOf(marked[0].rowId);
    for (const row of marked) this.removeRow(row.rowId);
    this.selected = this.rows[Math.min(first, this.rows.length - 1)]?.rowId ?? null;
  }
  /** C07: remove the shown rows that are not selected; returns how many. */
  removeUnselected() {
    const doomed = this.visible().filter((row) => !this.selection.has(row.rowId));
    for (const row of doomed) this.removeRow(row.rowId);
    return doomed.length;
  }
  /** Whether "移除未选" would remove anything and keep something. */
  canRemoveUnselected() {
    const rows = this.visible();
    const marked = rows.filter((row) => this.selection.has(row.rowId)).length;
    return marked > 0 && marked < rows.length;
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
  /** Whether the bound row still follows the current source. */
  following(state: PlayerState) {
    const binding = this.binding;
    if (!binding) return false;
    return binding.media ? binding.media.current : state.sourceId === binding.sourceId;
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
    if (!this.following(state) || state.kind !== "media") {
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
