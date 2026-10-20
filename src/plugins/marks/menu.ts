import type { CommandOptions, Plugin, Scope } from "../../sdk/plugin/index.ts";
import type { Actions } from "./actions.ts";
import type { Bookmarks } from "./bookmarks.ts";
import { chapterName, type Chapters, currentChapter } from "./chapters.ts";
import { clock, line } from "./format.ts";
import type { AbLoop } from "./loop.ts";
import { COUNTS, timing } from "./store.ts";

export const MARKS = ["播放", "章节/书签"];
export const AB = ["播放", "A-B 区段循环"];
/** Chapters and bookmarks listed in the menu; past this the last line opens the window. */
export const MENU_ITEMS = 50;

const two = (n: number) => String(n).padStart(2, "0");

/** Menu lines `01  Opening    00:00`, unique (a repeated line gets ` (2)`). */
export function chapterLines(chapters: ChapterLike[]): string[] {
  const used = new Set<string>();
  return chapters.map((chapter, i) => unique(used, `${two(i + 1)}  `, chapterName(chapter, i), `    ${clock(chapter.start)}`));
}
type ChapterLike = { title: string | null; start: number };
/** Menu lines `书签标题    01:23:45`, unique. */
export function bookmarkLines(bookmarks: readonly { title: string; time: number }[]): string[] {
  const used = new Set<string>();
  return bookmarks.map((bookmark) => unique(used, "", bookmark.title, `    ${clock(bookmark.time)}`));
}
function unique(used: Set<string>, prefix: string, text: string, suffix: string) {
  let title = line(prefix, text, suffix);
  for (let n = 2; used.has(title); n++) title = line(prefix, text, ` (${n})${suffix}`);
  used.add(title);
  return title;
}

/**
 * 播放 ▶ 章节/书签 (B21, orders 260–265; the chapters and bookmarks
 * 10000 + i / 20000 + i in a child scope rebuilt when they change) and
 * 播放 ▶ A-B 区段循环 (B23, orders 270–287).
 */
export class MarksMenu {
  readonly #plugin: Plugin;
  readonly #chapters: Chapters;
  readonly #bookmarks: Bookmarks;
  readonly #actions: Actions;
  readonly #openList: () => unknown;
  #scope: Scope | null = null;
  #built: string | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #building: Promise<void> = Promise.resolve();
  #disposed = false;

  constructor(plugin: Plugin, chapters: Chapters, bookmarks: Bookmarks, actions: Actions, openList: () => unknown) {
    this.#plugin = plugin;
    this.#chapters = chapters;
    this.#bookmarks = bookmarks;
    this.#actions = actions;
    this.#openList = openList;
    chapters.onChange(() => this.schedule());
    bookmarks.onChange(() => this.schedule());
    plugin.onDispose(() => {
      this.#disposed = true;
      if (this.#timer) clearTimeout(this.#timer);
    });
    this.#addFixed();
  }

  #addFixed() {
    const { commands } = this.#plugin;
    const actions = this.#actions;
    const jump = { enabled: () => !actions.jumpBlocked(), reason: () => actions.jumpBlocked() };
    commands.add({
      id: "marks.list",
      path: [...MARKS, "章节/书签列表…"],
      order: 260,
      group: "marks-list",
      bindings: ["H"],
      run: () => this.#openList(),
    });
    commands.add({
      id: "marks.add",
      path: [...MARKS, "添加书签"],
      order: 261,
      group: "marks-edit",
      bindings: ["P"],
      enabled: () => !actions.addBlocked(),
      reason: () => actions.addBlocked(),
      run: () => void actions.add(),
    });
    const steps: [string, string, number, string, -1 | 1, boolean][] = [
      ["marks.previous", "上一书签/章节", 262, "Shift+PageUp", -1, false],
      ["marks.next", "下一书签/章节", 263, "Shift+PageDown", 1, false],
      ["marks.previous-chapter", "上一章节", 264, "Ctrl+PageUp", -1, true],
      ["marks.next-chapter", "下一章节", 265, "Ctrl+PageDown", 1, true],
    ];
    for (const [id, title, order, binding, direction, chaptersOnly] of steps)
      commands.add({
        id,
        path: [...MARKS, title],
        order,
        group: "marks-edit",
        bindings: [binding],
        ...jump,
        run: () => actions.step(direction, chaptersOnly),
      });
  }

  /** Rebuild after `timing.menuRebuild`; further changes meanwhile join that rebuild. */
  schedule() {
    if (this.#timer || this.#disposed) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#building = this.#building.then(() => this.build());
    }, timing.menuRebuild);
  }

  /** The chapter and bookmark lines; nothing happens when no line changed. */
  async build() {
    const chapters = this.#chapters.list;
    const bookmarks = this.#bookmarks.items;
    const chapterTitles = chapterLines(chapters.slice(0, MENU_ITEMS));
    const bookmarkTitles = bookmarkLines(bookmarks.slice(0, MENU_ITEMS));
    const signature = JSON.stringify([chapterTitles, chapters.length, bookmarkTitles, bookmarks.map((b) => b.id)]);
    if (signature === this.#built || this.#disposed) return;
    this.#built = signature;
    const old = this.#scope;
    const scope = (this.#scope = this.#plugin.scope());
    if (old) await old.dispose();
    const add = (options: Omit<CommandOptions, "run" | "path"> & { title: string; run: () => unknown }) =>
      scope.commands.add({ ...options, path: [...MARKS, options.title] });
    const { player } = this.#plugin;
    const jump = { enabled: () => !this.#actions.jumpBlocked(), reason: () => this.#actions.jumpBlocked() };

    chapterTitles.forEach((title, i) =>
      add({
        id: `chapter.${i}`,
        title,
        order: 10000 + i,
        group: "marks-chapters",
        radio: true,
        checked: () => currentChapter(this.#chapters.list, player.state.position) === i,
        ...jump,
        run: () => this.#actions.jumpChapter(i),
      }),
    );
    if (chapters.length > MENU_ITEMS)
      add({
        id: "chapter.more",
        title: `更多章节…（共 ${chapters.length} 项）`,
        order: 10000 + MENU_ITEMS,
        group: "marks-chapters",
        run: () => this.#openList(),
      });
    bookmarks.slice(0, MENU_ITEMS).forEach((bookmark, i) =>
      add({
        id: `bookmark.${i}`,
        title: bookmarkTitles[i]!,
        order: 20000 + i,
        group: "marks-bookmarks",
        ...jump,
        run: () => this.#actions.jumpBookmark(bookmark.id),
      }),
    );
    if (bookmarks.length > MENU_ITEMS)
      add({
        id: "bookmark.more",
        title: `更多书签…（共 ${bookmarks.length} 项）`,
        order: 20000 + MENU_ITEMS,
        group: "marks-bookmarks",
        run: () => this.#openList(),
      });
  }
}

/** 播放 ▶ A-B 区段循环 (B23). */
export function addLoopCommands(plugin: Plugin, loop: AbLoop, actions: Actions) {
  const usable = () => loop.blocked();
  const needs = (ok: () => boolean, missing: string) => ({
    enabled: () => !usable() && ok(),
    reason: () => usable() ?? (ok() ? null : missing),
  });
  const hasA = () => loop.a !== null;
  const hasB = () => loop.b !== null;
  const any = { enabled: () => !usable(), reason: usable };
  const items: [string, string, number, string, string, Partial<CommandOptions>, () => unknown][] = [
    ["ab.set-a", "设定起点", 270, "ab-a", "[", any, () => loop.setA()],
    ["ab.clear-a", "解除起点", 271, "ab-a", "{", needs(hasA, "未设定起点"), () => loop.clearA()],
    ["ab.a-back", "起点后退 0.1 秒", 272, "ab-a", "Ctrl+[", { ...needs(hasA, "未设定起点"), repeat: true }, () => loop.nudgeA(-0.1)],
    ["ab.a-forward", "起点前进 0.1 秒", 273, "ab-a", "Alt+[", { ...needs(hasA, "未设定起点"), repeat: true }, () => loop.nudgeA(0.1)],
    ["ab.set-b", "设定止点", 274, "ab-b", "]", any, () => loop.setB()],
    ["ab.clear-b", "解除止点", 275, "ab-b", "}", needs(hasB, "未设定止点"), () => loop.clearB()],
    ["ab.b-back", "止点后退 0.1 秒", 276, "ab-b", "Ctrl+]", { ...needs(hasB, "未设定止点"), repeat: true }, () => loop.nudgeB(-0.1)],
    ["ab.b-forward", "止点前进 0.1 秒", 277, "ab-b", "Alt+]", { ...needs(hasB, "未设定止点"), repeat: true }, () => loop.nudgeB(0.1)],
    ["ab.back", "区段整体后退 0.1 秒", 278, "ab-both", "Ctrl+Alt+[", { ...needs(() => hasA() && hasB(), "未设定区段"), repeat: true }, () => loop.shift(-0.1)],
    ["ab.forward", "区段整体前进 0.1 秒", 279, "ab-both", "Ctrl+Alt+]", { ...needs(() => hasA() && hasB(), "未设定区段"), repeat: true }, () => loop.shift(0.1)],
    ["ab.segment", "循环当前章节/书签区段", 280, "ab-toggle", "Alt+\\", any, () => loop.loopSegment(actions.times())],
    [
      "ab.toggle",
      "区段循环 开/关",
      281,
      "ab-toggle",
      "\\",
      { ...needs(() => (hasA() && hasB()) || !!plugin.player.state.loop, "未设定区段"), checked: () => !!plugin.player.state.loop },
      () => loop.toggle(),
    ],
  ];
  for (const [id, title, order, group, binding, options, run] of items)
    plugin.commands.add({ id, path: [...AB, title], order, group, bindings: [binding], ...options, run });
  COUNTS.forEach((count, i) =>
    plugin.commands.add({
      id: `ab.count.${count ?? "endless"}`,
      path: [...AB, "循环次数", count === null ? "无限" : `${count} 次`],
      order: 282 + i,
      group: "ab-count",
      radio: true,
      checked: () => loop.count === count,
      run: () => loop.setCount(count),
    }),
  );
}
