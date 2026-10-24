import type { Plugin } from "@vestra-player/sdk";
import type { Bookmarks } from "./bookmarks.ts";
import { type Chapters, chapterTitle, hasMedia, stepPoint } from "./chapters.ts";
import { clock } from "./format.ts";
import type { Osd } from "./osd.ts";

/** Why a chapter/bookmark command is unavailable (shown as the command's reason). */
export type JumpBlocked = "No media open" | "This source can't seek";
export type AddBlocked = "No media open" | "Can't add bookmarks to this source";

/**
 * The B21 actions shared by the menu, the shortcuts and the list window:
 * jump to a chapter or bookmark, step between them, add a bookmark here.
 */
export class Actions {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #osd: Osd;

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, osd: Osd) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#osd = osd;
  }

  /** Why jumping is unavailable; `null` when it can run. */
  jumpBlocked(): JumpBlocked | null {
    const state = this.#plugin.player.state;
    if (!hasMedia(state)) return "No media open";
    if (!state.can.seek) return "This source can't seek";
    return null;
  }
  /** Why adding a bookmark is unavailable; `null` when it can run. */
  addBlocked(): AddBlocked | null {
    const state = this.#plugin.player.state;
    if (!hasMedia(state)) return "No media open";
    if (state.phase !== "ready" || !state.can.seek || state.position === null)
      return "Can't add bookmarks to this source";
    return null;
  }

  /** Seek, conditional on the state seen now; a refusal because someone else just acted is dropped. */
  async #seek(time: number, done: () => void) {
    try {
      await this.#plugin.player.seek(time);
      done();
    } catch (error) {
      if ((error as { code?: string }).code !== "stale") this.#osd("Can't seek: {}", [(error as Error).message]);
    }
  }

  jumpChapter(index: number) {
    const chapter = this.#chapters.list[index];
    if (!chapter || this.jumpBlocked()) return;
    const title = chapterTitle(chapter);
    return this.#seek(chapter.start, () =>
      title === null ? this.#osd("Chapter: Chapter {}", [index + 1]) : this.#osd("Chapter: {}", [title]),
    );
  }
  jumpBookmark(id: number) {
    const bookmark = this.#bookmarks.get(id);
    if (!bookmark || this.jumpBlocked()) return;
    return this.#seek(bookmark.time, () => this.#osd("Bookmark: {}", [bookmark.title]));
  }

  /** Shift+PgUp / PgDn: chapters and bookmarks merged; Ctrl+PgUp / PgDn: chapters only. */
  step(direction: -1 | 1, chaptersOnly: boolean) {
    const position = this.#plugin.player.state.position;
    if (this.jumpBlocked() || position === null) return;
    const points: { time: number; jump: () => unknown }[] = this.#chapters.list.map((chapter, i) => ({
      time: chapter.start,
      jump: () => this.jumpChapter(i),
    }));
    if (!chaptersOnly)
      for (const bookmark of this.#bookmarks.items)
        points.push({ time: bookmark.time, jump: () => this.jumpBookmark(bookmark.id) });
    const time = stepPoint(
      points.map((point) => point.time),
      position,
      direction,
    );
    if (time === null)
      return this.#osd(
        chaptersOnly
          ? direction < 0
            ? "No previous chapter"
            : "No next chapter"
          : direction < 0
            ? "No previous bookmark or chapter"
            : "No next bookmark or chapter",
      );
    // A bookmark on a chapter start: the chapter's name is the better hint.
    return points.find((point) => point.time === time)!.jump();
  }

  /** P: a bookmark at the position (refused near an existing one). */
  add() {
    if (this.addBlocked()) return null;
    const result = this.#bookmarks.add(this.#plugin.player.state.position!);
    if (!result.ok) {
      this.#osd(result.reason, result.args);
      return null;
    }
    this.#osd("Bookmark added: {} ({})", [result.bookmark.title, clock(result.bookmark.time)]);
    return result.bookmark;
  }

  /** Chapter starts and bookmark times, for the B23 current range. */
  times(): number[] {
    return [...this.#chapters.list.map((chapter) => chapter.start), ...this.#bookmarks.items.map((item) => item.time)];
  }
}
