import { readFile, stat } from "node:fs/promises";
import { basename, dirname } from "node:path";
import type { Command, CommandOptions, ConfirmOptions, Plugin, PluginError } from "../../sdk/plugin/index.ts";
import { dropExtra, DROP_EXTENSIONS, dropRow, expandDrop } from "./drop.ts";
import { follower, type OpenMode } from "./follow.ts";
import { linkDialog } from "./link.ts";
import {
  decodeList,
  formatList,
  LIST_ENTRY_LIMIT,
  LIST_SIZE_LIMIT,
  listName,
  parseList,
  savePath,
  writeAtomic,
} from "./listfile.ts";
import { extensionOf, folderError, isMedia, LIST_FILES, SCAN_LIMIT, scanFolder, siblings } from "./media.ts";
import { playback } from "./playback.ts";
import { Prober, toolRunner } from "./probe.ts";
import { clock, lengthOf, Queue, type Placement, type Repeat, type Row, type SortKey } from "./queue.ts";
import { centered, ensureVisible, inView, USER_SCROLL_HOLD_MS } from "./scroll.ts";
import { defaults, Store, type Saved } from "./store.ts";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const repeats: [Repeat, string][] = [
  ["none", "不循环"],
  ["all", "全部"],
  ["one", "单个"],
];
const openModes: [OpenMode, string][] = [
  ["only", "仅打开该文件"],
  ["similar", "加入相似文件"],
  ["folder", "加入同文件夹全部文件"],
];
/** C11 sort keys with their list-focus keys (Ctrl+3 … Ctrl+0), then the unbound ones. */
const sortKeys: [SortKey, string, string | null][] = [
  ["title", "按标题", "Ctrl+3"],
  ["filename", "按文件名", "Ctrl+4"],
  ["extension", "按扩展名", "Ctrl+5"],
  ["size", "按大小", "Ctrl+6"],
  ["duration", "按时长", "Ctrl+7"],
  ["date", "按日期", "Ctrl+8"],
  ["shuffle", "乱序", "Ctrl+9"],
  ["folder", "按文件夹", "Ctrl+0"],
  ["path", "按路径排序", null],
  ["reverse", "倒置当前顺序", null],
];
const STAT_LIMIT = 8;
const SEARCH_DEBOUNCE_MS = 150;
/** IA-PL-02: the page id of 选项 ▶ 播放 ▶ 播放列表. */
export const SETTINGS_PAGE = "playlist";
/** C23: at most this many selected files are shown in the file manager. */
export const REVEAL_LIMIT = 20;

/** Injected in tests: the sort's shuffle source. */
export type Options = { random: () => number };

/** Run `work` over `items`, at most `limit` at a time. */
async function pool<T>(items: T[], limit: number, work: (item: T) => Promise<void>) {
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (next < items.length) await work(items[next++]!);
    }),
  );
}

export default async function activate(plugin: Plugin, options: Partial<Options> = {}) {
  const random = options.random ?? Math.random;
  const queue = new Queue();
  queue.say = (text) => void plugin.osd.show(text.slice(0, 200)).catch(() => {});
  const prober = new Prober(queue, toolRunner(plugin.tools), () => plugin.update());
  queue.removed = (rowId) => prober.cancel(rowId);
  plugin.onDispose(() => prober.cancelAll());
  let follow: ReturnType<typeof follower> | null = null;
  const player = playback(plugin, queue, () => void follow?.check());
  const link = linkDialog(plugin, queue);
  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "播放列表" });
  const sidebar = plugin.panels.sidebar;
  const stop = new AbortController();
  plugin.onDispose(() => stop.abort());

  // C17: restore the settings and (when remembered) the list; nothing is opened.
  let store: Store | null = null;
  try {
    store = new Store(plugin);
  } catch {
    /* no data directory: nothing is remembered */
  }
  const loaded = store?.load() ?? { settings: defaults(), list: null };
  const settings = loaded.settings;
  queue.repeat = settings.repeat;
  queue.setShuffle(settings.shuffle);
  if (settings.remember && loaded.list) Store.restore(queue, loaded.list);
  prober.import(store?.cache.value);
  player.setFast(settings.gapless);
  const snapshot = (): [Saved, string] => {
    settings.repeat = queue.repeat;
    settings.shuffle = queue.shuffle;
    const value = Store.snapshot(queue, settings);
    return [value, JSON.stringify(value)];
  };
  let saved = snapshot()[1];
  /** Hand changes to the stores; they write within a second (C17). */
  const persist = () => {
    if (!store) return;
    const [value, json] = snapshot();
    if (json !== saved) store.lists.save(value);
    saved = json;
    if (prober.dirty) store.cache.save(prober.export());
  };
  /** Write now (turning "记住列表" off removes the saved list at once). */
  const flush = async () => {
    persist();
    try {
      await Promise.all([store?.lists.flush(), store?.cache.flush()]);
    } catch (error) {
      queue.message = `无法保存列表：${errorText(error)}`;
    }
  };
  // Registered after the stores: disposal runs it first, then the stores write what it handed them.
  plugin.onDispose(persist);
  prober.scan();

  let adding = false,
    scanning = false,
    checking = false,
    statting = false,
    listBusy = false;
  let lastList: string | null = null;
  let focusSearch = 0;
  const focus = { list: false, search: false };

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
  /** A removal took away the row being opened: its late reply must not bind it. */
  const removing = (remove: () => void) => {
    const pending = player.pendingRow;
    remove();
    if (pending && !queue.row(pending)) {
      queue.epoch++;
      queue.notice("正在打开的媒体已移出列表");
    }
  };

  /**
   * A system confirmation (`dialogs.confirm`). Another dialog still open
   * (`busy`) is a notice and counts as cancelled; other failures go to `guard`.
   */
  const ask = async (options: ConfirmOptions) => {
    try {
      return await plugin.dialogs.confirm(options);
    } catch (error) {
      if ((error as PluginError).code !== "busy") throw error;
      queue.notice("请先关闭另一个对话框");
      return { confirmed: false, checked: false };
    }
  };
  /** At most `max` characters of a name shown in a confirmation (its message is 1–200 characters). */
  const fitName = (name: string, max = 120) => ([...name].length > max ? `${[...name].slice(0, max - 1).join("")}…` : name);

  /** 移除全部: asks first unless 「不再询问」 was ticked on a confirmed clear (saved with the settings). */
  const clearAll = guard(async () => {
    if (!queue.rows.length) return;
    if (settings.confirmClear) {
      const answer = await ask({
        message: `移除播放列表中的全部 ${queue.rows.length} 项？`,
        detail: "正在播放的媒体继续播放。",
        confirm: "移除全部",
        destructive: true,
        checkbox: "不再询问",
      });
      if (!answer.confirmed) return;
      if (answer.checked) settings.confirmClear = false;
    }
    removing(() => queue.clear());
  });

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

  /** A03 (`play`) / C04: add a folder's media files; A03 then plays the first of them. */
  const addFolder = (play: boolean) =>
    guard(async () => {
      if (scanning) return;
      scanning = true;
      try {
        const folder = await plugin.dialogs.openDirectory({ title: play ? "打开文件夹" : "添加文件夹到播放列表" });
        if (!folder) return;
        const slow = setTimeout(() => queue.notice("正在扫描文件夹…"), 1000);
        let scan;
        try {
          scan = await scanFolder(folder, { signal: stop.signal });
        } catch (error) {
          if (stop.signal.aborted) return;
          return queue.notice(`无法读取文件夹：${folderError(error)}`);
        } finally {
          clearTimeout(slow);
        }
        if (!scan.files.length) return queue.notice("文件夹中没有可播放的文件");
        // IA-PL-01: 打开文件夹 replaces the list too (not while another source opens; 「追加」 adds).
        if (play && !settings.appendOnOpen && queue.rows.length && !player.busy()) removing(() => queue.clear(false));
        const wasEmpty = queue.rows.length === 0;
        const result = await queue.addEntries(
          scan.files.map((file) => ({ kind: "file" as const, source: file.path, key: file.key })),
        );
        prober.scan();
        const extra = [
          scan.unreadable ? `${scan.unreadable} 个子文件夹无法读取` : "",
          scan.truncated ? `已达上限 ${SCAN_LIMIT}` : "",
        ]
          .filter(Boolean)
          .join("，");
        const summary = queue.summary(result, extra);
        if (!play) {
          queue.selectFirstAdded(result, wasEmpty);
          return queue.notice(summary);
        }
        // The first new row, or (all duplicates) the row of the folder's first file.
        const target = result.fresh[0] ?? result.rows.find((row) => !!row);
        if (!target) return queue.notice(summary);
        if (player.busy()) return queue.notice(`${summary}；正在打开另一个来源，只加入列表`);
        queue.select(target.rowId);
        queue.notice(summary);
        await player.play(target.rowId, undefined, { quiet: true });
      } finally {
        scanning = false;
      }
    });
  const openFolder = addFolder(true);
  const addFolderOnly = addFolder(false);

  const playSelected = guard(() => (canPlay() && queue.selected ? player.play(queue.selected) : undefined));
  const step = (delta: 1 | -1) =>
    guard(() => {
      const row = !player.busy() ? queue.neighbor(delta) : undefined;
      return row ? player.play(row.rowId) : undefined;
    });
  const removeSelected = guard(() => removing(() => queue.removeSelected()));
  /** C07: keep the selection, remove the other shown rows. */
  const removeUnselected = guard(() => {
    let count = 0;
    removing(() => (count = queue.removeUnselected()));
    queue.notice(`已移除 ${count} 项`);
  });
  /** C07: remove local rows whose file is gone (not ones that are only unreadable now). */
  const removeInvalid = guard(async () => {
    if (checking) return;
    checking = true;
    try {
      const doomed: string[] = [];
      await pool(
        queue.visible().filter((row) => row.kind === "file"),
        STAT_LIMIT,
        async (row) => {
          try {
            if (!(await stat(row.source)).isFile()) doomed.push(row.rowId);
          } catch (error) {
            const code = (error as NodeJS.ErrnoException).code;
            if (code === "ENOENT" || code === "ENOTDIR") doomed.push(row.rowId);
          }
        },
      );
      const present = doomed.filter((id) => queue.row(id));
      removing(() => present.forEach((id) => queue.removeRow(id)));
      queue.notice(present.length ? `已移除 ${present.length} 个无效项` : "没有无效项");
    } finally {
      checking = false;
    }
  });
  /** G-08: the keyboard cursor is scrolled into view at the next update. */
  let revealCursor = false;
  const place = (where: Placement) => () => {
    queue.place(where);
    revealCursor = true;
  };

  /** C11: size and date need `stat` (8 at a time, not cached). */
  const sortBy = (key: SortKey) =>
    guard(async () => {
      if (statting || queue.rows.length < 2) return;
      let values: Map<string, number> | undefined;
      if (key === "size" || key === "date") {
        statting = true;
        try {
          values = new Map();
          const found = values;
          await pool(
            queue.rows.filter((row) => row.kind === "file"),
            STAT_LIMIT,
            async (row) => {
              try {
                const info = await stat(row.source);
                found.set(row.rowId, key === "size" ? info.size : info.mtimeMs);
              } catch {
                /* unreadable: sorted last */
              }
            },
          );
        } finally {
          statting = false;
        }
      }
      queue.sort(key, { values, random });
    });
  const setDirection = (descending: boolean) =>
    guard(() => {
      queue.descending = descending;
      return sortBy(queue.sortKey ?? "title")();
    });

  /**
   * C23: the selected local files (not links, at most 20), one file manager window per
   * folder showing its first file that still exists; else the folder itself.
   */
  const revealSelected = guard(async () => {
    const paths = queue.marked().filter((row) => row.kind === "file").map((row) => row.source);
    const notices: string[] = [];
    if (paths.length > REVEAL_LIMIT) notices.push(`只显示前 ${REVEAL_LIMIT} 个文件`);
    const folders = new Map<string, string[]>();
    for (const path of paths.slice(0, REVEAL_LIMIT)) folders.set(dirname(path), [...(folders.get(dirname(path)) ?? []), path]);
    const missing = (error: unknown) => {
      if ((error as PluginError).code !== "missing") throw error;
    };
    for (const [dir, files] of folders) {
      let shown = false;
      for (const file of files) {
        try {
          await plugin.system.reveal(file);
          shown = true;
          break;
        } catch (error) {
          missing(error);
        }
      }
      if (shown) continue;
      try {
        await plugin.system.openPath(dir);
      } catch (error) {
        missing(error);
        notices.push(`找不到文件：${basename(files[0]!)}`);
      }
    }
    if (notices.length) queue.notice(notices.join("；"));
  });

  /** C18: replace the list with a list file's entries (after confirmation when not empty). */
  const openList = guard(async () => {
    if (listBusy) return;
    listBusy = true;
    try {
      const path = await plugin.dialogs.openFile({ title: "打开播放列表文件" });
      if (!path) return;
      const ext = extensionOf(path);
      if (!LIST_FILES.has(ext)) return queue.notice("不支持的列表格式");
      let parsed;
      try {
        if ((await stat(path)).size > LIST_SIZE_LIMIT) return queue.notice("无法打开列表：文件超过 16 MB");
        parsed = parseList(decodeList(await readFile(path), ext), ext, dirname(path));
      } catch (error) {
        return queue.notice(`无法打开列表：${errorText(error)}`);
      }
      if (
        queue.rows.length &&
        !(await ask({ message: `用 ${fitName(basename(path))} 替换当前列表？`, confirm: "替换" })).confirmed
      )
        return;
      const entries = parsed.entries.slice(0, LIST_ENTRY_LIMIT);
      removing(() => queue.clear());
      const result = await queue.addEntries(
        entries.map((entry) => ({
          kind: entry.kind,
          source: entry.source,
          named: entry.title,
          duration: entry.duration,
          missing: "文件不存在",
        })),
      );
      if (result.fresh[0]) queue.selected = result.fresh[0].rowId;
      prober.scan();
      lastList = listName(path);
      const skipped = parsed.skipped + (parsed.entries.length - entries.length) + result.duplicate + result.refused;
      queue.notice(`已打开列表：${lastList}（${result.added} 项）${skipped ? `，跳过 ${skipped} 项` : ""}`);
    } finally {
      listBusy = false;
    }
  });
  /** C18: save every row as `.m3u8` (default), `.m3u` or `.pls`. */
  const saveList = guard(async () => {
    if (listBusy) return;
    listBusy = true;
    try {
      const chosen = await plugin.dialogs.saveFile({
        title: "保存播放列表",
        fileName: `${lastList ?? "播放列表"}.m3u8`,
      });
      if (!chosen) return;
      const { path, ext } = savePath(chosen);
      try {
        const rows = queue.rows.map((row) => ({ kind: row.kind, source: row.source, title: row.title, duration: lengthOf(row) }));
        writeAtomic(path, formatList(rows, path, ext));
      } catch (error) {
        return queue.notice(`无法保存列表：${errorText(error)}`);
      }
      lastList = listName(path);
      queue.notice(`已保存列表：${basename(path)}`);
    } finally {
      listBusy = false;
    }
  });

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

  // Main menu (allocation table: 打开 ▶ 110, 播放 ▶ 播放列表 ▶ 230–249, windows 800).
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
  plugin.commands.add({
    id: "playlist.open-folder",
    path: ["打开", "打开文件夹…"],
    group: "open",
    order: 110,
    bindings: ["F2"],
    busy: () => scanning,
    reason: () => (scanning ? "正在扫描文件夹" : null),
    run: openFolder,
  });
  const playlistMenu = (id: string, title: string[], order: number, options: Partial<CommandOptions>) =>
    plugin.commands.add({ id: `playlist.${id}`, path: ["播放", "播放列表", ...title], order, run: () => {}, ...options });
  const addingFiles = { enabled: () => !adding, busy: () => adding };
  const scanningFolder = { busy: () => scanning, reason: () => (scanning ? "正在扫描文件夹" : null) };
  const listFile = { busy: () => listBusy, group: "list" };
  playlistMenu("add", ["添加文件…"], 230, { ...addingFiles, run: addFiles });
  playlistMenu("link", ["添加链接…"], 231, { run: guard(() => link.show()) });
  playlistMenu("add-folder", ["添加文件夹…"], 232, { ...scanningFolder, run: addFolderOnly });
  // The options shared by the menu and the settings page (IA-PL-02).
  const setRepeat = (mode: Repeat) => {
    queue.repeat = mode;
    queue.notice(`循环：${repeats.find(([each]) => each === mode)![1]}`);
  };
  const setShuffle = (on: boolean) => {
    queue.setShuffle(on);
    queue.notice(queue.shuffle ? "随机播放：开" : "随机播放：关");
  };
  const setFast = (on: boolean) => {
    settings.gapless = on;
    player.setFast(settings.gapless);
    queue.notice(settings.gapless ? "快速推进到下一个：开" : "快速推进到下一个：关");
  };
  const setRemember = (on: boolean) => {
    settings.remember = on;
    void flush();
    queue.notice(settings.remember ? "将记住列表" : "不再记住列表");
  };
  const setConfirmClear = (on: boolean) => {
    settings.confirmClear = on;
    queue.notice(settings.confirmClear ? "移除全部前确认：开" : "移除全部前确认：关");
  };
  repeats.forEach(([mode, title], index) =>
    playlistMenu(`repeat-${mode}`, ["循环", title], 233 + index, {
      radio: true,
      checked: () => queue.repeat === mode,
      run: () => setRepeat(mode),
    }),
  );
  playlistMenu("shuffle", ["随机"], 236, {
    checked: () => queue.shuffle,
    run: () => setShuffle(!queue.shuffle),
  });
  playlistMenu("open-list", ["打开列表文件…"], 237, { ...listFile, run: openList });
  playlistMenu("save-list", ["保存列表…"], 238, {
    ...listFile,
    enabled: hasRows,
    reason: reason(hasRows),
    run: saveList,
  });
  openModes.forEach(([mode, title], index) =>
    playlistMenu(`open-mode-${mode}`, ["打开文件时", title], 239 + index, {
      group: "open-mode",
      radio: true,
      checked: () => settings.openMode === mode,
      run: () => {
        settings.openMode = mode;
      },
    }),
  );
  playlistMenu("fast-advance", ["快速推进到下一个"], 242, {
    group: "options",
    checked: () => settings.gapless,
    run: () => setFast(!settings.gapless),
  });
  playlistMenu("remember", ["记住列表"], 243, {
    group: "options",
    checked: () => settings.remember,
    run: () => setRemember(!settings.remember),
  });
  // Turns the 移除全部 question back on after 「不再询问」 (or off).
  playlistMenu("confirm-clear", ["移除全部前确认"], 244, {
    group: "options",
    checked: () => settings.confirmClear,
    run: () => setConfirmClear(!settings.confirmClear),
  });

  // List commands (context menu, bottom-bar popups and keys while the list has focus).
  const play = listCommand("play", "播放", canPlay, playSelected, ["Enter", "P"], {
    busy: () => player.busy(),
  });
  const add = listCommand("list-add", "添加文件…", () => !adding, addFiles, ["Ctrl+I"], { busy: () => adding });
  const addFolderCommand = listCommand("list-add-folder", "添加文件夹…", () => true, addFolderOnly, ["Ctrl+O"], scanningFolder);
  const addLink = listCommand("list-link", "添加链接…", () => true, guard(() => link.show()), ["Ctrl+U"]);
  const remove = listCommand("remove", "移除所选", () => queue.selection.size > 0, removeSelected, [
    "Delete",
    "Backspace",
  ]);
  const removeOthers = listCommand(
    "remove-unselected",
    "移除未选",
    () => queue.canRemoveUnselected(),
    removeUnselected,
    ["Ctrl+Delete"],
  );
  const removeInvalidCommand = listCommand("remove-invalid", "移除无效项", hasRows, removeInvalid, ["Ctrl+T"], {
    busy: () => checking,
  });
  const removeAll = listCommand("clear", "移除全部", hasRows, clearAll, ["Alt+Delete"]);
  const hasLocal = () => queue.marked().some((row) => row.kind === "file");
  const revealCommand = listCommand("reveal", "在 Finder 中显示", hasLocal, revealSelected, ["Ctrl+F"], {
    reason: () => (hasLocal() ? null : queue.selection.size ? "链接没有本地位置" : "当前没有可操作项"),
  });
  const moveReason = (where: Placement) => () =>
    queue.canPlace(where) ? null : queue.filtering ? "搜索时不能调整顺序" : "当前没有可操作项";
  const move = (id: string, title: string, where: Placement, binding: string) =>
    listCommand(id, title, () => queue.canPlace(where), place(where), [binding], { reason: moveReason(where) });
  const moves: Record<Placement, Command> = {
    top: move("move-top", "置顶", "top", "Ctrl+Home"),
    up: move("move-up", "上移", "up", "Ctrl+ArrowUp"),
    down: move("move-down", "下移", "down", "Ctrl+ArrowDown"),
    bottom: move("move-bottom", "置底", "bottom", "Ctrl+End"),
  };
  const sortable = () => queue.rows.length > 1;
  const sortState = { busy: () => statting };
  const sorts = [
    listCommand("sort-ascending", "正序", () => true, setDirection(false), ["Ctrl+1"], {
      ...sortState,
      radio: true,
      checked: () => !queue.descending,
    }),
    listCommand("sort-descending", "反序", () => true, setDirection(true), ["Ctrl+2"], {
      ...sortState,
      radio: true,
      checked: () => queue.descending,
    }),
    ...sortKeys.map(([key, title, binding]) =>
      listCommand(`sort-${key}`, title, sortable, sortBy(key), binding ? [binding] : undefined, sortState),
    ),
  ];
  const selectAll = listCommand("select-all", "全选", () => queue.visible().length > 0, () => queue.selectAll(), [
    "Ctrl+A",
  ]);
  const search = listCommand("search", "搜索", () => true, () => void focusSearch++, ["Ctrl+S"]);
  const openListCommand = listCommand("list-open-list", "打开列表文件…", () => true, openList, ["F3"], {
    busy: () => listBusy,
  });
  const saveListCommand = listCommand("list-save-list", "保存列表…", hasRows, saveList, ["F2"], {
    busy: () => listBusy,
  });
  // F6 while the list has focus closes it (reference player), shown as "关闭列表" in the list menu.
  const close = listCommand("close", "关闭列表", () => true, () => sidebar.setVisible(false), ["F6"]);
  // IA-PL-02: F5 while the list has focus opens its settings page (参考播放器播放列表设置);
  // elsewhere F5 stays the global 选项….
  listCommand("settings", "播放列表设置…", () => true, guard(() => plugin.settings.show(SETTINGS_PAGE)), ["F5"]);

  // IA-PL-02: 选项 ▶ 播放 ▶ 播放列表, in step with the menu items.
  const page = plugin.settings.page("ui/settings.slint", {
    id: SETTINGS_PAGE,
    category: "playback",
    title: "播放列表",
    order: 30,
  });
  page.bind(() => ({
    openMode: openModes.findIndex(([mode]) => mode === settings.openMode),
    appendOnOpen: settings.appendOnOpen,
    dropAtPointer: settings.dropAtPointer,
    repeat: repeats.findIndex(([mode]) => mode === queue.repeat),
    shuffle: queue.shuffle,
    fastAdvance: settings.gapless,
    remember: settings.remember,
    confirmClear: settings.confirmClear,
  }));
  page.on("setOpenMode", (index: number) => {
    const mode = openModes[index]?.[0];
    if (mode) settings.openMode = mode;
  });
  page.on("setAppendOnOpen", (on: boolean) => void (settings.appendOnOpen = on));
  page.on("setDropAtPointer", (on: boolean) => void (settings.dropAtPointer = on));
  page.on("setRepeat", (index: number) => {
    const mode = repeats[index]?.[0];
    if (mode && mode !== queue.repeat) setRepeat(mode);
  });
  page.on("setShuffle", (on: boolean) => (on !== queue.shuffle ? setShuffle(on) : undefined));
  page.on("setFastAdvance", (on: boolean) => (on !== settings.gapless ? setFast(on) : undefined));
  page.on("setRemember", (on: boolean) => (on !== settings.remember ? setRemember(on) : undefined));
  page.on("setConfirmClear", (on: boolean) => (on !== settings.confirmClear ? setConfirmClear(on) : undefined));

  // G-08: scrolling the list from the plugin (`scroll-to-y` on each `scroll-request`).
  const listArea = { scroll: 0, height: Number.POSITIVE_INFINITY };
  const scroller = { request: 0, to: 0, sentAt: Number.NEGATIVE_INFINITY, userAt: Number.NEGATIVE_INFINITY };
  let lastPlaying: string | null = null;
  /** The scroll this update asks for: the cursor after a key, else a newly playing row out of view. */
  const scrollTarget = (shown: Row[]): number | null => {
    const playing = queue.binding?.rowId ?? null;
    const playingChanged = playing !== lastPlaying;
    lastPlaying = playing;
    const reveal = revealCursor;
    revealCursor = false;
    if (!Number.isFinite(listArea.height)) return null;
    const at = (rowId: string | null) => shown.findIndex((row) => row.rowId === rowId);
    if (reveal) {
      const index = at(queue.selected);
      if (index >= 0) return ensureVisible(index, listArea.scroll, listArea.height, shown.length);
    }
    // Not while the user browses (scrolled lately), nor for a row the search hides.
    if (!playingChanged || !playing || Date.now() - scroller.userAt < USER_SCROLL_HOLD_MS) return null;
    const index = at(playing);
    if (index < 0 || inView(index, listArea.scroll, listArea.height)) return null;
    return centered(index, listArea.height, shown.length);
  };

  list.bind(() => {
    persist();
    player.prepare();
    const shown = queue.visible();
    const target = scrollTarget(shown);
    if (target !== null && target !== listArea.scroll) {
      scroller.request++;
      scroller.to = target;
      scroller.sentAt = Date.now();
      listArea.scroll = target;
    }
    const index = new Map(queue.rows.map((row, i) => [row.rowId, i]));
    const known = shown.map(lengthOf).filter((seconds): seconds is number => seconds !== null);
    // "+": some row's length is unavailable (ffprobe failed, or a URL that has not played).
    const partial = shown.some((row) => lengthOf(row) === null && (row.probe === "failed" || row.kind === "url"));
    const total = known.length ? clock(known.reduce((sum, seconds) => sum + seconds, 0)) + (partial ? "+" : "") : "";
    return {
      rows: shown.map((row: Row) => ({
        rowId: row.rowId,
        label: `${index.get(row.rowId)! + 1}. ${row.title}`,
        duration: lengthOf(row) !== null ? clock(lengthOf(row)!) : row.probe === "failed" ? "--:--" : "",
        tooltip: row.error ? `${row.source}\n${row.error}` : row.source,
        selected: queue.selection.has(row.rowId),
        current: row.rowId === queue.binding?.rowId,
        played: row.played,
        failed: row.status === "失败",
      })),
      total: queue.filtering && total ? `筛选 ${total}` : total,
      filtering: queue.filtering,
      focusSearch,
      detached: sidebar.state?.detached ?? false,
      canAdd: !adding,
      canRemove: queue.selection.size > 0,
      canSort: sortable(),
      canDrag: !queue.filtering,
      canTop: queue.canPlace("top"),
      canUp: queue.canPlace("up"),
      canDown: queue.canPlace("down"),
      canBottom: queue.canPlace("bottom"),
      scrollToY: scroller.to,
      scrollRequest: scroller.request,
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
  list.on("selectStep", (delta: number, extend: boolean) => {
    queue.selectNeighbor(delta, extend);
    revealCursor = true;
  });
  list.on("dropRow", (rowId: string, index: number) => queue.drop(rowId, index));
  list.on("place", (where: string) => (where in moves ? place(where as Placement)() : undefined));
  list.on("remove", () => removeSelected());
  list.on("addMenu", (x: number, y: number) => list.showMenu([add, addFolderCommand, addLink], { x, y }));
  list.on("sortMenu", (x: number, y: number) => list.showMenu(sorts, { x, y }));
  list.on("contextMenu", (x: number, y: number) =>
    list.showMenu(
      [
        play,
        null,
        add,
        addFolderCommand,
        addLink,
        null,
        remove,
        removeOthers,
        removeInvalidCommand,
        removeAll,
        null,
        revealCommand,
        null,
        selectAll,
        search,
        null,
        openListCommand,
        saveListCommand,
        null,
        close,
      ],
      { x, y },
    ),
  );
  list.on("hide", () => sidebar.setVisible(false));
  list.on("toggleDetached", () => sidebar.setDetached(!(sidebar.state?.detached ?? false)));
  const interact = () => list.setInteraction({ focused: focus.list || focus.search });
  list.on("focusChanged", (focused: boolean) => {
    focus.list = focused;
    interact();
  });
  list.on("dragging", (dragging: boolean) => list.setInteraction({ dragging }));

  // C21: the search box filters after 150 ms; Esc (empty text) applies at once.
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  plugin.onDispose(() => void (searchTimer && clearTimeout(searchTimer)));
  list.on("searchChanged", (text: string) => {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = null;
    if (!text.trim()) return queue.setFilter("");
    searchTimer = setTimeout(() => {
      searchTimer = null;
      queue.setFilter(text);
      plugin.update();
    }, SEARCH_DEBOUNCE_MS);
  });
  list.on("searchFocus", (focused: boolean) => {
    focus.search = focused;
    interact();
  });
  /** Enter in the search box: the cursor row when it is shown and selected, else the first match. */
  list.on("searchAccept", () => {
    const shown = queue.visible();
    const cursor = shown.find((row) => row.rowId === queue.selected && queue.selection.has(row.rowId));
    const row = cursor ?? shown[0];
    if (!row) return;
    queue.select(row.rowId);
    return playSelected();
  });
  /** ↓ in the search box: back to the list on the first match. */
  list.on("searchDown", () => {
    const first = queue.visible()[0];
    if (first) queue.select(first.rowId);
    revealCursor = true;
  });

  // The list reports its scroll and height (drops, G-08). A scroll that is not the one just
  // asked for is the user's (wheel, scroll bar, edge scrolling while dragging).
  list.on("listArea", (scroll: number, height: number) => {
    const next = Number.isFinite(scroll) ? scroll : 0;
    const ours = Math.abs(next - scroller.to) < 1 && Date.now() - scroller.sentAt < 500;
    if (Math.abs(next - listArea.scroll) >= 1 && !ours) scroller.userAt = Date.now();
    listArea.scroll = next;
    listArea.height = Number.isFinite(height) && height > 0 ? height : Number.POSITIVE_INFINITY;
  });

  // A15: drops from the file manager. On the video area (the player groups by extension): G-11
  // replace the list (「追加」 option: add to the end) and play the first item; Option only adds.
  // On the list (G-12): add to the end, or before the row under the drop point with
  // 「插入到拖放位置」; never play. One drop at a time, in order.
  let dropping: Promise<void> = Promise.resolve();
  const dropped = (
    paths: string[],
    how: { play: boolean; clear?: boolean; expand?: boolean; at?: { x: number; y: number } },
  ) =>
    (dropping = dropping.then(
      guard(async () => {
        const expanded = await expandDrop(paths, { signal: stop.signal });
        if (stop.signal.aborted) return;
        // A15 / IA-PL-01 (reference player): a single media file dropped on the video area brings its
        // similar files (or its group's whole folder) like an open does; several files never do.
        let start: string | null = null;
        const single = expanded.entries.length === 1 ? expanded.entries[0]! : null;
        if (how.expand && paths.length === 1 && single?.kind === "file" && single.source === paths[0] && isMedia(single.source)) {
          start = single.source;
          if (settings.openMode !== "only") {
            try {
              const found = await siblings(start, settings.openMode === "similar" ? "similar" : "folder");
              expanded.entries = found.paths.map((path) => ({ kind: "file" as const, source: path }));
              expanded.truncated ||= found.truncated;
            } catch {
              /* the folder is unreadable: only this file */
            }
          }
        }
        const extra = dropExtra(expanded);
        if (!expanded.entries.length)
          return queue.notice(extra ? `没有可加入的项目：${extra}` : "没有可加入的项目");
        let before: number | undefined;
        if (how.at) {
          const shown = queue.visible();
          const row = dropRow(how.at.y, listArea.scroll, listArea.height, shown.length);
          if (row !== null) before = queue.rows.indexOf(shown[row]!);
        }
        // G-11: replaced without asking; not while another source opens (only added then).
        if (how.clear && queue.rows.length && !player.busy()) removing(() => queue.clear(false));
        const wasEmpty = queue.rows.length === 0;
        const result = await queue.addEntries(expanded.entries, before === undefined ? {} : { before });
        prober.scan();
        const summary = queue.summary(result, extra);
        if (!how.play) {
          queue.selectFirstAdded(result, wasEmpty);
          return queue.notice(summary);
        }
        // The dropped file when it brought its siblings; else the first new row, or (all
        // duplicates) the existing row of the first item.
        const at = start === null ? -1 : expanded.entries.findIndex((entry) => entry.source === start);
        const target = (at >= 0 ? result.rows[at] : undefined) ?? result.fresh[0] ?? result.rows.find((row) => !!row);
        if (!target) return queue.notice(summary);
        if (player.busy()) return queue.notice(`${summary}；正在打开，已加入列表`);
        queue.select(target.rowId);
        queue.notice(summary);
        await player.play(target.rowId, undefined, { quiet: true });
      }),
    ));
  plugin.drops.provide({
    extensions: DROP_EXTENSIONS,
    drop: ({ paths, modifiers }) =>
      dropped(paths, { play: !modifiers.alt, clear: !modifiers.alt && !settings.appendOnOpen, expand: !modifiers.alt }),
  });
  list.onDrop(({ paths, at }) => dropped(paths, { play: false, ...(settings.dropAtPointer ? { at } : {}) }));

  // Other plugins (the bar, PgUp/PgDn) step through the list with player.next() / previous().
  plugin.player.provideSequence({
    canNext: () => !!queue.neighbor(1) && !player.busy(),
    canPrevious: () => !!queue.neighbor(-1) && !player.busy(),
    next: step(1),
    previous: step(-1),
  });

  // A13: sources other plugins open (and one already open at start-up, C17).
  follow = follower(plugin, queue, player, () => settings.openMode, () => settings.appendOnOpen, () => {
    prober.scan();
    plugin.update();
  });
  plugin.player.onChange((state) => follow?.check(state));
  void follow.check();
}
