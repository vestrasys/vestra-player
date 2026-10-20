import type { Plugin } from "../../sdk/plugin/index.ts";
import type { Bookmarks } from "./bookmarks.ts";
import { chapterName, type Chapters, hasMedia, stepPoint } from "./chapters.ts";
import { clock } from "./format.ts";

/**
 * The B21 actions shared by the menu, the shortcuts and the list window:
 * jump to a chapter or bookmark, step between them, add a bookmark here.
 */
export class Actions {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #osd: (text: string) => void;

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#osd = osd;
  }

  /** Why jumping is unavailable; `null` when it can run. */
  jumpBlocked(): string | null {
    const state = this.#plugin.player.state;
    if (!hasMedia(state)) return "未打开媒体";
    if (!state.can.seek) return "当前来源不能跳转";
    return null;
  }
  /** Why adding a bookmark is unavailable; `null` when it can run. */
  addBlocked(): string | null {
    const state = this.#plugin.player.state;
    if (!hasMedia(state)) return "未打开媒体";
    if (state.phase !== "ready" || !state.can.seek || state.position === null) return "当前来源不能添加书签";
    return null;
  }

  /** Seek, conditional on the state seen now; a refusal because someone else just acted is dropped. */
  async #seek(time: number, text: string) {
    try {
      await this.#plugin.player.seek(time);
      this.#osd(text);
    } catch (error) {
      if ((error as { code?: string }).code !== "stale") this.#osd(`无法跳转：${(error as Error).message}`);
    }
  }

  jumpChapter(index: number) {
    const chapter = this.#chapters.list[index];
    if (!chapter || this.jumpBlocked()) return;
    return this.#seek(chapter.start, `章节定位：${chapterName(chapter, index)}`);
  }
  jumpBookmark(id: number) {
    const bookmark = this.#bookmarks.get(id);
    if (!bookmark || this.jumpBlocked()) return;
    return this.#seek(bookmark.time, `书签：定位到 ${bookmark.title}`);
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
    const what = chaptersOnly ? "章节" : "书签/章节";
    if (time === null) return this.#osd(direction < 0 ? `没有上一个${what}` : `没有下一个${what}`);
    // A bookmark on a chapter start: the chapter's name is the better hint.
    return points.find((point) => point.time === time)!.jump();
  }

  /** P: a bookmark at the position (refused near an existing one). */
  add() {
    if (this.addBlocked()) return null;
    const result = this.#bookmarks.add(this.#plugin.player.state.position!);
    if (!result.ok) {
      this.#osd(result.reason);
      return null;
    }
    this.#osd(`书签：${result.bookmark.title} 已添加（${clock(result.bookmark.time)}）`);
    return result.bookmark;
  }

  /** Chapter starts and bookmark times, for B23 当前区段. */
  times(): number[] {
    return [...this.#chapters.list.map((chapter) => chapter.start), ...this.#bookmarks.items.map((item) => item.time)];
  }
}
