import type { Chapter, MarkKind, Plugin, TimelineLayer } from "../../sdk/plugin/index.ts";
import type { Bookmarks } from "./bookmarks.ts";
import { chapterName, type Chapters, hasMedia } from "./chapters.ts";
import type { AbLoop } from "./loop.ts";
import { MARKS } from "./menu.ts";
import type { Settings } from "./store.ts";

/** The player's limits for one layer (B22). */
export const LAYER_MAX = 1000;
export const MARK_TITLE_MAX = 100;

export type SeekbarMark = { time: number; kind: MarkKind; title: string };

/**
 * B22: what this plugin puts on the seek bar: chapter starts (named as in
 * the menu), bookmarks and, while only A is set, an `A` marker (B23). Times
 * outside the media are left out (the player would refuse the whole layer),
 * then the first 1000 by time are kept.
 */
export function seekbarMarks(input: {
  chapters: readonly Chapter[];
  bookmarks: readonly { time: number; title: string }[];
  a: number | null;
  b: number | null;
  duration: number | null;
}): SeekbarMark[] {
  const title = (text: string) => [...text].slice(0, MARK_TITLE_MAX).join("");
  const marks: SeekbarMark[] = [
    ...input.chapters.map((chapter, i) => ({ time: chapter.start, kind: "chapter" as const, title: title(chapterName(chapter, i)) })),
    ...input.bookmarks.map((bookmark) => ({ time: bookmark.time, kind: "bookmark" as const, title: title(bookmark.title) })),
  ];
  if (input.a !== null && input.b === null) marks.push({ time: input.a, kind: "marker", title: "A" });
  const { duration } = input;
  return marks
    .filter((mark) => Number.isFinite(mark.time) && mark.time >= 0 && (duration === null || mark.time <= duration))
    .sort((x, y) => x.time - y.time) // stable: chapters before bookmarks at the same time
    .slice(0, LAYER_MAX);
}

/**
 * B22 发布侧: this plugin's timeline layer follows the chapters, the
 * bookmarks, A and the 在进度条上显示标记 switch (`marks.json`
 * `showOnSeekbar`). Each write is conditional on the source it was built for
 * (`ifCurrent`); the player clears every layer on a new source, and this
 * publishes again once the new one is ready. Switched off: an empty layer.
 */
export class SeekbarMarks {
  readonly #plugin: Plugin;
  readonly #settings: Settings;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #loop: AbLoop;
  readonly #layer: TimelineLayer;
  readonly #osd: (text: string) => void;
  #scheduled = false;
  #sourceId: number | null = null;
  /** The inputs of the last build, and what the player holds for this source. */
  #inputs: string | null = null;
  #sent = "[]";

  constructor(
    plugin: Plugin,
    settings: Settings,
    chapters: Chapters,
    bookmarks: Bookmarks,
    loop: AbLoop,
    osd: (text: string) => void,
  ) {
    this.#plugin = plugin;
    this.#settings = settings;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#loop = loop;
    this.#osd = osd;
    this.#layer = plugin.timeline.layer();
    const schedule = () => this.schedule();
    chapters.onChange(schedule);
    bookmarks.onChange(schedule);
    loop.onPoints(schedule);
  }

  get shown(): boolean {
    return this.#settings.showOnSeekbar;
  }

  /** The menu switch (PotPlayer S:3870). */
  toggle() {
    const show = !this.shown;
    this.#settings.setShowOnSeekbar(show);
    this.#osd(`进度条上显示书签/章节标记：${show ? "开" : "关"}`);
    this.schedule();
  }

  /** Publish after the current update: a source switch changes chapters and bookmarks one after the other. */
  schedule() {
    if (this.#scheduled) return;
    this.#scheduled = true;
    queueMicrotask(() => {
      this.#scheduled = false;
      this.#publish();
    });
  }

  #publish() {
    const state = this.#plugin.player.state;
    if (state.sourceId !== this.#sourceId) {
      // The player emptied every layer for the new source.
      this.#sourceId = state.sourceId;
      this.#sent = "[]";
      this.#inputs = null;
    }
    const media = hasMedia(state);
    const show = this.shown;
    const loop = this.#loop;
    const inputs = JSON.stringify([media, show, state.duration, this.#chapters.version, this.#bookmarks.version, loop.a, loop.b]);
    if (inputs === this.#inputs) return;
    this.#inputs = inputs;
    const marks =
      media && show
        ? seekbarMarks({
            chapters: this.#chapters.list,
            bookmarks: this.#bookmarks.items,
            a: loop.a,
            b: loop.b,
            duration: state.duration,
          })
        : [];
    const sent = JSON.stringify(marks);
    if (sent === this.#sent) return;
    this.#sent = sent;
    this.#layer.set(marks, { ifCurrent: state });
  }
}

/** 播放 ▶ 章节/书签 ▶ 在进度条上显示标记 (B22, order 30000: after the chapter and bookmark lines). */
export function addSeekbarCommand(plugin: Plugin, seekbar: SeekbarMarks) {
  plugin.commands.add({
    id: "marks.seekbar",
    path: [...MARKS, "在进度条上显示标记"],
    order: 30000,
    group: "marks-option",
    checked: () => seekbar.shown,
    run: () => seekbar.toggle(),
  });
}
