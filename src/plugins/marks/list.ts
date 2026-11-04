import type { Plugin, Text } from "@vestra-player/sdk";
import type { Actions } from "./actions.ts";
import { type Bookmarks, bookmarkName, cleanTitle } from "./bookmarks.ts";
import { type Chapters, chapterTitle, hasMedia } from "./chapters.ts";
import { parseTime, precise } from "./format.ts";

/**
 * One list row; `rowId` is `c<index>` for a chapter, `b<id>` for a bookmark.
 * A chapter or bookmark without a title has `title: ""` and is shown as
 * "Chapter <number>" / "Bookmark <number>".
 */
type Row = { rowId: string; chapter: boolean; time: string; title: string; number: number; selected: boolean };

/**
 * The line under the list after an edit (the window words it): `text` and
 * `count` are its data.
 */
type Status = {
  kind: "" | "empty-title" | "bad-time" | "too-late" | "removed" | "confirm-failed" | "cleared";
  text?: string | Text;
  count?: number;
};
const NO_STATUS: Status = { kind: "" };

/** `"书签 {}"`, 3 → `"书签 3"`. */
const named = (name: string, number: number) => name.replace("{}", String(number));

/**
 * B21 Chapters and Bookmarks window (H): chapters (read only) and bookmarks
 * by time, filtered by the search box; double-click jumps; a selected
 * bookmark can be renamed, moved and removed; Clear Bookmarks asks first.
 * It follows the current media; a new source drops the selection and
 * unsaved edits.
 */
export class ListWindow {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #actions: Actions;
  readonly #marksWindow;
  #selected: string | null = null;
  #filter = "";
  /** How the window names an untitled chapter / bookmark, `{}` for the number (it sends them with the search). */
  #names = { chapter: "Chapter {}", bookmark: "Bookmark {}" };
  #status: Status = NO_STATUS;
  #rows: { key: string; rows: Row[] } = { key: "", rows: [] };

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, actions: Actions) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#actions = actions;
    const marksWindow = (this.#marksWindow = plugin.windows.create("ui/marks.slint"));
    marksWindow.on("select", (rowId: string) => this.#select(rowId));
    marksWindow.on("activate", (rowId: string) => this.#activate(rowId));
    marksWindow.on("search", (text: string, chapter: string, bookmark: string) => {
      this.#filter = text.trim().toLowerCase();
      // The window's translated names for an untitled row (`@tr` in marks.slint).
      if (typeof chapter === "string" && chapter.includes("{}")) this.#names.chapter = chapter;
      if (typeof bookmark === "string" && bookmark.includes("{}")) this.#names.bookmark = bookmark;
    });
    marksWindow.on("commitTitle", (text: string) => this.#commitTitle(text));
    marksWindow.on("commitTime", (text: string) => this.#commitTime(text));
    marksWindow.on("add", () => this.#add());
    marksWindow.on("remove", () => this.#remove());
    marksWindow.on("clear", () => this.#clear());
    marksWindow.bind(() => {
      const bookmark = this.#bookmark();
      const state = plugin.player.state;
      const media = hasMedia(state);
      return {
        rows: this.#rowList(),
        media,
        fileChanged: media && bookmarks.fileChanged,
        filtering: this.#filter !== "",
        canAdd: !actions.addBlocked(),
        canRemove: !!bookmark,
        canClear: bookmarks.items.length > 0,
        editing: !!bookmark,
        status: this.#status.kind,
        statusText: this.#status.text ?? "",
        statusCount: this.#status.count ?? 0,
      };
    });
    // Another file: forget the selection and whatever was typed.
    let media = bookmarks.media;
    bookmarks.onChange(() => {
      if (bookmarks.media === media) return;
      media = bookmarks.media;
      this.#selected = null;
      this.#status = NO_STATUS;
      this.#loadEdit();
    });
    chapters.onChange(() => {
      if (this.#selected?.startsWith("c")) this.#selected = null;
    });
  }

  /** H: open, or bring the open window to the front. */
  async show() {
    await this.#marksWindow.show();
  }

  #bookmark() {
    if (!this.#selected?.startsWith("b")) return undefined;
    return this.#bookmarks.get(Number(this.#selected.slice(1)));
  }

  /** Rows are rebuilt only when what they show changed (not on every position update). */
  #rowList(): Row[] {
    const key = `${this.#chapters.version}/${this.#bookmarks.version}/${this.#filter}/${this.#selected}`;
    if (key === this.#rows.key) return this.#rows.rows;
    const all: (Row & { at: number; order: number; match: string })[] = [
      ...this.#chapters.list.map((chapter, i) => ({
        rowId: `c${i}`,
        chapter: true,
        at: chapter.start,
        order: 0,
        time: precise(chapter.start),
        title: chapterTitle(chapter) ?? "",
        number: i + 1,
        // An untitled row matches by the name the window shows ("Chapter N" in English).
        match: chapterTitle(chapter) ?? named(this.#names.chapter, i + 1),
        selected: false,
      })),
      ...this.#bookmarks.items.map((bookmark) => ({
        rowId: `b${bookmark.id}`,
        chapter: false,
        at: bookmark.time,
        order: 1,
        time: precise(bookmark.time),
        title: bookmark.title,
        number: bookmark.number,
        match: bookmark.title || named(this.#names.bookmark, bookmark.number),
        selected: false,
      })),
    ];
    const rows = all
      .filter((row) => !this.#filter || row.match.toLowerCase().includes(this.#filter))
      .sort((x, y) => x.at - y.at || x.order - y.order)
      .map(({ rowId, chapter, time, title, number }) => ({
        rowId,
        chapter,
        time,
        title,
        number,
        selected: rowId === this.#selected,
      }));
    this.#rows = { key, rows };
    return rows;
  }

  /** Put the selected bookmark's values into the two inputs (always sent: they may hold typing). */
  #loadEdit() {
    const bookmark = this.#bookmark();
    this.#marksWindow.set({
      editTime: bookmark ? precise(bookmark.time, true) : "",
      editTitle: bookmark ? bookmark.title : "",
      editNumber: bookmark && !bookmark.title ? bookmark.number : 0,
    });
  }

  #select(rowId: string) {
    if (rowId === this.#selected) return;
    this.#selected = rowId;
    this.#status = NO_STATUS;
    this.#loadEdit();
  }
  #activate(rowId: string) {
    this.#select(rowId);
    if (rowId.startsWith("c")) return this.#actions.jumpChapter(Number(rowId.slice(1)));
    return this.#actions.jumpBookmark(Number(rowId.slice(1)));
  }

  #commitTitle(text: string) {
    const bookmark = this.#bookmark();
    if (!bookmark) return;
    // Left empty, a bookmark without a title keeps none.
    if (!bookmark.title && cleanTitle(text) === null) this.#status = NO_STATUS;
    else if (!this.#bookmarks.rename(bookmark.id, text)) this.#status = { kind: "empty-title" };
    else this.#status = NO_STATUS;
    this.#loadEdit();
  }
  #commitTime(text: string) {
    const bookmark = this.#bookmark();
    if (!bookmark) return;
    const time = parseTime(text);
    const duration = this.#plugin.player.state.duration;
    if (time === null) this.#status = { kind: "bad-time" };
    else if (duration !== null && time > duration) this.#status = { kind: "too-late", text: precise(duration) };
    else {
      this.#bookmarks.retime(bookmark.id, time);
      this.#status = NO_STATUS;
    }
    this.#loadEdit();
  }

  #add() {
    const bookmark = this.#actions.add();
    if (!bookmark) return;
    this.#filter = "";
    this.#marksWindow.set({ searchText: "" });
    this.#selected = `b${bookmark.id}`;
    this.#status = NO_STATUS;
    this.#loadEdit();
  }
  #remove() {
    const bookmark = this.#bookmark();
    if (!bookmark) return;
    // Keep the place in the list: select the next bookmark (or the one before).
    const items = this.#bookmarks.items;
    const index = items.findIndex((item) => item.id === bookmark.id);
    const next = items[index + 1] ?? items[index - 1];
    this.#bookmarks.remove(bookmark.id);
    this.#selected = next ? `b${next.id}` : null;
    this.#status = { kind: "removed", text: bookmarkName(this.#plugin.i18n.text, bookmark) };
    this.#loadEdit();
  }
  async #clear() {
    const count = this.#bookmarks.items.length;
    if (!count) return;
    const media = this.#bookmarks.media;
    let answer;
    try {
      answer = await this.#plugin.dialogs.confirm({
        message: "Clear all bookmarks of this file?",
        detail:
          count === 1
            ? "{} bookmark will be removed. This can't be undone."
            : "{} bookmarks will be removed. This can't be undone.",
        confirm: "Clear Bookmarks",
        destructive: true,
        args: [count],
      });
    } catch (error) {
      this.#status = { kind: "confirm-failed", text: (error as Error).message };
      return;
    }
    // The answer is for the file it was asked about.
    if (!answer.confirmed || this.#bookmarks.media !== media) return;
    this.#bookmarks.clear();
    if (this.#selected?.startsWith("b")) this.#selected = null;
    this.#status = { kind: "cleared", count };
    this.#loadEdit();
  }
}
