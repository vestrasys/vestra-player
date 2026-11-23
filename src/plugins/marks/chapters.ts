import type { Chapter, I18n, Player, PlayerState, Text } from "@vestra-player/sdk";

/** The media is open far enough for chapters, bookmarks and seeking to mean something. */
export const hasMedia = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.source !== null;

/** A chapter's own title; `null` when it has none (it is then called "Chapter N"). */
export const chapterTitle = (chapter: Chapter): string | null => chapter.title?.trim() || null;

/** A chapter's name (seek-bar marks): its title, or "Chapter N" (a text the player translates). */
export const chapterName = (text: I18n["text"], chapter: Chapter, index: number): string | Text =>
  chapterTitle(chapter) ?? text("Chapter {}", [index + 1]);

/**
 * B21 chapters: read with `player.chapters()` once per source, when it first is
 * ready (the player does not push them). Cleared as soon as the source
 * changes, so an old file's chapters never show for a new one. The list
 * arrives after the state event that asked for it, outside any SDK handler,
 * so `update` (`plugin.update()`) asks for the pass that shows it.
 */
export class Chapters {
  readonly #player: Player;
  readonly #update: () => void;
  readonly #listeners: (() => void)[] = [];
  #sourceId: number | null = null;
  #loaded: number | null = null;
  #list: Chapter[] = [];
  version = 0;

  constructor(player: Player, update: () => void) {
    this.#player = player;
    this.#update = update;
  }
  onChange(listener: () => void) {
    this.#listeners.push(listener);
  }
  get list(): readonly Chapter[] {
    return this.#list;
  }

  /**
   * Follow the player: forget on a new source and on a close (which keeps the source id, with no
   * source open), read once it is ready.
   */
  observe(state: PlayerState) {
    const open = state.source === null ? null : state.sourceId;
    if (open !== this.#sourceId) {
      this.#sourceId = open;
      this.#loaded = null;
      if (this.#list.length) this.#set([]);
    }
    if (this.#loaded === state.sourceId || !hasMedia(state)) return;
    this.#loaded = state.sourceId;
    void this.#player.chapters().then(
      ({ sourceId, chapters }) => {
        if (sourceId !== this.#sourceId) return;
        this.#set(
          chapters
            .filter((chapter) => Number.isFinite(chapter.start) && chapter.start >= 0)
            .sort((a, b) => a.start - b.start),
        );
        // No player field changed: without this the window's rows wait for the SDK's safety pass.
        this.#update();
      },
      () => {
        // Refused (source gone meanwhile): try again on the next ready state.
        if (this.#loaded === state.sourceId) this.#loaded = null;
      },
    );
  }

  #set(list: Chapter[]) {
    this.#list = list;
    this.version++;
    for (const listener of this.#listeners) listener();
  }
}

/** The index of the chapter playing at `position` (the last one started), or -1. */
export function currentChapter(chapters: readonly Chapter[], position: number | null): number {
  if (position === null) return -1;
  let index = -1;
  chapters.forEach((chapter, i) => {
    if (chapter.start <= position + 0.05) index = i;
  });
  return index;
}

/**
 * The ranges in which each of the first `count` chapters is the current one
 * (`currentChapter`: from 0.05 s before its start to 0.05 s before the next
 * one's), for a trigger's `when.position.in`; `chapters[i]` is the chapter of
 * `ranges[i]`. A chapter that is never the current one (another starts at the
 * same time) has no range.
 */
export function chapterRanges(
  list: readonly Chapter[],
  count: number,
): { ranges: { start: number; end?: number }[]; chapters: number[] } {
  const ranges: { start: number; end?: number }[] = [];
  const chapters: number[] = [];
  for (let i = 0; i < Math.min(list.length, count); i++) {
    const start = Math.max(0, list[i]!.start - 0.05);
    const next = list[i + 1];
    if (!next) {
      ranges.push({ start });
      chapters.push(i);
      continue;
    }
    // The last chapter listed ends where the next (unlisted) one begins: no check past it.
    const end = next.start - 0.05;
    if (end <= start) continue;
    ranges.push({ start, end });
    chapters.push(i);
  }
  return { ranges, chapters };
}

/**
 * B21 previous / next: the point before `position − 1` (so repeated presses move
 * past the point just reached) or after `position + 0.5`; `null` when none.
 */
export function stepPoint(times: readonly number[], position: number, direction: -1 | 1): number | null {
  const sorted = [...times].sort((a, b) => a - b);
  if (direction < 0) {
    const base = position - 1;
    return sorted.filter((t) => t <= base).at(-1) ?? null;
  }
  const base = position + 0.5;
  return sorted.find((t) => t > base) ?? null;
}
