import type { Plugin } from "../../sdk/plugin/index.ts";
import { linkDialog } from "./link.ts";
import { playback } from "./playback.ts";
import { Queue } from "./queue.ts";

const titles = {
  add: "添加文件…",
  link: "添加链接…",
  play: "播放选中项",
  previous: "上一项",
  next: "下一项",
  remove: "移除选中项",
  "move-up": "上移",
  "move-down": "下移",
  clear: "清空列表",
  visibility: "显示播放列表",
} as const;
type Action = keyof typeof titles;
const actions = Object.keys(titles) as Action[];
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

export default async function activate(plugin: Plugin) {
  const queue = new Queue();
  const player = playback(plugin, queue);
  const link = linkDialog(plugin, queue);
  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint");
  let adding = false;

  const enabled = (action: Action) => {
    const index = queue.rows.findIndex((row) => row.rowId === queue.selected);
    switch (action) {
      case "add":
        return !adding;
      case "play":
        return !!queue.row(queue.selected) && !player.busy();
      case "previous":
        return !!queue.neighbor(-1) && !player.busy();
      case "next":
        return !!queue.neighbor(1) && !player.busy();
      case "remove":
        return index >= 0;
      case "move-up":
        return index > 0;
      case "move-down":
        return index >= 0 && index < queue.rows.length - 1;
      case "clear":
        return queue.rows.length > 0;
      default:
        return true;
    }
  };

  // Menu items and sidebar buttons share this; buttons skip the host's enabled check.
  async function action(name: Action) {
    if (!enabled(name)) return;
    try {
      switch (name) {
        case "add":
          adding = true;
          try {
            const paths = await plugin.dialogs.openFiles({ title: "添加到播放列表" });
            if (paths.length) await queue.addFiles(paths);
          } finally {
            adding = false;
          }
          break;
        case "link":
          await link.show();
          break;
        case "play":
          if (queue.selected) await player.play(queue.selected);
          break;
        case "previous":
        case "next": {
          const row = queue.neighbor(name === "previous" ? -1 : 1);
          if (row) await player.play(row.rowId);
          break;
        }
        case "move-up":
        case "move-down":
          queue.move(name === "move-up" ? -1 : 1);
          break;
        case "remove": {
          const removed = queue.selected;
          queue.remove();
          if (player.pendingRow === removed) {
            queue.epoch++;
            queue.message = "正在打开的媒体已移出列表";
          }
          break;
        }
        case "clear":
          queue.clear();
          break;
        case "visibility":
          await list.toggle();
          break;
      }
    } catch (error) {
      queue.message = errorText(error);
    }
  }

  list.bind(() => {
    const selected = queue.row(queue.selected);
    return {
      rows: queue.rows.map((row, index) => ({
        rowId: row.rowId,
        label: `${index + 1}. ${row.title}`,
        status: row.status,
        selected: row.rowId === queue.selected,
        current: row.rowId === queue.binding?.rowId,
      })),
      message: queue.message,
      details: selected ? `${selected.source}${selected.error ? `\n${selected.error}` : ""}` : "尚未选择项目",
      canAdd: enabled("add"),
      canPlay: enabled("play"),
      canPrevious: enabled("previous"),
      canNext: enabled("next"),
      canRemove: enabled("remove"),
      canUp: enabled("move-up"),
      canDown: enabled("move-down"),
      canClear: enabled("clear"),
    };
  });
  list.on("action", (name: string) => (Object.hasOwn(titles, name) ? action(name as Action) : undefined));
  list.on("selectRow", (rowId: string) => queue.select(rowId));
  list.on("playRow", (rowId: string) => {
    if (!queue.row(rowId)) return;
    queue.select(rowId);
    return action("play");
  });
  list.on("selectStep", (step: number) => queue.selectNeighbor(step < 0 ? -1 : 1));
  list.on("hide", () => list.setVisible(false));
  list.on("focusChanged", (focused: boolean) => list.setInteraction({ focused }));

  for (const name of actions)
    plugin.commands.add({
      id: `playlist.${name}`,
      path: name === "visibility" ? ["视图", titles[name]] : ["文件", "播放列表", titles[name]],
      enabled: () => enabled(name),
      checked: () => (name === "visibility" ? (list.visibility?.preferred ?? true) : null),
      busy: () =>
        name === "add" ? adding : ["play", "next", "previous"].includes(name) && player.busy(),
      reason: () =>
        name === "visibility" && list.visibility?.reason === "space"
          ? "空间不足，放大窗口后可显示"
          : !enabled(name)
            ? player.busy()
              ? "正在打开或结果未知"
              : "当前没有可操作项"
            : null,
      run: () => action(name),
    });
}
