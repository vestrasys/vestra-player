import type { Plugin } from "../../sdk/plugin/index.ts";

/**
 * A sidebar panel with its own right-click menu (C11), a key that works only
 * while the panel has focus, commands outside the main menu, and the shared
 * next/previous it provides (C6). Detach it with the core-ui example (C14).
 * Use it instead of the playlist: there is one sidebar.
 */
export default async function activate(plugin: Plugin) {
  const items = ["第一项", "第二项", "第三项"];
  let selected = 0;
  const panel = plugin.panels.mount("sidebar", "panel.slint", {
    title: "示例列表",
  });
  panel.bind(() => ({ rows: items, selected }));
  const move = (step: number) =>
    (selected = Math.max(0, Math.min(items.length - 1, selected + step)));

  const first = plugin.commands.add({
    id: "first",
    title: "选择第一项",
    focus: "panel",
    bindings: ["Home"],
    run: () => (selected = 0),
  });
  const remove = plugin.commands.add({
    id: "remove",
    title: "移除所选",
    focus: "panel",
    bindings: ["Delete"],
    enabled: () => items.length > 0,
    run: () => {
      items.splice(selected, 1);
      move(0);
    },
  });
  panel.on("select", (index: number) => (selected = index));
  panel.on("contextMenu", (x: number, y: number) =>
    panel.showMenu([first, null, remove], { x, y }),
  );

  plugin.player.provideSequence({
    canNext: () => selected < items.length - 1,
    canPrevious: () => selected > 0,
    next: () => move(1),
    previous: () => move(-1),
  });
}
