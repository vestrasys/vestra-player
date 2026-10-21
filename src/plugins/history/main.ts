import type { Plugin } from "@vestra-player/sdk";
import { addResumeCommands, RecentMenu } from "./menu.ts";
import { Recent } from "./recent.ts";
import { Resume } from "./resume.ts";

/**
 * 播放历史: 打开 ▶ 最近播放 (A12) and 播放 ▶ 记忆播放位置 (B25). Sources are
 * seen only through `player.state`, whoever opened them; reopening uses
 * `player.open`. Data lives in `plugin.storage` files (recent.json,
 * positions.json), written after each change and on deactivation.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  const recent = new Recent(plugin);
  const resume = new Resume(plugin, osd);
  const menu = new RecentMenu(plugin, recent, resume, osd);
  addResumeCommands(plugin, resume, osd);
  plugin.onError((error) => osd(error.message));

  recent.observe(player.state);
  resume.observe(player.state, true);
  void menu.build();
  player.onChange((state) => {
    recent.observe(state);
    resume.observe(state);
  });
}
