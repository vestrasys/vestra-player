import type { Plugin } from "../../sdk/plugin/index.ts";
import { IgnoreDefaults } from "./ignore.ts";
import { KeysWindow } from "./keys-window.ts";
import { BindingsModel } from "./model.ts";
import { MouseWindow } from "./mouse-window.ts";

/** 设置 ▶ (orders 832–834, group settings-input; no default bindings of its own). */
export const SETTINGS = "设置";

/**
 * 快捷键设置: 设置 ▶ 快捷键… (J01), 鼠标… (J02) and 忽略默认快捷键 (J03).
 * Talks only to the player's binding table (`plugin.bindings`); never to
 * other plugins. Their commands appear and go as they load and stop.
 */
export default async function activate(plugin: Plugin) {
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const model = new BindingsModel(plugin);
  const ignore = new IgnoreDefaults(plugin, model, osd);
  const keys = new KeysWindow(plugin, model, ignore, osd);
  const mouse = new MouseWindow(plugin, model, osd);

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

  // J03 is checked against the table at start: commands of plugins that
  // loaded while hotkeys was away are switched off now.
  void ignore.sync();
}
