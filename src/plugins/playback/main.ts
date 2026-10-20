import type { Plugin } from "../../sdk/plugin/index.ts";
import { wireBar } from "./bar.ts";
import { addCommands } from "./commands.ts";
import { Playback } from "./controller.ts";
import { Editor } from "./editor.ts";

/** Basic playback: the bottom bar plus the 文件/播放/音频/视图 commands. */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const bar = plugin.panels.mount("bottom", "ui/bar.slint");
  const playback = new Playback(player, plugin.dialogs, () => plugin.update());
  const editor = new Editor(playback, player, bar);
  const run = addCommands(plugin, playback, editor);
  wireBar(bar, player, playback, editor, run);
  plugin.onError((error) => playback.report(error));

  // Another source, or anyone else's change during a drag, ends the gesture
  // and closes the editor; further pointer moves wait for the next press.
  let sourceId = player.state.sourceId;
  player.onChange(async (state) => {
    const replaced = state.sourceId !== sourceId;
    sourceId = state.sourceId;
    await Promise.resolve(); // the drag gesture observes the same change first
    if (replaced || playback.gestureLost) {
      playback.clearSeek();
      playback.blockedGesture = true;
      playback.dragging = false;
      playback.error = "";
      editor.kind = "";
      bar.setInteraction({ focused: false, dragging: false, popupOpen: false });
    }
    editor.invalidate();
  });
}
