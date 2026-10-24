import { realpath } from "node:fs/promises";
import { basename, dirname, extname, resolve } from "node:path";
import type { Disposable, Media, PlayerState } from "@vestra-player/sdk";
import { collator, compareByFolder, SCAN_LIMIT } from "./media.ts";
import { fitName, type Notice, type NoticeArgs, type ReadFailure } from "./notices.ts";

export type Row = {
  rowId: string;
  source: string;
  key: string;
  /** Display title: `named`, else the file name (a link shows its URL). */
  title: string;
  /** Title given by a list file (C18), kept across restarts (C17). */
  named: string | null;
  kind: "file" | "url";
  status: Status;
  /** Why it cannot be read or played (shown in its tooltip), null when nothing failed. */
  error: RowError;
  /** Seconds, once the player reported it while this row played. */
  duration: number | null;
  /** Seconds read by ffprobe before playing; `duration` overrides it. */
  probed: number | null;
  /** ffprobe state: null = not asked (URL rows stay so), failed = shown as `--:--`. */
  probe: null | "pending" | "done" | "failed";
  /** Started playing at least once (drawn grey when no longer current). */
  played: boolean;
};
/** A row's state (not shown as text: the list draws current, played and failed rows). */
export type Status = "ready" | "retry" | "opening" | "paused" | "playing" | "ended" | "failed" | "switched";
/** Fixed reasons, each a sentence of the list's own; else the player's (or the system's) message. */
export type RowError = null | { kind: "missing" | "unreadable" | "unplayable" } | { kind: "message"; text: string };
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
  /** A file that cannot be read is "missing" (list files), not "unreadable, can retry". */
  missing?: boolean;
};
/** What a drop or a folder scan could not take (the notice after adding names the first). */
export type Problems = {
  skipped?: number;
  unreadable?: number;
  truncated?: boolean;
  failed?: { name: string; failure: ReadFailure }[];
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
/** Paths resolved at once while adding. */
const RESOLVE_LIMIT = 16;

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
  /** Last status (an English template); only notices (see `notice`) reach the user, as on-screen text. */
  message: string = "";
  /** Shows a notice to the user; the plugin wires this to `osd.show`. */
  say: (text: Notice, args?: NoticeArgs) => void = () => {};
  /** Shows a row's title (data, not translated) on screen. */
  sayTitle: (title: string) => void = () => {};
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
  /** rowId -> row, for every row in `rows` (lookups stay O(1) with thousands of rows). */
  private byId = new Map<string, Row>();
  /** Folded search text per row (its title and source do not change). */
  private folded = new WeakMap<Row, string>();

  get filtering() {
    return this.terms.length > 0;
  }
  matches(row: Row) {
    if (!this.terms.length) return true;
    // Title, file name and (for links) the URL; a local file's folders do not match.
    let text = this.folded.get(row);
    if (text === undefined) {
      text = fold(`${row.title}\n${fileNameOf(row)}${row.kind === "url" ? `\n${row.source}` : ""}`);
      this.folded.set(row, text);
    }
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
  notice(text: Notice, args?: NoticeArgs) {
    this.message = text;
    this.say(text, args);
  }
  row(id: string | null) {
    return id === null ? undefined : this.byId.get(id);
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
    // Local files without a known realpath are resolved first, `RESOLVE_LIMIT` at a time
    // (one after another, 3000 files took seconds on Windows).
    const sources = entries.map((entry) => (entry.kind === "file" ? resolve(entry.source) : entry.source));
    const real = new Map<number, string | null>();
    let next = 0;
    const unknown = entries.flatMap((entry, i) => (entry.kind === "file" && entry.key === undefined ? [i] : []));
    await Promise.all(
      Array.from({ length: Math.min(RESOLVE_LIMIT, unknown.length) }, async () => {
        while (next < unknown.length) {
          const i = unknown[next++]!;
          real.set(i, await realpath(sources[i]!).catch(() => null));
        }
      }),
    );
    for (const [i, entry] of entries.entries()) {
      let key = entry.key ?? (entry.kind === "url" ? this.urlKey(entry.source) : entry.source),
        error: RowError = null;
      const source = sources[i]!;
      if (real.has(i)) {
        const found = real.get(i);
        if (found) key = found;
        else {
          key = source;
          error = { kind: entry.missing ? "missing" : "unreadable" };
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
  /**
   * The notice after adding: how many were added and skipped as duplicates, then the
   * first problem (unreadable rows, `problems` of a drop or scan, the list limit).
   */
  summary(result: Added, problems: Problems = {}): [Notice, NoticeArgs] {
    const added = result.added,
      duplicates = result.duplicate;
    const dup = duplicates > 0;
    const failed = problems.failed?.[0];
    if (result.invalid) {
      const count = result.invalid;
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, not readable for now: {count}", { added, duplicates, count }]
        : ["Added: {added}, not readable for now: {count}", { added, count }];
    }
    if (problems.skipped) {
      const count = problems.skipped;
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, unsupported items skipped: {count}", { added, duplicates, count }]
        : ["Added: {added}, unsupported items skipped: {count}", { added, count }];
    }
    if (problems.unreadable) {
      const count = problems.unreadable;
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, subfolders that could not be read: {count}", { added, duplicates, count }]
        : ["Added: {added}, subfolders that could not be read: {count}", { added, count }];
    }
    if (problems.truncated) {
      const limit = SCAN_LIMIT;
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, limit of {limit} reached", { added, duplicates, limit }]
        : ["Added: {added}, limit of {limit} reached", { added, limit }];
    }
    if (failed) {
      const name = fitName(failed.name, 80);
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, could not read {name}", { added, duplicates, name }]
        : ["Added: {added}, could not read {name}", { added, name }];
    }
    if (result.refused) {
      const limit = ROW_LIMIT;
      return dup
        ? ["Added: {added}, duplicates skipped: {duplicates}, the list is full at {limit} items", { added, duplicates, limit }]
        : ["Added: {added}, the list is full at {limit} items", { added, limit }];
    }
    return dup ? ["Added: {added}, duplicates skipped: {duplicates}", { added, duplicates }] : ["Added: {added}", { added }];
  }
  /** The first row a batch created is selected when the list was empty (as "Add Files…"). */
  selectFirstAdded(result: Added, wasEmpty: boolean) {
    const first = result.fresh[0];
    if (wasEmpty && first) this.selected = first.rowId;
  }
  async addFiles(paths: string[]) {
    const wasEmpty = this.rows.length === 0;
    const result = await this.addEntries(paths.map((source) => ({ kind: "file" as const, source })));
    this.selectFirstAdded(result, wasEmpty);
    this.notice(...this.summary(result));
    return result;
  }
  addUrl(input: string) {
    const source = input.trim();
    let url: URL;
    try {
      url = new URL(source);
    } catch {
      throw new LinkError("incomplete");
    }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) throw new LinkError("refused");
    const key = url.href;
    const existing = this.rows.find((row) => row.kind === "url" && row.key === key);
    if (!existing && this.full) throw new LinkError("full");
    const row = existing ?? this.append({ source, key, named: null, kind: "url", error: null });
    this.selected = row.rowId;
    if (existing) this.notice("Existing link selected");
    else this.notice("Link added");
  }
  /** C17: a saved row, trusted as is (its realpath is learnt when probed). */
  restoreRow(entry: Required<Pick<Entry, "kind" | "source" | "named" | "duration">>) {
    const key = entry.kind === "url" ? this.urlKey(entry.source) : entry.source;
    const row = this.append({ source: entry.source, key, named: entry.named, kind: entry.kind, error: null });
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
      status: input.error ? "retry" : "ready",
      duration: null,
      probed: null,
      probe: null,
      played: false,
    };
    this.rows.push(row);
    this.byId.set(row.rowId, row);
    return row;
  }

  /** Whether `place` would move any marked row. */
  canPlace(place: Placement) {
    if (this.filtering || !this.selection.size) return false;
    const rows = this.rows,
      marked = (i: number) => this.selection.has(rows[i]!.rowId);
    if (place === "top" || place === "up") {
      for (let i = 1; i < rows.length; i++) if (marked(i) && !marked(i - 1)) return true;
    } else for (let i = 0; i < rows.length - 1; i++) if (marked(i) && !marked(i + 1)) return true;
    return false;
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
    const first = this.rows.findIndex((row) => this.selection.has(row.rowId));
    if (first < 0) return;
    this.removeRows(new Set(this.selection));
    this.selected = this.rows[Math.min(first, this.rows.length - 1)]?.rowId ?? null;
  }
  /** C07: remove the shown rows that are not selected; returns how many. */
  removeUnselected() {
    const doomed = this.visible().filter((row) => !this.selection.has(row.rowId));
    this.removeRows(new Set(doomed.map((row) => row.rowId)));
    return doomed.length;
  }
  /** Whether "Remove Unselected" would remove anything and keep something. */
  canRemoveUnselected() {
    const rows = this.visible();
    const marked = rows.filter((row) => this.selection.has(row.rowId)).length;
    return marked > 0 && marked < rows.length;
  }
  /** Remove one row without touching the selection of the others. */
  removeRow(rowId: string) {
    this.removeRows(new Set([rowId]));
  }
  /** Remove these rows in one pass (thousands at once stay linear). */
  removeRows(ids: ReadonlySet<string>) {
    const gone = ids.size ? [...ids].filter((id) => this.byId.has(id)) : [];
    if (!gone.length) return;
    this.rows = this.rows.filter((row) => !ids.has(row.rowId));
    const bound = this.binding?.rowId;
    let detach = false;
    for (const rowId of gone) {
      this.byId.delete(rowId);
      this.removed(rowId);
      this.selection.delete(rowId);
      this.shuffled.delete(rowId);
      if (this.cursor === rowId) this.cursor = null;
      if (rowId === bound) detach = true;
      if (rowId === this.failed) this.failed = null;
    }
    if (detach) this.detach("The current media was removed from the list", true);
  }
  /** `loud`: say that the current media goes on (not when something new plays right after). */
  clear(loud = true) {
    for (const row of this.rows) this.removed(row.rowId);
    this.rows = [];
    this.byId.clear();
    this.selected = null;
    this.failed = null;
    this.shuffled.clear();
    this.detach("List cleared; the current media continues", loud);
  }
  /** Stop following the bound media; `loud` shows `message` on screen (else it is only the status). */
  detach(message: string, loud = false) {
    const row = this.row(this.binding?.rowId ?? null);
    if (row) row.status = "switched";
    this.unbind();
    this.epoch++;
    if (loud) this.notice(message as Notice);
    else this.message = message;
  }
  begin(rowId: string) {
    this.detach("Opening…");
    this.failed = null;
    const row = this.row(rowId);
    if (row) {
      row.status = "opening";
      row.error = null;
    }
    return this.epoch;
  }
  /** `error`: a message of the player, else a fixed reason. */
  fail(rowId: string, error: string | { kind: "missing" | "unplayable" }) {
    const row = this.row(rowId);
    if (!row) return;
    row.status = "failed";
    row.error = typeof error === "string" ? { kind: "message", text: error } : error;
    this.failed = rowId;
    this.unbind();
    const title = fitName(row.title);
    if (typeof error === "string") this.notice("Cannot open {}: {}", [title, fitName(error, 60)]);
    else if (error.kind === "missing") this.notice("Cannot open {}: the file does not exist", [title]);
    else this.notice("Cannot open {}: cannot open this media", [title]);
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
    if (!row) return this.detach("The current media was removed from the list");
    // Another open (even of the same path) is a new source; a restart is not.
    // UI-PL-20: switching to an outside source is not announced. The bound source may
    // be a generated stream: a file a plugin's file resolver turned into one (§11),
    // e.g. a ROM in a mixed list. It never ends naturally, so only its phases count.
    if (!this.following(state)) {
      this.failed = null;
      return this.detach("Switched to a source outside the list");
    }
    if (state.duration !== null && state.duration > 0) row.duration = state.duration;
    if (state.phase === "error") {
      this.fail(row.rowId, state.error ?? { kind: "unplayable" });
    } else if (state.phase === "ready") {
      row.status = state.paused ? "paused" : "playing";
      row.error = null;
      row.played = true;
      this.message = row.status;
    } else if (state.phase === "loading") {
      row.status = "opening";
    } else if (state.phase === "ended") {
      row.status = "ended";
      row.played = true;
    } else if (state.phase === "empty") {
      this.detach("The current source was closed");
    }
  }
}

/** Why a link was not added (the link window shows it). */
export class LinkError extends Error {
  readonly code: "incomplete" | "refused" | "full";
  constructor(code: "incomplete" | "refused" | "full") {
    super(
      code === "incomplete"
        ? "Enter a complete HTTP(S) link"
        : code === "refused"
          ? "Only HTTP(S) links without a user name or password are accepted"
          : `The list is full at ${ROW_LIMIT} items`,
    );
    this.code = code;
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
