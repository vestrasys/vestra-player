import type { Plugin } from "../../sdk/plugin/index.ts";
import type { Actions } from "./actions.ts";
import type { Bookmarks } from "./bookmarks.ts";
import { chapterName, type Chapters, hasMedia } from "./chapters.ts";
import { parseTime, precise } from "./format.ts";

/** One list row; `rowId` is `c<index>` for a chapter, `b<id>` for a bookmark. */
type Row = { rowId: string; chapter: boolean; time: string; title: string; selected: boolean };

/**
 * B21 「章节/书签」 window (H): chapters (read only) and bookmarks by time,
 * filtered by the search box; double-click jumps; a selected bookmark can be
 * renamed, moved and removed; 清空书签 asks first. It follows the current
 * media; a new source drops the selection and unsaved edits.
 */
export class ListWindow {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #actions: Actions;
  readonly #window;
  #selected: string | null = null;
  #filter = "";
  #status = "";
  #rows: { key: string; rows: Row[] } = { key: "", rows: [] };

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, actions: Actions) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#actions = actions;
    const window = (this.#window = plugin.windows.create("ui/marks.slint"));
    window.on("select", (rowId: string) => this.#select(rowId));
    window.on("activate", (rowId: string) => this.#activate(rowId));
    window.on("search", (text: string) => {
      this.#filter = text.trim().toLowerCase();
    });
    window.on("commitTitle", (text: string) => this.#commitTitle(text));
    window.on("commitTime", (text: string) => this.#commitTime(text));
    window.on("add", () => this.#add());
    window.on("remove", () => this.#remove());
    window.on("clear", () => this.#clear());
    window.bind(() => {
      const bookmark = this.#bookmark();
      const state = plugin.player.state;
      const media = hasMedia(state);
      return {
        rows: this.#rowList(),
        media,
        note: !media
          ? "未打开媒体"
          : bookmarks.fileChanged
            ? "文件已改变，书签时间可能不准"
            : "",
        filtering: this.#filter !== "",
        canAdd: !actions.addBlocked(),
        canRemove: !!bookmark,
        canClear: bookmarks.items.length > 0,
        editing: !!bookmark,
        status: this.#status,
      };
    });
    // Another file: forget the selection and whatever was typed.
    let media = bookmarks.media;
    bookmarks.onChange(() => {
      if (bookmarks.media === media) return;
      media = bookmarks.media;
      this.#selected = null;
      this.#status = "";
      this.#loadEdit();
    });
    chapters.onChange(() => {
      if (this.#selected?.startsWith("c")) this.#selected = null;
    });
  }

  /** H: open, or bring the open window to the front. */
  async show() {
    await this.#window.show();
  }

  #bookmark() {
    if (!this.#selected?.startsWith("b")) return undefined;
    return this.#bookmarks.get(Number(this.#selected.slice(1)));
  }

  /** Rows are rebuilt only when what they show changed (not on every position update). */
  #rowList(): Row[] {
    const key = `${this.#chapters.version}/${this.#bookmarks.version}/${this.#filter}/${this.#selected}`;
    if (key === this.#rows.key) return this.#rows.rows;
    const all: (Row & { at: number; order: number })[] = [
      ...this.#chapters.list.map((chapter, i) => ({
        rowId: `c${i}`,
        chapter: true,
        at: chapter.start,
        order: 0,
        time: precise(chapter.start),
        title: chapterName(chapter, i),
        selected: false,
      })),
      ...this.#bookmarks.items.map((bookmark) => ({
        rowId: `b${bookmark.id}`,
        chapter: false,
        at: bookmark.time,
        order: 1,
        time: precise(bookmark.time),
        title: bookmark.title,
        selected: false,
      })),
    ];
    const rows = all
      .filter((row) => !this.#filter || row.title.toLowerCase().includes(this.#filter))
      .sort((x, y) => x.at - y.at || x.order - y.order)
      .map(({ rowId, chapter, time, title }) => ({ rowId, chapter, time, title, selected: rowId === this.#selected }));
    this.#rows = { key, rows };
    return rows;
  }

  /** Put the selected bookmark's values into the two inputs (always sent: they may hold typing). */
  #loadEdit() {
    const bookmark = this.#bookmark();
    this.#window.set({
      editTime: bookmark ? precise(bookmark.time, true) : "",
      editTitle: bookmark ? bookmark.title : "",
    });
  }

  #select(rowId: string) {
    if (rowId === this.#selected) return;
    this.#selected = rowId;
    this.#status = "";
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
    if (!this.#bookmarks.rename(bookmark.id, text)) this.#status = "标题不能为空";
    else this.#status = "";
    this.#loadEdit();
  }
  #commitTime(text: string) {
    const bookmark = this.#bookmark();
    if (!bookmark) return;
    const time = parseTime(text);
    const duration = this.#plugin.player.state.duration;
    if (time === null) this.#status = "时间格式为 h:mm:ss.d";
    else if (duration !== null && time > duration) this.#status = `时间超出时长（${precise(duration)}）`;
    else {
      this.#bookmarks.retime(bookmark.id, time);
      this.#status = "";
    }
    this.#loadEdit();
  }

  #add() {
    const bookmark = this.#actions.add();
    if (!bookmark) return;
    this.#filter = "";
    this.#window.set({ searchText: "" });
    this.#selected = `b${bookmark.id}`;
    this.#status = "";
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
    this.#status = `已移除：${bookmark.title}`;
    this.#loadEdit();
  }
  async #clear() {
    const count = this.#bookmarks.items.length;
    if (!count) return;
    const media = this.#bookmarks.media;
    let answer;
    try {
      answer = await this.#plugin.dialogs.confirm({
        message: "清空当前文件的全部书签？",
        detail: `将移除 ${count} 个书签，不能撤销。`,
        confirm: "清空书签",
        destructive: true,
      });
    } catch (error) {
      this.#status = `无法确认：${(error as Error).message}`;
      return;
    }
    // The answer is for the file it was asked about.
    if (!answer.confirmed || this.#bookmarks.media !== media) return;
    this.#bookmarks.clear();
    if (this.#selected?.startsWith("b")) this.#selected = null;
    this.#status = `已清空 ${count} 个书签`;
    this.#loadEdit();
  }
}
