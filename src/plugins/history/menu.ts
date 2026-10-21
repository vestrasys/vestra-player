import { basename } from "node:path";
import type { CommandOptions, Plugin, Scope } from "@vestra-player/sdk";
import { clock, displayName, middle } from "./identity.ts";
import { exists, MENU_ITEMS, type Recent, type RecentItem } from "./recent.ts";
import type { Mode, Resume } from "./resume.ts";
import { timing } from "./store.ts";

const RECENT = ["打开", "最近播放"];
/** All of 打开 ▶ 最近播放 shares one order; declaration order arranges it (reference §4). */
const RECENT_ORDER = 114;
/** A menu line is at most this long. */
const TITLE_MAX = 80;

/** `1`–`9`, `0`, then nothing: the number in front of the first ten records. */
const number = (index: number) => (index < 9 ? `${index + 1}  ` : index === 9 ? "0  " : "");

/** The menu lines of the records: number, display name, and the B25 point when there is one. */
export function recentTitles(items: RecentItem[], resume: Resume): string[] {
  const used = new Set<string>();
  return items.slice(0, MENU_ITEMS).map((item, index) => {
    const memo = resume.memo(item.key);
    const prefix = number(index);
    const suffix = memo ? `（${clock(memo.position)}）` : "";
    const room = TITLE_MAX - [...prefix].length - [...suffix].length;
    let title = `${prefix}${middle(displayName(item.source), room)}${suffix}`;
    // Two records with the same name past the tenth would be the same menu path.
    for (let n = 2; used.has(title); n++) title = `${prefix}${middle(displayName(item.source), room - 4)} (${n})${suffix}`;
    used.add(title);
    return title;
  });
}

/**
 * 打开 ▶ 最近播放: rebuilt in a child scope whenever the records (or a shown
 * B25 point) change, changes within `timing.menuRebuild` merged. The whole
 * submenu is rebuilt so that the records stay above the management items.
 */
export class RecentMenu {
  readonly #plugin: Plugin;
  readonly #recent: Recent;
  readonly #resume: Resume;
  readonly #osd: (text: string) => void;
  #scope: Scope | null = null;
  #built: string | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #building: Promise<void> = Promise.resolve();
  #disposed = false;

  constructor(plugin: Plugin, recent: Recent, resume: Resume, osd: (text: string) => void) {
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
    const titles = recentTitles(items, this.#resume);
    const signature = JSON.stringify([titles, items.map((item) => item.source)]);
    if (signature === this.#built || this.#disposed) return;
    this.#built = signature;
    const old = this.#scope;
    const scope = (this.#scope = this.#plugin.scope());
    if (old) await old.dispose();
    const add = (id: string, title: string, group: string, options: Partial<CommandOptions> = {}) =>
      scope.commands.add({ id, path: [...RECENT, title], order: RECENT_ORDER, group, run: () => {}, ...options });

    if (!items.length) add("recent.empty", "（无记录）", "recent", { enabled: false });
    items.forEach((item, index) =>
      add(`recent.item.${index}`, titles[index]!, "recent", { run: () => this.open(item) }),
    );
    const some = () => this.#recent.items.length > 0;
    add("recent.prune", "移除无效记录", "recent-manage", {
      enabled: some,
      run: async () => this.#osd(`已移除 ${await this.#recent.prune()} 条无效记录`),
    });
    add("recent.clear", "清除记录", "recent-manage", {
      enabled: some,
      run: () => {
        this.#recent.clear();
        this.#osd("已清除播放记录");
      },
    });
    add("recent.enabled", "记录播放历史", "recent-manage", {
      checked: () => this.#recent.enabled,
      run: () => this.#recent.setEnabled(!this.#recent.enabled),
    });
  }

  /** Open a record (user initiated, so unconditional); a missing file is reported and kept. */
  async open(item: RecentItem) {
    if (item.kind === "file" && !(await exists(item.source))) {
      this.#osd(`文件不存在：${basename(item.source)}`);
      return;
    }
    // The root player: the media outlives this submenu's scope.
    const { player } = this.#plugin;
    const reason = (error: unknown) => (error instanceof Error ? error.message : String(error));
    try {
      const media = await player.open(item.source);
      media.ready().catch((error: { code?: string }) => {
        if (error?.code === "failed") this.#osd(`无法打开：${reason(error)}`);
      });
    } catch (error) {
      this.#osd(`无法打开：${reason(error)}`);
    }
  }
}

/** 播放 ▶ 记忆播放位置 ▶ (250–254). */
export function addResumeCommands(plugin: Plugin, resume: Resume, osd: (text: string) => void) {
  const path = ["播放", "记忆播放位置"];
  const modes: [Mode, string][] = [
    ["off", "不记忆"],
    ["auto", "自动继续"],
    ["ask", "每次询问"],
  ];
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
    path: [...path, "同时记忆音频文件"],
    order: 253,
    group: "resume-options",
    checked: () => resume.audio,
    run: () => resume.setAudio(!resume.audio),
  });
  plugin.commands.add({
    id: "resume.clear",
    path: [...path, "清除所有记忆点"],
    order: 254,
    group: "resume-options",
    run: () => {
      resume.clear();
      osd("已清除所有记忆点");
    },
  });
}
