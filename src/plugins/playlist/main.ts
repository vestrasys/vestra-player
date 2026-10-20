import type { Command, CommandOptions, Plugin } from "../../sdk/plugin/index.ts";
import { linkDialog } from "./link.ts";
import { playback } from "./playback.ts";
import { createProbeRunner, Prober, type ProbeRunner } from "./probe.ts";
import { clock, lengthOf, Queue, type Placement, type Repeat, type SortKey } from "./queue.ts";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const repeats: [Repeat, string][] = [
  ["none", "不循环"],
  ["all", "全部"],
  ["one", "单个"],
];

export default async function activate(plugin: Plugin, runner: ProbeRunner = createProbeRunner()) {
  const queue = new Queue();
  queue.say = (text) => void plugin.osd.show(text.slice(0, 200)).catch(() => {});
  const prober = new Prober(queue, runner, () => plugin.update());
  queue.removed = (rowId) => prober.cancel(rowId);
  plugin.onDispose(() => prober.cancelAll());
  const player = playback(plugin, queue);
  const link = linkDialog(plugin, queue);
  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "播放列表" });
  const sidebar = plugin.panels.sidebar;
  let adding = false;

  /** Every action runs through here; failures become a notice. */
  const guard =
    (run: () => unknown) =>
    async (): Promise<void> => {
      try {
        await run();
      } catch (error) {
        queue.notice(errorText(error));
      }
    };
  const canPlay = () => !!queue.row(queue.selected) && !player.busy();
  const hasRows = () => queue.rows.length > 0;

  const addFiles = guard(async () => {
    if (adding) return;
    adding = true;
    try {
      const paths = await plugin.dialogs.openFiles({ title: "添加到播放列表" });
      if (paths.length) await queue.addFiles(paths);
      prober.scan();
    } finally {
      adding = false;
    }
  });
  const playSelected = guard(() => (canPlay() && queue.selected ? player.play(queue.selected) : undefined));
  const step = (delta: 1 | -1) =>
    guard(() => {
      const row = !player.busy() ? queue.neighbor(delta) : undefined;
      return row ? player.play(row.rowId) : undefined;
    });
  const removeSelected = guard(() => {
    const pending = player.pendingRow;
    queue.removeSelected();
    if (pending && !queue.row(pending)) {
      queue.epoch++;
      queue.notice("正在打开的媒体已移出列表");
    }
  });
  const place = (where: Placement) => () => queue.place(where);
  const sort = (key: SortKey) => () => queue.sort(key);

  const reason = (enabled: () => boolean) => () =>
    enabled() ? null : player.busy() ? "正在打开或结果未知" : "当前没有可操作项";
  /** A list command: not in the main menu; its keys work while the list has focus. */
  const listCommand = (
    id: string,
    title: string,
    enabled: () => boolean,
    run: () => unknown,
    bindings?: string[],
    extra: Partial<CommandOptions> = {},
  ) =>
    plugin.commands.add({
      id: `playlist.${id}`,
      title,
      enabled,
      reason: reason(enabled),
      ...(bindings ? { bindings, focus: "panel" as const } : {}),
      run,
      ...extra,
    });

  // Main menu (round allocation table: windows 800, 播放 ▶ 播放列表 ▶ 230–249).
  plugin.commands.add({
    id: "playlist.visibility",
    path: ["播放列表"],
    group: "windows",
    order: 800,
    bindings: ["F6"],
    checked: () => sidebar.state?.preferred ?? true,
    reason: () => (sidebar.state?.reason === "space" ? "空间不足，放大窗口后可显示" : null),
    run: () => sidebar.toggle(),
  });
  const playlistMenu = (id: string, title: string[], order: number, options: Partial<CommandOptions>) =>
    plugin.commands.add({ id: `playlist.${id}`, path: ["播放", "播放列表", ...title], order, run: () => {}, ...options });
  const add = playlistMenu("add", ["添加文件…"], 230, {
    enabled: () => !adding,
    busy: () => adding,
    run: addFiles,
  });
  const addLink = playlistMenu("link", ["添加链接…"], 231, { run: guard(() => link.show()) });
  repeats.forEach(([mode, title], index) =>
    playlistMenu(`repeat-${mode}`, ["循环", title], 232 + index, {
      radio: true,
      checked: () => queue.repeat === mode,
      run: () => {
        queue.repeat = mode;
        queue.notice(`循环：${title}`);
      },
    }),
  );
  playlistMenu("shuffle", ["随机"], 235, {
    checked: () => queue.shuffle,
    run: () => {
      queue.setShuffle(!queue.shuffle);
      queue.notice(queue.shuffle ? "随机播放：开" : "随机播放：关");
    },
  });

  // List commands (context menu, bottom-bar popups and keys while the list has focus).
  const play = listCommand("play", "播放", canPlay, playSelected, ["Enter", "P"], {
    busy: () => player.busy(),
  });
  const remove = listCommand("remove", "移除所选", () => queue.selection.size > 0, removeSelected, [
    "Delete",
    "Backspace",
  ]);
  const removeAll = listCommand("clear", "移除全部", hasRows, guard(() => queue.clear()));
  const moves: Record<Placement, Command> = {
    top: listCommand("move-top", "置顶", () => queue.canPlace("top"), place("top"), ["Ctrl+Home"]),
    up: listCommand("move-up", "上移", () => queue.canPlace("up"), place("up"), ["Ctrl+ArrowUp"]),
    down: listCommand("move-down", "下移", () => queue.canPlace("down"), place("down"), ["Ctrl+ArrowDown"]),
    bottom: listCommand("move-bottom", "置底", () => queue.canPlace("bottom"), place("bottom"), ["Ctrl+End"]),
  };
  const sortable = () => queue.rows.length > 1;
  const sorts = [
    listCommand("sort-title", "按名称排序", sortable, sort("title")),
    listCommand("sort-path", "按路径排序", sortable, sort("path")),
    listCommand("sort-reverse", "倒序排列", sortable, sort("reverse")),
  ];
  const selectAll = listCommand("select-all", "全选", hasRows, () => queue.selectAll(), ["Ctrl+A"]);
  // F6 while the list has focus closes it (reference player), shown as "关闭列表" in the list menu.
  const close = listCommand("close", "关闭列表", () => true, () => sidebar.setVisible(false), ["F6"]);

  list.bind(() => {
    const known = queue.rows.map(lengthOf).filter((seconds): seconds is number => seconds !== null);
    // "+": some row's length is unavailable (ffprobe failed, or a URL that has not played).
    const partial = queue.rows.some(
      (row) => lengthOf(row) === null && (row.probe === "failed" || row.kind === "url"),
    );
    return {
      rows: queue.rows.map((row, index) => ({
        rowId: row.rowId,
        label: `${index + 1}. ${row.title}`,
        duration: lengthOf(row) !== null ? clock(lengthOf(row)!) : row.probe === "failed" ? "--:--" : "",
        tooltip: row.error ? `${row.source}\n${row.error}` : row.source,
        selected: queue.selection.has(row.rowId),
        current: row.rowId === queue.binding?.rowId,
        played: row.played,
        failed: row.status === "失败",
      })),
      total: known.length ? clock(known.reduce((sum, seconds) => sum + seconds, 0)) + (partial ? "+" : "") : "",
      detached: sidebar.state?.detached ?? false,
      canAdd: !adding,
      canRemove: queue.selection.size > 0,
      canSort: sortable(),
      canTop: queue.canPlace("top"),
      canUp: queue.canPlace("up"),
      canDown: queue.canPlace("down"),
      canBottom: queue.canPlace("bottom"),
    };
  });
  list.on("selectRow", (rowId: string, toggle: boolean, extend: boolean) => {
    if (extend) queue.extend(rowId);
    else if (toggle) queue.toggle(rowId);
    else queue.select(rowId);
  });
  list.on("playRow", (rowId: string) => {
    if (!queue.row(rowId)) return;
    queue.select(rowId);
    return playSelected();
  });
  list.on("selectStep", (delta: number, extend: boolean) => queue.selectNeighbor(delta, extend));
  list.on("dropRow", (rowId: string, index: number) => queue.drop(rowId, index));
  list.on("place", (where: string) => (where in moves ? queue.place(where as Placement) : undefined));
  list.on("remove", () => removeSelected());
  list.on("addMenu", (x: number, y: number) => list.showMenu([add, addLink], { x, y }));
  list.on("sortMenu", (x: number, y: number) => list.showMenu(sorts, { x, y }));
  list.on("contextMenu", (x: number, y: number) =>
    list.showMenu(
      [play, null, add, addLink, null, remove, removeAll, null, ...sorts, null, selectAll, null, close],
      { x, y },
    ),
  );
  list.on("hide", () => sidebar.setVisible(false));
  list.on("toggleDetached", () => sidebar.setDetached(!(sidebar.state?.detached ?? false)));
  list.on("focusChanged", (focused: boolean) => list.setInteraction({ focused }));
  list.on("dragging", (dragging: boolean) => list.setInteraction({ dragging }));

  // Other plugins (the bar, PgUp/PgDn) step through the list with player.next() / previous().
  plugin.player.provideSequence({
    canNext: () => !!queue.neighbor(1) && !player.busy(),
    canPrevious: () => !!queue.neighbor(-1) && !player.busy(),
    next: step(1),
    previous: step(-1),
  });
}
