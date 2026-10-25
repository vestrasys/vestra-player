import type { Chapter, I18n, MarkKind, Plugin, Text, TimelineLayer } from "@vestra-player/sdk";
import { type Bookmarks, bookmarkName } from "./bookmarks.ts";
import { chapterName, type Chapters, hasMedia } from "./chapters.ts";
import type { AbLoop } from "./loop.ts";
import { MARKS } from "./menu.ts";
import type { Osd } from "./osd.ts";
import type { Settings } from "./store.ts";

/** The player's limits for one layer (B22). */
export const LAYER_MAX = 1000;
export const MARK_TITLE_MAX = 100;

export type SeekbarMark = { time: number; kind: MarkKind; title: string | Text };

/**
 * B22: what this plugin puts on the seek bar: chapter starts and bookmarks
 * (named as in the menu: "Chapter N" / "Bookmark N" without a title) and,
 * while only A is set, an `A` marker (B23). Times outside the media are left
 * out (the player would refuse the whole layer), then the first 1000 by time
 * are kept.
 */
export function seekbarMarks(
  text: I18n["text"],
  input: {
    chapters: readonly Chapter[];
    bookmarks: readonly { time: number; title: string; number: number }[];
    a: number | null;
    b: number | null;
    duration: number | null;
  },
): SeekbarMark[] {
  // Own titles are cut to the player's limit; "Chapter N" / "Bookmark N" are short.
  const title = (name: string | Text) => (typeof name === "string" ? [...name].slice(0, MARK_TITLE_MAX).join("") : name);
  const marks: SeekbarMark[] = [
    ...input.chapters.map((chapter, i) => ({ time: chapter.start, kind: "chapter" as const, title: title(chapterName(text, chapter, i)) })),
    ...input.bookmarks.map((bookmark) => ({ time: bookmark.time, kind: "bookmark" as const, title: title(bookmarkName(text, bookmark)) })),
  ];
  if (input.a !== null && input.b === null) marks.push({ time: input.a, kind: "marker", title: "A" });
  const { duration } = input;
  return marks
    .filter((mark) => Number.isFinite(mark.time) && mark.time >= 0 && (duration === null || mark.time <= duration))
    .sort((x, y) => x.time - y.time) // stable: chapters before bookmarks at the same time
    .slice(0, LAYER_MAX);
}

/**
 * B22 publishing: this plugin's timeline layer follows the chapters, the
 * bookmarks, A and the Show Marks on Seek Bar switch (`marks.json`
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
  readonly #osd: Osd;
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
    osd: Osd,
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

  /** The menu switch. */
  toggle() {
    const show = !this.shown;
    this.#settings.setShowOnSeekbar(show);
    this.#osd(
      show ? "Bookmark and chapter marks on the seek bar: on" : "Bookmark and chapter marks on the seek bar: off",
    );
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
        ? seekbarMarks(this.#plugin.i18n.text, {
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

/** Playback ▸ Chapters and Bookmarks ▸ Show Marks on Seek Bar (B22, order 30000: after the chapter and bookmark lines). */
export function addSeekbarCommand(plugin: Plugin, seekbar: SeekbarMarks) {
  plugin.commands.add({
    id: "marks.seekbar",
    path: [...MARKS, "Show Marks on Seek Bar"],
    order: 30000,
    group: "marks-option",
    checked: () => seekbar.shown,
    run: () => seekbar.toggle(),
  });
}
