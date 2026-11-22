import type { Condition, Plugin, PlayerState } from "@vestra-player/sdk";
import { type Bookmarks, bookmarkName } from "./bookmarks.ts";
import { type Chapters, chapterTitle, hasMedia, stepPoint } from "./chapters.ts";
import { clock } from "./format.ts";
import type { Osd } from "./osd.ts";

/** Why a chapter/bookmark command is unavailable (shown as the command's reason). */
export type JumpBlocked = "No media open" | "This source can't seek";
export type AddBlocked = "No media open" | "Can't add bookmarks to this source";

/** What the "media" trigger keeps: a media source is open (`hasMedia`). */
export type MediaFact = { readonly media: boolean };

/** Why jumping is unavailable in `state`; `null` when it can run. */
export function jumpBlocked(state: PlayerState): JumpBlocked | null {
  if (!hasMedia(state)) return "No media open";
  if (!state.can.seek) return "This source can't seek";
  return null;
}
/** Why adding a bookmark is unavailable in `state`; `null` when it can run. */
export function addBlocked(state: PlayerState): AddBlocked | null {
  if (!hasMedia(state)) return "No media open";
  if (state.phase !== "ready" || !state.can.seek || state.position === null) return "Can't add bookmarks to this source";
  return null;
}

/**
 * The B21 actions shared by the menu, the shortcuts and the list window:
 * jump to a chapter or bookmark, step between them, add a bookmark here.
 */
export class Actions {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #osd: Osd;
  readonly #facts: MediaFact;
  /** A jump can run (the player evaluates it): `jumpBlocked` is `null`. */
  readonly canJump: Condition;
  /** A bookmark can be added: `addBlocked` is `null` (a ready source has a position). */
  readonly canAdd: Condition;

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, osd: Osd, facts: MediaFact) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#osd = osd;
    this.#facts = facts;
    const { when } = plugin;
    this.canJump = when.phase("ready", "ended").and(when.stream.not(), when.can("seek"));
    this.canAdd = when.ready.and(when.stream.not(), when.can("seek"));
  }

  /** The reason a jump command shows while `canJump` is false. */
  jumpReason(): JumpBlocked {
    return this.#facts.media ? "This source can't seek" : "No media open";
  }
  /** The reason Add Bookmark shows while `canAdd` is false. */
  addReason(): AddBlocked {
    return this.#facts.media ? "Can't add bookmarks to this source" : "No media open";
  }

  /** The state now: a command runs on what the player shows, not on the last snapshot. */
  async #current(): Promise<PlayerState> {
    return this.#plugin.player.refresh().catch(() => this.#plugin.player.state);
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

  /** `state`: the state the caller follows (the open window's stream); read now without one. */
  async jumpChapter(index: number, state?: PlayerState) {
    return this.#jumpChapter(state ?? (await this.#current()), index);
  }
  async jumpBookmark(id: number, state?: PlayerState) {
    return this.#jumpBookmark(state ?? (await this.#current()), id);
  }
  #jumpChapter(state: PlayerState, index: number) {
    const chapter = this.#chapters.list[index];
    if (!chapter || jumpBlocked(state)) return;
    const title = chapterTitle(chapter);
    return this.#seek(chapter.start, () =>
      title === null ? this.#osd("Chapter: Chapter {}", [index + 1]) : this.#osd("Chapter: {}", [title]),
    );
  }
  #jumpBookmark(state: PlayerState, id: number) {
    const bookmark = this.#bookmarks.get(id);
    if (!bookmark || jumpBlocked(state)) return;
    return this.#seek(bookmark.time, () => this.#osd("Bookmark: {}", [bookmarkName(this.#plugin.i18n.text, bookmark)]));
  }

  /** Shift+PgUp / PgDn: chapters and bookmarks merged; Ctrl+PgUp / PgDn: chapters only. */
  async step(direction: -1 | 1, chaptersOnly: boolean) {
    const state = await this.#current();
    const position = state.position;
    if (jumpBlocked(state) || position === null) return;
    const points: { time: number; jump: () => unknown }[] = this.#chapters.list.map((chapter, i) => ({
      time: chapter.start,
      jump: () => this.#jumpChapter(state, i),
    }));
    if (!chaptersOnly)
      for (const bookmark of this.#bookmarks.items)
        points.push({ time: bookmark.time, jump: () => this.#jumpBookmark(state, bookmark.id) });
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

  /** P: a bookmark at the position (refused near an existing one); `state` as for `jumpChapter`. */
  async add(state?: PlayerState) {
    state ??= await this.#current();
    if (addBlocked(state)) return null;
    const result = this.#bookmarks.add(state.position!);
    if (!result.ok) {
      this.#osd(result.reason, result.args);
      return null;
    }
    this.#osd("Bookmark added: {} ({})", [bookmarkName(this.#plugin.i18n.text, result.bookmark), clock(result.bookmark.time)]);
    return result.bookmark;
  }
}
