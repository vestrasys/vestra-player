import type { I18n, Osd, Plugin, Scope, Text } from "@vestra-player/sdk";
import { bytes, clock, displayName, menuText, middleBytes } from "./identity.ts";
import { MENU_ITEMS, present, type Recent, type RecentItem } from "./recent.ts";
import type { Mode, Resume } from "./resume.ts";
import { timing } from "./store.ts";

const RECENT = ["@open", "Recently Played"] as const;
/** All of Open ▸ Recently Played shares one order; declaration order arranges it (SDK: Concepts, Commands, "Order and separators"). */
const RECENT_ORDER = 114;
/** Headless hosts refuse the display; that never fails the action itself. */
export const quiet = (shown: Promise<void>) => void shown.catch(() => {});

/** The player's limit on a menu path segment (a menu line), in UTF-8 bytes. */
export const SEGMENT_BYTES = 80;
/**
 * The most `{} ({})` adds around the name and the point, in bytes: the player
 * checks (and keys the command by) the English rendering, ` (` and `)` (3);
 * Chinese and Japanese show `{}（{}）` (6). A line fits in every language.
 */
export const POINT_BYTES = 6;

/** The English titles of the fixed items of Open ▸ Recently Played (the literals stay in each `commands.add` for `i18n extract`). */
const FIXED_TITLES = ["(No Entries)", "Remove Invalid Entries", "Clear Entries", "Record Playback History"];

/** `1`–`9`, `0`, then nothing: the number in front of the first ten records. */
const number = (index: number) => (index < 9 ? `${index + 1}  ` : index === 9 ? "0  " : "");

/** What the player accepts as a menu text: 1–80 bytes, no controls, no surrounding whitespace. */
export const segmentValid = (text: string) => text !== "" && bytes(text) <= SEGMENT_BYTES && menuText(text) === text;

/**
 * The menu lines of the records: number and display name (data), with the B25
 * point when there is one as a text value (`{} ({})`, in Chinese `{}（{}）`).
 * Each line fits the player's 80 bytes in every language: the name loses
 * controls and edge whitespace and is shortened in the middle by bytes; a
 * duplicate gets ` (2)`, ` (3)`, …. `null` for a line that still would not
 * pass (not expected; the menu leaves it out).
 */
export function recentTitles(items: RecentItem[], resume: Resume, { text }: I18n): (string | Text | null)[] {
  // The player tells the lines of a submenu apart by their English rendering
  // (a second one is refused, asynchronously): the fixed items count too.
  const used = new Set<string>(FIXED_TITLES);
  return items.slice(0, MENU_ITEMS).map((item, index) => {
    const memo = resume.memo(item);
    const prefix = number(index);
    const point = memo ? clock(memo.position) : null;
    const shown = menuText(displayName(item.source)) || menuText(item.source) || "…";
    const room = SEGMENT_BYTES - bytes(prefix) - (point ? bytes(point) + POINT_BYTES : 0);
    const english = (name: string) => (point ? `${name} (${point})` : name);
    let name = `${prefix}${middleBytes(shown, room)}`;
    // Two records with the same name past the tenth (or shortened alike, or a
    // name that reads like another line with its point) would be the same menu path.
    for (let n = 2; used.has(english(name)); n++) {
      const suffix = ` (${n})`;
      name = `${prefix}${middleBytes(shown, room - bytes(suffix))}${suffix}`;
    }
    used.add(english(name));
    // The English rendering is what the player checks.
    if (!segmentValid(english(name))) return null;
    return point ? text("{} ({})", [name, point]) : name;
  });
}

/**
 * Open ▸ Recently Played: rebuilt in a child scope whenever the records (or a shown
 * B25 point) change, changes within `timing.menuRebuild` merged. The whole
 * submenu is rebuilt so that the records stay above the management items.
 */
export class RecentMenu {
  readonly #plugin: Plugin;
  readonly #recent: Recent;
  readonly #resume: Resume;
  readonly #osd: Osd;
  /** Holds the submenu scopes; created before the onDispose below, so it is released after it. */
  readonly #root: Scope;
  #scope: Scope | null = null;
  #built: string | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #building: Promise<void> = Promise.resolve();
  #disposed = false;

  constructor(plugin: Plugin, recent: Recent, resume: Resume, osd: Osd) {
    this.#plugin = plugin;
    this.#recent = recent;
    this.#resume = resume;
    this.#osd = osd;
    this.#root = plugin.scope();
    recent.onChange(() => this.schedule());
    resume.onChange(() => this.schedule());
    // Runs before the submenu scopes are released (newest first): a rebuild due
    // meanwhile would otherwise declare its commands while the old ones are still
    // being removed ("command recent.item.0 is already declared").
    plugin.onDispose(() => {
      this.#disposed = true;
      if (this.#timer) clearTimeout(this.#timer);
    });
  }

  /** Rebuild after `timing.menuRebuild`; further calls meanwhile join that rebuild. */
  schedule() {
    if (this.#timer || this.#disposed) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#building = this.#building.then(() => this.build());
    }, timing.menuRebuild);
  }

  /** Build now (at activation, and when a scheduled rebuild is due); nothing happens when no line changed. */
  async build() {
    const items = this.#recent.items.slice(0, MENU_ITEMS);
    const titles = recentTitles(items, this.#resume, this.#plugin.i18n);
    const signature = JSON.stringify([titles, items.map((item) => item.source)]);
    if (signature === this.#built || this.#disposed) return;
    this.#built = signature;
    const old = this.#scope;
    const scope = (this.#scope = this.#root.scope());
    if (old) await old.dispose();
    if (this.#disposed) return; // stopped meanwhile: `#root` releases `scope`
    // Each title is a literal in its own `scope.commands.add` (`i18n extract` reads only those).
    const order = RECENT_ORDER;
    const { text } = this.#plugin.i18n;
    // The palette's "Category: Title" (player 0.7.6): the submenu's own label, so its translations.
    const category = text("Recently Played");

    if (!items.length)
      scope.commands.add({
        id: "recent.empty",
        path: [...RECENT, "(No Entries)"],
        category,
        order,
        group: "recent",
        enabled: false,
        run: () => {},
      });
    items.forEach((item, index) => {
      const title = titles[index];
      // A line the player would refuse must not cost the others nor the plugin.
      // The player's refusal arrives asynchronously and during activation still
      // fails it (the SDK joins every registration to the activation), so the
      // check in `recentTitles` (`segmentValid`, distinct English renderings)
      // is what protects activation: such a line is left out and logged. The
      // try/catch below only covers what `commands.add` throws synchronously.
      if (!title) return console.warn(`[vestra.history] Recently Played: line ${index + 1} left out (not a valid menu text)`);
      try {
        // The titles are data (number, name), with the point as a text value.
        // i18n-ignore
        scope.commands.add({
          id: `recent.item.${index}`,
          path: [...RECENT, title],
          category,
          order,
          group: "recent",
          run: () => this.open(item),
        });
      } catch (error) {
        console.warn(`[vestra.history] Recently Played: line ${index + 1} left out:`, error);
      }
    });
    const some = () => this.#recent.items.length > 0;
    // The palette hides a disabled command without a reason (player 0.7.6).
    const noEntries = () => (some() ? null : text("No recently played items"));
    scope.commands.add({
      id: "recent.prune",
      path: [...RECENT, "Remove Invalid Entries"],
      category,
      order,
      group: "recent-manage",
      enabled: some,
      reason: noEntries,
      run: async () => quiet(this.#osd.show("Removed {} invalid entries", [await this.#recent.prune()])),
    });
    scope.commands.add({
      id: "recent.clear",
      path: [...RECENT, "Clear Entries"],
      category,
      order,
      group: "recent-manage",
      enabled: some,
      reason: noEntries,
      run: () => {
        this.#recent.clear();
        quiet(this.#osd.show("Playback history cleared"));
      },
    });
    scope.commands.add({
      id: "recent.enabled",
      path: [...RECENT, "Record Playback History"],
      category,
      order,
      group: "recent-manage",
      checked: () => this.#recent.enabled,
      run: () => this.#recent.setEnabled(!this.#recent.enabled),
    });
  }

  /** Open a record (user initiated, so unconditional); a missing file is reported and kept. */
  async open(item: RecentItem) {
    if (!(await present(item.source))) {
      quiet(this.#osd.show("File not found: {}", [displayName(item.source)]));
      return;
    }
    // The root player: the media outlives this submenu's scope.
    const { player } = this.#plugin;
    const reason = (error: unknown) => (error instanceof Error ? error.message : String(error));
    try {
      const media = await player.open(item.source);
      media.ready().catch((error: { code?: string }) => {
        if (error?.code === "failed") quiet(this.#osd.show("Cannot open: {}", [reason(error)]));
      });
    } catch (error) {
      quiet(this.#osd.show("Cannot open: {}", [reason(error)]));
    }
  }
}

/** Playback ▸ Resume Playback ▸ (250–254). */
export function addResumeCommands(plugin: Plugin, resume: Resume, osd: Osd) {
  const path = ["@playback", "Resume Playback"] as const;
  // The palette's "Category: Title" (player 0.7.6): the submenu's own label, so its translations.
  const category = plugin.i18n.text("Resume Playback");
  const modes = [
    ["off", "Don't Resume"],
    ["auto", "Resume Automatically"],
    ["ask", "Ask Every Time"],
  ] as const satisfies readonly (readonly [Mode, string])[];
  modes.forEach(([mode, title], index) =>
    plugin.commands.add({
      id: `resume.${mode}`,
      path: [...path, title],
      category,
      order: 250 + index,
      group: "domain",
      radio: true,
      checked: () => resume.mode === mode,
      run: () => resume.setMode(mode),
    }),
  );
  plugin.commands.add({
    id: "resume.audio",
    path: [...path, "Also Remember Audio Files"],
    category,
    order: 253,
    group: "resume-options",
    checked: () => resume.audio,
    run: () => resume.setAudio(!resume.audio),
  });
  plugin.commands.add({
    id: "resume.clear",
    path: [...path, "Clear All Saved Positions"],
    category,
    order: 254,
    group: "resume-options",
    run: () => {
      resume.clear();
      quiet(osd.show("All saved positions cleared"));
    },
  });
}
