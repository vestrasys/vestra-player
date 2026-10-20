import type { Plugin } from "../../sdk/plugin/index.ts";
import { PluginsWindow } from "./manager.ts";

/**
 * 插件管理: 设置 ▶ 插件管理… (J07, order 836, group settings-plugins).
 * Everything goes through `plugin.packages`; the player stops, starts,
 * installs and removes plugins, this plugin is only the window.
 */
export default async function activate(plugin: Plugin) {
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const window = new PluginsWindow(plugin, osd);
  plugin.commands.add({
    id: "manage",
    path: ["设置", "插件管理…"],
    order: 836,
    group: "settings-plugins",
    run: () => window.show(),
  });
}
