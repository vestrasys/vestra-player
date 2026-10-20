import type { Plugin } from "../../sdk/plugin/index.ts";
import { Actions } from "./actions.ts";
import { Bookmarks } from "./bookmarks.ts";
import { Chapters, hasMedia } from "./chapters.ts";
import { ListWindow } from "./list.ts";
import { AbLoop } from "./loop.ts";
import { addLoopCommands, MarksMenu } from "./menu.ts";
import { Settings } from "./store.ts";

/**
 * 章节、书签与区段: 播放 ▶ 章节/书签 (B21: chapters from `player.chapters()`,
 * bookmarks per file in bookmarks.json, the 章节/书签 window) and
 * 播放 ▶ A-B 区段循环 (B23: `player.setLoop`, the count in marks.json).
 * Everything follows `player.state`, whoever opened the source.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const settings = new Settings(plugin);
  const chapters = new Chapters(player);
  const bookmarks = new Bookmarks(plugin);
  const actions = new Actions(plugin, chapters, bookmarks, osd);
  const loop = new AbLoop(plugin, settings, osd);
  const list = new ListWindow(plugin, chapters, bookmarks, actions);
  const menu = new MarksMenu(plugin, chapters, bookmarks, actions, () => list.show());
  addLoopCommands(plugin, loop, actions);

  const observe = (state: typeof player.state) => {
    chapters.observe(state);
    bookmarks.open(hasMedia(state) || state.phase === "loading" ? state.source : null, state.sourceId);
    loop.observe(state);
  };
  observe(player.state);
  void menu.build();
  player.onChange(observe);
}
