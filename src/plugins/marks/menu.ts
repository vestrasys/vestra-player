import type { I18n, Plugin, Scope, Text } from "@vestra-player/sdk";
import type { Actions } from "./actions.ts";
import type { Bookmarks } from "./bookmarks.ts";
import { type Chapters, currentChapter } from "./chapters.ts";
import { clock, line } from "./format.ts";
import type { AbLoop } from "./loop.ts";
import { COUNTS, timing } from "./store.ts";

/** Shared submenus (English keys, doc/product/menus.md). */
export const MARKS = ["@playback", "Chapters and Bookmarks"] as const;
export const AB = ["@playback", "A-B Loop"] as const;
/**
 * Command palette categories (player 0.7.6): the submenu names, so their
 * translations are the menu's. Every command of a submenu shares one.
 */
export const MARKS_CATEGORY = MARKS[1];
export const AB_CATEGORY = AB[1];
export const COUNT_CATEGORY = "Loop Count";
/** The command that opens the Chapters and Bookmarks window: the `home` of what the window lists or adds. */
export const LIST_COMMAND = "marks.list";
/** Chapters and bookmarks listed in the menu; past this the last line opens the window. */
export const MENU_ITEMS = 50;

const two = (n: number) => String(n).padStart(2, "0");

/** Menu lines `01  Opening    00:00` (`01  Chapter 1    00:00` without a title), unique. */
export function chapterLines(text: I18n["text"], chapters: readonly ChapterLike[]): Text[] {
  const used = new Set<string>();
  return chapters.map((chapter, i) =>
    unique(text, used, `${two(i + 1)}  `, chapter.title?.trim() || { kind: "Chapter", number: i + 1 }, `    ${clock(chapter.start)}`),
  );
}
type ChapterLike = { title: string | null; start: number };
/** Menu lines `Bookmark title    01:23:45` (`Bookmark 1    01:23:45` without a title), unique. */
export function bookmarkLines(text: I18n["text"], bookmarks: readonly { title: string; number: number; time: number }[]): Text[] {
  const used = new Set<string>();
  return bookmarks.map((bookmark) =>
    unique(text, used, "", bookmark.title || { kind: "Bookmark", number: bookmark.number }, `    ${clock(bookmark.time)}`),
  );
}
/**
 * A menu line: an own title (data) shortened to fit the 80-byte path
 * segment, or "Chapter N" / "Bookmark N" in the interface language; a
 * repeated line gets ` (2)`. The menu tells lines apart by their English text.
 */
function unique(
  text: I18n["text"],
  used: Set<string>,
  prefix: string,
  name: string | { kind: "Chapter" | "Bookmark"; number: number },
  suffix: string,
): Text {
  const english = (more: string) =>
    typeof name === "string" ? line(prefix, name, more + suffix) : `${prefix}${name.kind} ${name.number}${more}${suffix}`;
  let more = "";
  for (let n = 2; used.has(english(more)); n++) more = ` (${n})`;
  used.add(english(more));
  // TRANSLATORS: a chapter or bookmark menu line made of the user's own title and its time; keep "{}".
  if (typeof name === "string") return text("{}", [english(more)]);
  const shown = name.kind === "Chapter" ? text("Chapter {}", [name.number]) : text("Bookmark {}", [name.number]);
  // TRANSLATORS: a menu line: chapter number ("01  ", or nothing for a bookmark), "Chapter N" / "Bookmark N", then "    01:23"; usually kept as is.
  return text("{}{}{}", [prefix, shown, more + suffix]);
}

/** Playback ▸ Chapters and Bookmarks ▸ previous / next (B21). */
const STEPS = [
  { id: "marks.previous", title: "Previous Bookmark or Chapter", order: 262, binding: "Shift+PageUp", direction: -1, chaptersOnly: false },
  { id: "marks.next", title: "Next Bookmark or Chapter", order: 263, binding: "Shift+PageDown", direction: 1, chaptersOnly: false },
  { id: "marks.previous-chapter", title: "Previous Chapter", order: 264, binding: "Ctrl+PageUp", direction: -1, chaptersOnly: true },
  { id: "marks.next-chapter", title: "Next Chapter", order: 265, binding: "Ctrl+PageDown", direction: 1, chaptersOnly: true },
] as const;

/**
 * Playback ▸ Chapters and Bookmarks (B21, orders 260–265; the chapters and
 * bookmarks 10000 + i / 20000 + i in a child scope rebuilt when they change)
 * and Playback ▸ A-B Loop (B23, orders 270–287).
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
    commands.add({
      id: LIST_COMMAND,
      path: [...MARKS, "Chapter and Bookmark List…"],
      category: MARKS_CATEGORY,
      order: 260,
      group: "marks-list",
      bindings: ["H"],
      run: () => this.#openList(),
    });
    commands.add({
      id: "marks.add",
      path: [...MARKS, "Add Bookmark"],
      category: MARKS_CATEGORY,
      home: LIST_COMMAND,
      order: 261,
      group: "marks-edit",
      bindings: ["P"],
      enabled: () => !actions.addBlocked(),
      reason: () => actions.addBlocked(),
      run: () => void actions.add(),
    });
    for (const step of STEPS)
      commands.add({
        id: step.id,
        path: [...MARKS, step.title],
        category: MARKS_CATEGORY,
        order: step.order,
        group: "marks-edit",
        bindings: [step.binding],
        enabled: () => !actions.jumpBlocked(),
        reason: () => actions.jumpBlocked(),
        run: () => actions.step(step.direction, step.chaptersOnly),
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
    const { text } = this.#plugin.i18n;
    const chapterTitles = chapterLines(text, chapters.slice(0, MENU_ITEMS));
    const bookmarkTitles = bookmarkLines(text, bookmarks.slice(0, MENU_ITEMS));
    const signature = JSON.stringify([chapterTitles, chapters.length, bookmarkTitles, bookmarks.map((b) => b.id)]);
    if (signature === this.#built || this.#disposed) return;
    this.#built = signature;
    const old = this.#scope;
    const scope = (this.#scope = this.#plugin.scope());
    if (old) await old.dispose();
    const { player } = this.#plugin;
    const actions = this.#actions;

    chapterTitles.forEach((title, i) =>
      scope.commands.add({
        id: `chapter.${i}`,
        path: [...MARKS, title],
        category: MARKS_CATEGORY,
        home: LIST_COMMAND,
        order: 10000 + i,
        group: "marks-chapters",
        radio: true,
        checked: () => currentChapter(this.#chapters.list, player.state.position) === i,
        enabled: () => !actions.jumpBlocked(),
        reason: () => actions.jumpBlocked(),
        run: () => actions.jumpChapter(i),
      }),
    );
    if (chapters.length > MENU_ITEMS)
      scope.commands.add({
        id: "chapter.more",
        path: [...MARKS, text("More Chapters… ({} in all)", [chapters.length])],
        category: MARKS_CATEGORY,
        order: 10000 + MENU_ITEMS,
        group: "marks-chapters",
        run: () => this.#openList(),
      });
    bookmarks.slice(0, MENU_ITEMS).forEach((bookmark, i) =>
      scope.commands.add({
        id: `bookmark.${i}`,
        path: [...MARKS, bookmarkTitles[i]!],
        category: MARKS_CATEGORY,
        home: LIST_COMMAND,
        order: 20000 + i,
        group: "marks-bookmarks",
        enabled: () => !actions.jumpBlocked(),
        reason: () => actions.jumpBlocked(),
        run: () => actions.jumpBookmark(bookmark.id),
      }),
    );
    if (bookmarks.length > MENU_ITEMS)
      scope.commands.add({
        id: "bookmark.more",
        path: [...MARKS, text("More Bookmarks… ({} in all)", [bookmarks.length])],
        category: MARKS_CATEGORY,
        order: 20000 + MENU_ITEMS,
        group: "marks-bookmarks",
        run: () => this.#openList(),
      });
  }
}

/** What an A-B command needs besides a usable source, and the reason shown without it. */
const MISSING = {
  a: "Start point not set",
  b: "End point not set",
  range: "A-B range not set",
} as const;

/** Playback ▸ A-B Loop ▸ Loop Count (B23), in `COUNTS` order. */
const COUNT_TITLES = ["Endless", "Once", "2 Times", "3 Times", "5 Times", "10 Times"] as const;

/** Playback ▸ A-B Loop (B23). */
export function addLoopCommands(plugin: Plugin, loop: AbLoop, actions: Actions) {
  const usable = () => loop.blocked();
  const hasA = () => loop.a !== null;
  const hasB = () => loop.b !== null;
  const has = { a: hasA, b: hasB, range: () => hasA() && hasB() };
  const items = [
    { id: "ab.set-a", title: "Set Start Point", order: 270, group: "ab-a", binding: "[", needs: null, run: () => loop.setA() },
    { id: "ab.clear-a", title: "Clear Start Point", order: 271, group: "ab-a", binding: "{", needs: "a", run: () => loop.clearA() },
    { id: "ab.a-back", title: "Start Point Back 0.1 s", order: 272, group: "ab-a", binding: "Ctrl+[", needs: "a", repeat: true, run: () => loop.nudgeA(-0.1) },
    { id: "ab.a-forward", title: "Start Point Forward 0.1 s", order: 273, group: "ab-a", binding: "Alt+[", needs: "a", repeat: true, run: () => loop.nudgeA(0.1) },
    { id: "ab.set-b", title: "Set End Point", order: 274, group: "ab-b", binding: "]", needs: null, run: () => loop.setB() },
    { id: "ab.clear-b", title: "Clear End Point", order: 275, group: "ab-b", binding: "}", needs: "b", run: () => loop.clearB() },
    { id: "ab.b-back", title: "End Point Back 0.1 s", order: 276, group: "ab-b", binding: "Ctrl+]", needs: "b", repeat: true, run: () => loop.nudgeB(-0.1) },
    { id: "ab.b-forward", title: "End Point Forward 0.1 s", order: 277, group: "ab-b", binding: "Alt+]", needs: "b", repeat: true, run: () => loop.nudgeB(0.1) },
    { id: "ab.back", title: "Move Range Back 0.1 s", order: 278, group: "ab-both", binding: "Ctrl+Alt+[", needs: "range", repeat: true, run: () => loop.shift(-0.1) },
    { id: "ab.forward", title: "Move Range Forward 0.1 s", order: 279, group: "ab-both", binding: "Ctrl+Alt+]", needs: "range", repeat: true, run: () => loop.shift(0.1) },
    { id: "ab.segment", title: "Loop Current Chapter or Bookmark Range", order: 280, group: "ab-toggle", binding: "Alt+\\", needs: null, run: () => loop.loopSegment(actions.times()) },
  ] as const;
  for (const item of items) {
    const ok = item.needs === null ? () => true : has[item.needs];
    plugin.commands.add({
      id: item.id,
      path: [...AB, item.title],
      category: AB_CATEGORY,
      order: item.order,
      group: item.group,
      bindings: [item.binding],
      ...("repeat" in item ? { repeat: item.repeat } : {}),
      enabled: () => !usable() && ok(),
      reason: () => usable() ?? (ok() || item.needs === null ? null : MISSING[item.needs]),
      run: item.run,
    });
  }
  // `\`: on while any loop runs (another plugin's too), so it can always stop one.
  const toggleable = () => (hasA() && hasB()) || !!plugin.player.state.loop;
  plugin.commands.add({
    id: "ab.toggle",
    path: [...AB, "A-B Loop On/Off"],
    category: AB_CATEGORY,
    order: 281,
    group: "ab-toggle",
    bindings: ["\\"],
    enabled: () => !usable() && toggleable(),
    reason: () => usable() ?? (toggleable() ? null : MISSING.range),
    checked: () => !!plugin.player.state.loop,
    run: () => loop.toggle(),
  });
  COUNTS.forEach((count, i) =>
    plugin.commands.add({
      id: `ab.count.${count ?? "endless"}`,
      path: [...AB, "Loop Count", COUNT_TITLES[i]!],
      category: COUNT_CATEGORY,
      order: 282 + i,
      group: "ab-count",
      radio: true,
      checked: () => loop.count === count,
      run: () => loop.setCount(count),
    }),
  );
}
