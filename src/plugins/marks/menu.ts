import type { I18n, Plugin, Scope, Text } from "@vestra-player/sdk";
import type { Actions } from "./actions.ts";
import type { Bookmarks } from "./bookmarks.ts";
import type { Chapters } from "./chapters.ts";
import { clock, line } from "./format.ts";
import { timing } from "./store.ts";

/**
 * Command palette categories (player 0.7.6): the names of the former Playback ▸
 * submenus, so their translations are kept. Every command of a former submenu
 * shares one. Only Chapter and Bookmark List… is in the menu (Tools ▸, user
 * ruling 2026-11-11); everything else is in the command palette only.
 */
export const MARKS_CATEGORY = "Chapters and Bookmarks";
/** The command that opens the Chapters and Bookmarks window: the `home` of what the window lists or adds. */
export const LIST_COMMAND = "marks.list";
/** Chapters and bookmarks registered as commands; past this the last line opens the window. */
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

/** Chapters and Bookmarks: previous / next (B21). */
const STEPS = [
  { id: "marks.previous", title: "Previous Bookmark or Chapter", order: 262, binding: "Shift+PageUp", direction: -1, chaptersOnly: false },
  { id: "marks.next", title: "Next Bookmark or Chapter", order: 263, binding: "Shift+PageDown", direction: 1, chaptersOnly: false },
  { id: "marks.previous-chapter", title: "Previous Chapter", order: 264, binding: "Ctrl+PageUp", direction: -1, chaptersOnly: true },
  { id: "marks.next-chapter", title: "Next Chapter", order: 265, binding: "Ctrl+PageDown", direction: 1, chaptersOnly: true },
] as const;

/**
 * Chapters and Bookmarks (B21: Chapter and Bookmark List… in Tools ▸, order
 * 801 in its own marks group; the rest palette only, orders
 * 261–265; the chapters and bookmarks 10000 + i / 20000 + i in a child scope
 * rebuilt when they change). A-B Loop (B23, orders 270–287) moved to the playback plugin.
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
  /** The chapter playing (the "chapter" trigger keeps it; -1: none). */
  readonly #facts: { readonly chapter: number };

  constructor(
    plugin: Plugin,
    chapters: Chapters,
    bookmarks: Bookmarks,
    actions: Actions,
    openList: () => unknown,
    facts: { readonly chapter: number },
  ) {
    this.#plugin = plugin;
    this.#facts = facts;
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
      path: ["@tools", "Chapter and Bookmark List…"],
      category: MARKS_CATEGORY,
      order: 801,
      group: "marks",
      bindings: ["H"],
      run: () => this.#openList(),
    });
    commands.add({
      id: "marks.add",
      title: "Add Bookmark",
      category: MARKS_CATEGORY,
      home: LIST_COMMAND,
      order: 261,
      group: "marks-edit",
      bindings: ["P"],
      enabled: actions.canAdd,
      reason: () => actions.addReason(),
      run: () => void actions.add(),
    });
    for (const step of STEPS)
      commands.add({
        id: step.id,
        title: step.title,
        category: MARKS_CATEGORY,
        order: step.order,
        group: "marks-edit",
        bindings: [step.binding],
        enabled: actions.canJump,
        reason: () => actions.jumpReason(),
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
    const actions = this.#actions;
    const facts = this.#facts;

    chapterTitles.forEach((title, i) =>
      scope.commands.add({
        id: `chapter.${i}`,
        title,
        category: MARKS_CATEGORY,
        home: LIST_COMMAND,
        order: 10000 + i,
        group: "marks-chapters",
        radio: true,
        checked: () => facts.chapter === i,
        enabled: actions.canJump,
        reason: () => actions.jumpReason(),
        run: () => actions.jumpChapter(i),
      }),
    );
    if (chapters.length > MENU_ITEMS)
      scope.commands.add({
        id: "chapter.more",
        title: text("More Chapters… ({} in all)", [chapters.length]),
        category: MARKS_CATEGORY,
        order: 10000 + MENU_ITEMS,
        group: "marks-chapters",
        run: () => this.#openList(),
      });
    bookmarks.slice(0, MENU_ITEMS).forEach((bookmark, i) =>
      scope.commands.add({
        id: `bookmark.${i}`,
        title: bookmarkTitles[i]!,
        category: MARKS_CATEGORY,
        home: LIST_COMMAND,
        order: 20000 + i,
        group: "marks-bookmarks",
        enabled: actions.canJump,
        reason: () => actions.jumpReason(),
        run: () => actions.jumpBookmark(bookmark.id),
      }),
    );
    if (bookmarks.length > MENU_ITEMS)
      scope.commands.add({
        id: "bookmark.more",
        title: text("More Bookmarks… ({} in all)", [bookmarks.length]),
        category: MARKS_CATEGORY,
        order: 20000 + MENU_ITEMS,
        group: "marks-bookmarks",
        run: () => this.#openList(),
      });
  }
}
