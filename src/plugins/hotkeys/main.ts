import type { Plugin } from "../../sdk/plugin/index.ts";
import { IgnoreDefaults } from "./ignore.ts";
import { KeysPage } from "./keys-page.ts";
import { BindingsModel } from "./model.ts";
import { MousePage } from "./mouse-page.ts";

/** 设置 ▶ (orders 832–834, group settings-input; no default bindings of its own). */
export const SETTINGS = "设置";

/**
 * 快捷键设置: 设置 ▶ 快捷键… (J01), 鼠标… (J02) and 忽略默认快捷键 (J03).
 * 快捷键… and 鼠标… open the options window on this plugin's pages
 * (hotkeys.keyboard, hotkeys.mouse; category 快捷键与鼠标).
 * Talks only to the player's binding table (`plugin.bindings`); never to
 * other plugins. Their commands appear and go as they load and stop.
 */
export default async function activate(plugin: Plugin) {
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const model = new BindingsModel(plugin);
  const ignore = new IgnoreDefaults(plugin, model, osd);
  const keys = new KeysPage(plugin, model, ignore, osd);
  const mouse = new MousePage(plugin, model, osd);

  plugin.commands.add({
    id: "keys",
    path: [SETTINGS, "快捷键…"],
    order: 832,
    group: "settings-input",
    run: () => keys.show(),
  });
  plugin.commands.add({
    id: "mouse",
    path: [SETTINGS, "鼠标…"],
    order: 833,
    group: "settings-input",
    run: () => mouse.show(),
  });
  plugin.commands.add({
    id: "ignore-defaults",
    path: [SETTINGS, "忽略默认快捷键"],
    order: 834,
    group: "settings-input",
    checked: () => ignore.enabled,
    run: () => ignore.toggle(),
  });

  // The pages can be reached from the options window's tree at any time.
  void model.reload().catch(() => {});
  // J03 is checked against the table at start: commands of plugins that
  // loaded while hotkeys was away are switched off now.
  void ignore.sync();
}
