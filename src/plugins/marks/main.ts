import type { Plugin } from "@vestra-player/sdk";
import { Actions } from "./actions.ts";
import { Bookmarks } from "./bookmarks.ts";
import { Chapters, hasMedia } from "./chapters.ts";
import { ListWindow } from "./list.ts";
import { AbLoop } from "./loop.ts";
import { addLoopCommands, MarksMenu } from "./menu.ts";
import { osdOf } from "./osd.ts";
import { addSeekbarCommand, SeekbarMarks } from "./seekbar.ts";
import { Settings } from "./store.ts";

/**
 * Chapters, Bookmarks and A-B Loop: Playback ▸ Chapters and Bookmarks (B21:
 * chapters from `player.chapters()`, bookmarks per file in bookmarks.json,
 * the Chapters and Bookmarks window) and Playback ▸ A-B Loop (B23:
 * `player.setLoop`, the count in marks.json) and
 * the seek-bar marks (B22: this plugin's `timeline` layer, the switch in
 * marks.json).
 * Everything follows `player.state`, whoever opened the source.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const osd = osdOf(plugin);
  plugin.onError((error) => {
    // i18n-ignore: the player's (or the SDK's) own message
    void plugin.osd.show(error.message).catch(() => {});
  });

  const settings = new Settings(plugin);
  const chapters = new Chapters(player, () => plugin.update());
  const bookmarks = new Bookmarks(plugin);
  const actions = new Actions(plugin, chapters, bookmarks, osd);
  const loop = new AbLoop(plugin, settings, osd);
  const list = new ListWindow(plugin, chapters, bookmarks, actions);
  const menu = new MarksMenu(plugin, chapters, bookmarks, actions, () => list.show());
  addLoopCommands(plugin, loop, actions);
  const seekbar = new SeekbarMarks(plugin, settings, chapters, bookmarks, loop, osd);
  addSeekbarCommand(plugin, seekbar);

  const observe = (state: typeof player.state) => {
    chapters.observe(state);
    bookmarks.open(hasMedia(state) || state.phase === "loading" ? state.source : null, state.sourceId);
    loop.observe(state);
    seekbar.schedule();
  };
  observe(player.state);
  void menu.build();
  player.onChange(observe);
}
