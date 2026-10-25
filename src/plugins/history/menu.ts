import { basename } from "node:path";
import type { I18n, Osd, Plugin, Scope, Text } from "@vestra-player/sdk";
import { clock, displayName, middle } from "./identity.ts";
import { exists, MENU_ITEMS, type Recent, type RecentItem } from "./recent.ts";
import type { Mode, Resume } from "./resume.ts";
import { timing } from "./store.ts";

const RECENT = ["@open", "Recently Played"] as const;
/** All of Open ▸ Recently Played shares one order; declaration order arranges it (SDK: Concepts, Commands, "Order and separators"). */
const RECENT_ORDER = 114;
/** Headless hosts refuse the display; that never fails the action itself. */
export const quiet = (shown: Promise<void>) => void shown.catch(() => {});

/** A menu line is at most this long. */
const TITLE_MAX = 80;

/** `1`–`9`, `0`, then nothing: the number in front of the first ten records. */
const number = (index: number) => (index < 9 ? `${index + 1}  ` : index === 9 ? "0  " : "");

/**
 * The menu lines of the records: number and display name (data), with the B25
 * point when there is one as a text value (`{} ({})`, in Chinese `{}（{}）`).
 */
export function recentTitles(items: RecentItem[], resume: Resume, { text }: I18n): (string | Text)[] {
  const used = new Set<string>();
  return items.slice(0, MENU_ITEMS).map((item, index) => {
    const memo = resume.memo(item.key);
    const prefix = number(index);
    const point = memo ? clock(memo.position) : null;
    // The point and its two brackets.
    const room = TITLE_MAX - [...prefix].length - (point ? point.length + 2 : 0);
    let name = `${prefix}${middle(displayName(item.source), room)}`;
    // Two records with the same name past the tenth would be the same menu path.
    for (let n = 2; used.has(`${name} ${point}`); n++) name = `${prefix}${middle(displayName(item.source), room - 4)} (${n})`;
    used.add(`${name} ${point}`);
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
    recent.onChange(() => this.schedule());
    resume.onChange(() => this.schedule());
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
    const scope = (this.#scope = this.#plugin.scope());
    if (old) await old.dispose();
    // Each title is a literal in its own `scope.commands.add` (`i18n extract` reads only those).
    const order = RECENT_ORDER;

    if (!items.length)
      scope.commands.add({
        id: "recent.empty",
        path: [...RECENT, "(No Entries)"],
        order,
        group: "recent",
        enabled: false,
        run: () => {},
      });
    // The titles are data (number, name), with the point as a text value.
    // i18n-ignore
    items.forEach((item, index) =>
      scope.commands.add({
        id: `recent.item.${index}`,
        path: [...RECENT, titles[index]!],
        order,
        group: "recent",
        run: () => this.open(item),
      }),
    );
    const some = () => this.#recent.items.length > 0;
    scope.commands.add({
      id: "recent.prune",
      path: [...RECENT, "Remove Invalid Entries"],
      order,
      group: "recent-manage",
      enabled: some,
      run: async () => quiet(this.#osd.show("Removed {} invalid entries", [await this.#recent.prune()])),
    });
    scope.commands.add({
      id: "recent.clear",
      path: [...RECENT, "Clear Entries"],
      order,
      group: "recent-manage",
      enabled: some,
      run: () => {
        this.#recent.clear();
        quiet(this.#osd.show("Playback history cleared"));
      },
    });
    scope.commands.add({
      id: "recent.enabled",
      path: [...RECENT, "Record Playback History"],
      order,
      group: "recent-manage",
      checked: () => this.#recent.enabled,
      run: () => this.#recent.setEnabled(!this.#recent.enabled),
    });
  }

  /** Open a record (user initiated, so unconditional); a missing file is reported and kept. */
  async open(item: RecentItem) {
    if (item.kind === "file" && !(await exists(item.source))) {
      quiet(this.#osd.show("File not found: {}", [basename(item.source)]));
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
  const modes = [
    ["off", "Don't Resume"],
    ["auto", "Resume Automatically"],
    ["ask", "Ask Every Time"],
  ] as const satisfies readonly (readonly [Mode, string])[];
  modes.forEach(([mode, title], index) =>
    plugin.commands.add({
      id: `resume.${mode}`,
      path: [...path, title],
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
    order: 253,
    group: "resume-options",
    checked: () => resume.audio,
    run: () => resume.setAudio(!resume.audio),
  });
  plugin.commands.add({
    id: "resume.clear",
    path: [...path, "Clear All Saved Positions"],
    order: 254,
    group: "resume-options",
    run: () => {
      resume.clear();
      quiet(osd.show("All saved positions cleared"));
    },
  });
}
