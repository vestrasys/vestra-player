import type { Plugin } from "@vestra-player/sdk";
import { IgnoreDefaults } from "./ignore.ts";
import { KeysPage } from "./keys-page.ts";
import { hasCmd } from "./keys.ts";
import { BindingsModel } from "./model.ts";
import { MousePage } from "./mouse-page.ts";

/** The three commands (orders 832–834, group settings-input) are in the command palette only (menu sweep B5); no default bindings of its own. */

/** Shows an OSD message (`plugin.osd.show(…)`); headless hosts refuse the display, which never fails the action itself. */
export type Osd = (shown: Promise<unknown>) => void;

/**
 * Shortcut Settings: Keyboard Shortcuts… (J01), Mouse… (J02) and Ignore Default
 * Shortcuts (J03), in the command palette ("Settings: …"). Keyboard Shortcuts… and Mouse… open the options
 * window on this plugin's pages (hotkeys.keyboard, hotkeys.mouse; category
 * Keys and Mouse). Talks only to the player's binding table (`plugin.bindings`);
 * never to other plugins. Their commands appear and go as they load and stop.
 */
export default async function activate(plugin: Plugin) {
  const osd: Osd = (shown) => void shown.catch(() => {});
  // i18n-ignore: the player's (or the SDK's) own message
  plugin.onError((error) => osd(plugin.osd.show(error.message)));

  // `Cmd` is offered on macOS only (⌘); elsewhere it is the Windows key (`hasCmd`).
  const { os } = await plugin.app.system().catch(() => ({ os: "other" as const }));
  const mac = hasCmd(os);
  const model = new BindingsModel(plugin);
  const ignore = new IgnoreDefaults(plugin, model, osd);
  const keys = new KeysPage(plugin, model, ignore, osd, mac);
  const mouse = new MousePage(plugin, model, osd, mac);

  // Command palette category (player 0.7.6): the Settings menu the three lived in. A text
  // value, so `i18n extract` finds it; translated like the player's Settings menu.
  const category = plugin.i18n.text("Settings");
  plugin.commands.add({
    id: "keys",
    category,
    title: "Keyboard Shortcuts…",
    order: 832,
    group: "settings-input",
    run: () => keys.show(),
  });
  plugin.commands.add({
    id: "mouse",
    category,
    title: "Mouse…",
    order: 833,
    group: "settings-input",
    run: () => mouse.show(),
  });
  plugin.commands.add({
    id: "ignore-defaults",
    category,
    home: "keys", // the Keyboard Shortcuts page has the same switch
    title: "Ignore Default Shortcuts",
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
