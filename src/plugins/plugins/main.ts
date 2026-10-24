import type { Plugin } from "@vestra-player/sdk";
import { PluginsPage } from "./manager.ts";

/**
 * Plugin Manager: Settings ▸ Plugin Manager… (J07, order 836, group settings-plugins).
 * Everything goes through `plugin.packages`; the player stops, starts,
 * installs and removes plugins, this plugin is only the window.
 */
export default async function activate(plugin: Plugin) {
  plugin.onError((error) => {
    // Headless hosts refuse the display; that never fails the action itself.
    // i18n-ignore: the player's (or the SDK's) own message
    void plugin.osd.show(error.message).catch(() => {});
  });

  const page = new PluginsPage(plugin);
  // The page can be reached from the options window's tree at any time.
  void page.refresh();
  plugin.commands.add({
    id: "manage",
    path: ["@settings", "Plugin Manager…"],
    order: 836,
    group: "settings-plugins",
    run: () => page.show(),
  });
}
