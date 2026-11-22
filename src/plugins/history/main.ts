import type { Plugin } from "@vestra-player/sdk";
import { addResumeCommands, quiet, RecentMenu } from "./menu.ts";
import { Recent } from "./recent.ts";
import { Resume } from "./resume.ts";

/**
 * Playback History: Open ▸ Recently Played (A12) and Resume Playback (B25, command palette only). Sources are
 * seen only through `player.state`, whoever opened them; reopening uses
 * `player.open`. Data lives in `plugin.storage` files (recent.json,
 * positions.json), written after each change and on deactivation.
 */
export default async function activate(plugin: Plugin) {
  plugin.player.subscribe(); // B3: removed in SDK 1.11; migrate (events-and-triggers.md §7)
  await plugin.player.read(); // was "playback.read"
  const { player } = plugin;
  const recent = new Recent(plugin);
  const resume = new Resume(plugin, plugin.osd);
  const menu = new RecentMenu(plugin, recent, resume, plugin.osd);
  addResumeCommands(plugin, resume, plugin.osd);
  // The player's own message, shown as it is.
  // i18n-ignore
  plugin.onError((error) => quiet(plugin.osd.show(error.message)));

  recent.observe(player.state);
  resume.observe(player.state, true);
  void menu.build();
  player.onChange((state) => { // B3: Player.onChange is removed in SDK 1.11
    recent.observe(state);
    resume.observe(state);
  });
}
