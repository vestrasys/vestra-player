import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  let mark: number | null = null;

  const window = plugin.windows.create("bookmark.slint");
  window.bind(() => ({
    label: mark === null ? "没有书签" : `书签：${mark.toFixed(1)} 秒`,
    canMark: player.state.position !== null,
    canJump: mark !== null && player.state.can.seek,
  }));
  window.on("mark", () => (mark = player.state.position));
  window.on("jump", () => player.seek(mark!));

  plugin.commands.add({
    id: "bookmark.open",
    path: ["播放", "书签…"],
    order: 270,
    run: () => window.show(),
  });
}
