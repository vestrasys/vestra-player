import type { Plugin } from "../../sdk/plugin/index.ts";
import { addResumeCommands, RecentMenu } from "./menu.ts";
import { Recent } from "./recent.ts";
import { Resume } from "./resume.ts";

/**
 * 播放历史: 打开 ▶ 最近播放 (A12) and 播放 ▶ 记忆播放位置 (B25). Sources are
 * seen only through `player.state`, whoever opened them; reopening uses
 * `player.open`. Data lives in `plugin.dataDirectory`.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  let directory: string | null = null;
  try {
    directory = plugin.dataDirectory;
  } catch {
    // No data directory: history lives for this run only.
  }

  const recent = new Recent(directory);
  const resume = new Resume(plugin, directory, osd);
  const menu = new RecentMenu(plugin, recent, resume, osd);
  addResumeCommands(plugin, resume, osd);
  plugin.onError((error) => osd(error.message));
  plugin.onDispose(() => recent.flush());

  recent.observe(player.state);
  resume.observe(player.state, true);
  void menu.build();
  player.onChange((state) => {
    recent.observe(state);
    resume.observe(state);
  });
}
