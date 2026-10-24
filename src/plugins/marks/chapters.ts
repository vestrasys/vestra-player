import type { Chapter, Player, PlayerState } from "@vestra-player/sdk";

/** The media is open far enough for chapters, bookmarks and seeking to mean something. */
export const hasMedia = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.source !== null;

/** A chapter's own title; `null` when it has none (it is then called "Chapter N"). */
export const chapterTitle = (chapter: Chapter): string | null => chapter.title?.trim() || null;

/**
 * A chapter's name as plain data, for texts the player shows as they are
 * (menu lines, seek-bar marks): its title, `Chapter N` when it has none.
 */
// i18n: needs r2-api (a translatable value inside a menu title / seek-bar mark title)
export const chapterName = (chapter: Chapter, index: number) => chapterTitle(chapter) ?? `Chapter ${index + 1}`;

/**
 * B21 chapters: read with `player.chapters()` once per source, when it first is
 * ready (the player does not push them). Cleared as soon as the source
 * changes, so an old file's chapters never show for a new one.
 */
export class Chapters {
  readonly #player: Player;
  readonly #listeners: (() => void)[] = [];
  #sourceId: number | null = null;
  #loaded: number | null = null;
  #list: Chapter[] = [];
  version = 0;

  constructor(player: Player) {
    this.#player = player;
  }
  onChange(listener: () => void) {
    this.#listeners.push(listener);
  }
  get list(): readonly Chapter[] {
    return this.#list;
  }

  /** Follow the player: forget on a new source, read once it is ready. */
  observe(state: PlayerState) {
    if (state.sourceId !== this.#sourceId) {
      this.#sourceId = state.sourceId;
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

/**
 * B23 current range: the span between the adjacent marks around `position`, the
 * start and end of the media counting as marks; `null` without marks.
 */
export function segmentAt(
  times: readonly number[],
  position: number,
  duration: number,
): { a: number; b: number } | null {
  if (!times.length) return null;
  const points = [...new Set([0, ...times.filter((t) => t > 0 && t < duration), duration])].sort((a, b) => a - b);
  for (let i = 0; i < points.length - 1; i++) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (position >= a && position < b) return { a, b };
  }
  return null;
}
