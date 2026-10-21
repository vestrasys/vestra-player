import type { Plugin, ThemeId } from "../../sdk/plugin/index.ts";
import { DEFAULT_THEME, isTheme, THEMES, titleOf } from "./themes.ts";

/** 设置 ▶ (选项… 830; 外观 ▶ 840–849, of which the palettes take 840–841). */
const SETTINGS = "设置";
export const PAGE_ID = "appearance";

/**
 * 首选项: 设置 ▶ 选项… (J04, F5) opens the player's options window; this
 * plugin's own page there is 外观 (H10: palette radio with previews and
 * 恢复默认), also reachable as 设置 ▶ 外观 ▶ Vestra / PotPlayer 经典.
 * The palette itself is the player's (`plugin.appearance`): it is saved by
 * the player and stays when this plugin stops.
 */
export default async function activate(plugin: Plugin) {
  const { appearance } = plugin;
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  // One switch at a time: a second click waits and then sees the new value.
  let queue: Promise<void> = Promise.resolve();
  const choose = (theme: ThemeId) => {
    const next = queue.then(async () => {
      if (theme === appearance.theme) return;
      await appearance.set(theme);
      osd(`配色：${titleOf(theme)}`);
    });
    queue = next.catch(() => {});
    return next;
  };

  const page = plugin.settings.page("ui/appearance.slint", {
    id: PAGE_ID,
    category: "appearance",
    title: "外观",
    order: 10,
  });
  page.bind(() => ({
    themes: THEMES,
    theme: appearance.theme,
    canReset: appearance.theme !== DEFAULT_THEME,
  }));
  page.on("pick", (theme: unknown) => (isTheme(theme) ? choose(theme) : undefined));
  page.on("reset", () => choose(DEFAULT_THEME));

  plugin.commands.add({
    id: "options",
    path: [SETTINGS, "选项…"],
    order: 830,
    group: "settings-main",
    bindings: ["F5"],
    run: () => plugin.settings.show(),
  });
  THEMES.forEach((row, index) =>
    plugin.commands.add({
      id: `theme.${row.id}`,
      path: [SETTINGS, "外观", row.title],
      order: 840 + index,
      group: "settings-look",
      radio: true,
      checked: () => appearance.theme === row.id,
      run: () => choose(row.id),
    }),
  );
}
