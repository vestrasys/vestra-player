import { readFile, stat } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import type { Command, CommandOptions, ConfirmOptions, ImageRef, Panel, Plugin, PluginError } from "@vestra-player/sdk";
import {
  DROP_EXTENSIONS,
  dropProblems,
  dropRow,
  expandDrop,
  LIST_TOP,
  nothingToAdd,
  ROW_HEIGHT,
  THUMB_ROW_HEIGHT,
} from "./drop.ts";
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
import { extensionOf, folderError, isMedia, LIST_FILES, mediaKind, scanFolder, siblings } from "./media.ts";
import { failureText, fitName, type NoticeArgs, type NoticeText } from "./notices.ts";
import { playback } from "./playback.ts";
import { Prober, toolRunner } from "./probe.ts";
import { clock, lengthOf, Queue, type Placement, type Repeat, type Row, type SortKey } from "./queue.ts";
import {
  centered,
  ensureVisible,
  inView,
  limit,
  SHOW_SETTLE_MS,
  THUMB_MARGIN,
  THUMB_SEND_MARGIN,
  USER_SCROLL_HOLD_MS,
} from "./scroll.ts";
import { defaults, PERSIST_CHECK_MS, Store, type Saved } from "./store.ts";
import { ffmpegRunner, HOLD_AFTER_OPEN_MS, THUMB_WIDTH, Thumbnailer } from "./thumbs.ts";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const repeats = [
  ["none", "Off"],
  ["all", "All"],
  ["one", "One"],
  ["once", "Stop After Current"],
] as const satisfies readonly (readonly [Repeat, string])[];
const openModes = [
  ["only", "Open Only That File"],
  ["similar", "Add Similar Files"],
  ["folder", "Add All Files in the Folder"],
] as const satisfies readonly (readonly [OpenMode, string])[];
/** C11 sort keys with their list-focus keys (Ctrl+3 … Ctrl+0), then the unbound ones. */
const sortKeys = [
  ["title", "By Title", "Ctrl+3"],
  ["filename", "By File Name", "Ctrl+4"],
  ["extension", "By Extension", "Ctrl+5"],
  ["size", "By Size", "Ctrl+6"],
  ["duration", "By Duration", "Ctrl+7"],
  ["date", "By Date", "Ctrl+8"],
  ["shuffle", "Random Order", "Ctrl+9"],
  ["folder", "By Folder", "Ctrl+0"],
  ["path", "Sort by Path", null],
  ["reverse", "Reverse Current Order", null],
] as const satisfies readonly (readonly [SortKey, string, string | null])[];
/** The titles of the list commands (context menu, bottom-bar popups, list-focus keys). */
type ListTitle =
  | "Play"
  | "Add Files…"
  | "Add Folder…"
  | "Add Link…"
  | "Remove Selected"
  | "Remove Unselected"
  | "Remove Invalid Items"
  | "Remove All"
  | "Show in Folder"
  | "Move to Top"
  | "Move Up"
  | "Move Down"
  | "Move to Bottom"
  | "Ascending"
  | "Descending"
  | (typeof sortKeys)[number][1]
  | "Select All"
  | "Search"
  | "Open List File…"
  | "Save List…"
  | "Close List"
  | "Playlist Settings…";
/** Why a command is disabled or busy (shown by the menu). */
type Reason =
  | "Opening, or the result is not known yet"
  | "Nothing to act on"
  | "Scanning folder"
  | "Links have no local location"
  | "The order cannot change while searching";
/** Command options whose `reason` is one of `Reason`, without the title and path (so the texts are found for translation). */
type CommandSetup = Partial<Omit<CommandOptions, "reason" | "title" | "path">> & { reason?: () => Reason | null };
/** The former Playback ▸ Playlist ▸ … items, now in the command palette only (menu sweep B5); "Repeat" and "When Opening a File" are their categories. */
type PlaylistItem =
  | "Add Files…"
  | "Add Link…"
  | "Add Folder…"
  | "Repeat"
  | (typeof repeats)[number][1]
  | "Shuffle"
  | "Open List File…"
  | "Save List…"
  | "When Opening a File"
  | (typeof openModes)[number][1]
  | "Fast Advance to Next"
  | "Remember List"
  | "Confirm Before Removing All";
const STAT_LIMIT = 8;
const SEARCH_DEBOUNCE_MS = 150;

/** The confirmations this plugin asks (`dialogs.confirm`; the player translates the texts). */
type Question = Omit<ConfirmOptions, "cancel"> &
  (
    | {
        message: "Remove all {} items from the playlist?";
        detail: "The media that is playing continues to play.";
        confirm: "Remove All";
        checkbox: "Don't ask again";
      }
    | { message: "Replace the current list with {}?"; confirm: "Replace"; detail?: never; checkbox?: never }
  );

/** IA-PL-02: the page id of Options ▸ Playback ▸ Playlist. */
export const SETTINGS_PAGE = "playlist";
/** C23: at most this many selected files are shown in the file manager. */
export const REVEAL_LIMIT = 20;

/** Injected in tests: the sort's shuffle source. */
export type Options = { random: () => number };

/** The line under a row's title beside its thumbnail: the file's folder, or the link's host. */
function detailOf(row: Row) {
  if (row.kind === "file") return basename(dirname(row.source));
  try {
    return new URL(row.source).host;
  } catch {
    return "";
  }
}

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
  plugin.player.subscribe(); // was "events"
  await plugin.player.read(); // was "playback.read"
  const random = options.random ?? Math.random;
  const { text } = plugin.i18n;
  const queue = new Queue(plugin.i18n);
  queue.say = (notice: NoticeText, args?: NoticeArgs) => void plugin.osd.show(notice, args).catch(() => {});
  // i18n-ignore: a row's title is data
  queue.sayTitle = (title) => void plugin.osd.show(title.slice(0, 200)).catch(() => {});
  const prober = new Prober(queue, toolRunner(plugin.tools), () => plugin.update());
  // 0.1.11: thumbnails, made like the durations (only for the rows on screen), kept in
  // `<dataDirectory>/thumbs`; none without a data directory.
  let thumbDir: string | null = null;
  try {
    thumbDir = join(plugin.dataDirectory, "thumbs");
  } catch {
    /* no data directory: rows show no picture */
  }
  const thumbs = new Thumbnailer(
    queue,
    ffmpegRunner(plugin.tools),
    thumbDir,
    (path) => plugin.ui.image(path, { maxSide: THUMB_WIDTH }),
    () => plugin.update(),
  );
  queue.removed = (rowId) => {
    prober.cancel(rowId);
    thumbs.cancel(rowId);
  };
  plugin.onDispose(() => prober.cancelAll());
  plugin.onDispose(() => thumbs.cancelAll());
  // Durations are read for the rows on screen only (set at each update below).
  prober.want([]);
  let follow: ReturnType<typeof follower> | null = null;
  const player = playback(plugin, queue, () => void follow?.check());
  const link = linkDialog(plugin, queue);
  // SDK 1.5.2 sidebar tabs (`Panel.state`, `Panel.toggle`); absent from older SDKs, which have
  // one sidebar panel and use the position's state instead.
  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "Playlist" }) as Panel &
    Partial<{ readonly state: { preferred: boolean; active: boolean } | null; toggle(): Promise<void> }>;
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
  thumbs.import(store?.thumbs.value);
  player.setFast(settings.gapless);
  /**
   * What the saved list depends on besides the rows, cheap to compare: the settings (a few
   * fields), the sort and `queue.revision`, which every change of the rows, their lengths
   * or the selection moves. The list's snapshot (every row) is taken only when this changed.
   */
  const signature = () => {
    settings.repeat = queue.repeat;
    settings.shuffle = queue.shuffle;
    return `${queue.revision}\n${queue.sortKey}\n${queue.descending}\n${JSON.stringify(settings)}`;
  };
  let saved = signature();
  /** Whether anything to save changed since the last `persist`. */
  const dirty = () => signature() !== saved || prober.dirty || thumbs.dirty;
  /** Hand changes to the stores; they write within a second (C17). */
  const persist = () => {
    if (!store) return;
    const now = signature();
    if (now !== saved) store.lists.save(Store.snapshot(queue, settings));
    saved = now;
    if (prober.dirty) store.cache.save(prober.export());
    if (thumbs.dirty) store.thumbs.save(thumbs.export());
  };
  /** Write now (turning "Remember List" off removes the saved list at once). */
  const flush = async () => {
    persist();
    try {
      await Promise.all([store?.lists.flush(), store?.cache.flush(), store?.thumbs.flush()]);
    } catch (error) {
      queue.message = `Cannot save the list: ${errorText(error)}`;
      // After an await, outside the SDK's handlers: show it now (SDK 1.5.45 precise invalidation).
      plugin.update();
    }
  };
  let persistTimer: ReturnType<typeof setTimeout> | null = null;
  /** Called on every update pass: a timer only when something changed (it gathers a burst of changes). */
  const persistSoon = () => {
    if (!store || persistTimer || !dirty()) return;
    persistTimer = setTimeout(() => {
      persistTimer = null;
      persist();
    }, PERSIST_CHECK_MS);
  };
  // Registered after the stores: disposal runs it first, then the stores write what it handed them.
  plugin.onDispose(() => {
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = null;
    persist();
  });
  prober.scan();

  let adding = false,
    scanning = false,
    checking = false,
    statting = false,
    listBusy = false;
  let lastList: string | null = null;
  let focusSearch = 0;
  const focus = { search: false };

  /** Every action runs through here; failures (the player's or the SDK's messages) become a notice. */
  const guard =
    (run: () => unknown) =>
    async (): Promise<void> => {
      try {
        await run();
      } catch (error) {
        queue.message = errorText(error);
        // i18n-ignore: the player's (or the SDK's) own message
        void plugin.osd.show(errorText(error).slice(0, 200)).catch(() => {});
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
      queue.notice("The media being opened was removed from the list");
    }
  };

  /**
   * A system confirmation (`dialogs.confirm`). Another dialog still open
   * (`busy`) is a notice and counts as cancelled; other failures go to `guard`.
   */
  const ask = async (options: Question) => {
    try {
      return await plugin.dialogs.confirm(options);
    } catch (error) {
      if ((error as PluginError).code !== "busy") throw error;
      queue.notice("Close the other dialog first");
      return { confirmed: false, checked: false };
    }
  };
  /** Remove All: asks first unless "Don't ask again" was ticked on a confirmed clear (saved with the settings). */
  const clearAll = guard(async () => {
    if (!queue.rows.length) return;
    if (settings.confirmClear) {
      const answer = await ask({
        message: "Remove all {} items from the playlist?",
        detail: "The media that is playing continues to play.",
        confirm: "Remove All",
        destructive: true,
        checkbox: "Don't ask again",
        args: [queue.rows.length],
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
      const paths = await plugin.dialogs.openFiles({ title: "Add to Playlist" });
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
        const folder = await plugin.dialogs.openDirectory({ title: play ? "Open Folder" : "Add Folder to Playlist" });
        if (!folder) return;
        // A timer is outside the SDK's handlers: ask for the update (SDK 1.5.45 precise invalidation).
        const slow = setTimeout(() => {
          queue.notice("Scanning folder…");
          plugin.update();
        }, 1000);
        let scan;
        try {
          scan = await scanFolder(folder, { signal: stop.signal });
        } catch (error) {
          if (stop.signal.aborted) return;
          return queue.notice("Cannot read the folder: {}", [failureText(plugin.i18n, folderError(error))]);
        } finally {
          clearTimeout(slow);
        }
        if (!scan.files.length) return queue.notice("No playable files in the folder");
        // IA-PL-01: Open Folder replaces the list too (not while another source opens; "Append" adds).
        if (play && !settings.appendOnOpen && queue.rows.length && !player.busy()) removing(() => queue.clear(false));
        const wasEmpty = queue.rows.length === 0;
        const result = await queue.addEntries(
          scan.files.map((file) => ({ kind: "file" as const, source: file.path, key: file.key })),
        );
        prober.scan();
        const summary = queue.summary(result, { unreadable: scan.unreadable, truncated: scan.truncated });
        if (!play) {
          queue.selectFirstAdded(result, wasEmpty);
          return queue.notice(summary);
        }
        // The first new row, or (all duplicates) the row of the folder's first file.
        const target = result.fresh[0] ?? result.rows.find((row) => !!row);
        if (!target) return queue.notice(summary);
        if (player.busy())
          return queue.notice(plugin.i18n.list([summary, text("another source is opening, only added to the list")], "semicolons"));
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
    queue.notice("Removed: {}", [count]);
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
      removing(() => queue.removeRows(new Set(present)));
      if (present.length) queue.notice("Invalid items removed: {}", [present.length]);
      else queue.notice("No invalid items");
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
    // One clause each: the limit, then every file not found.
    const notices = paths.length > REVEAL_LIMIT ? [text("Only the first {} files are shown", [REVEAL_LIMIT])] : [];
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
        notices.push(text("File not found: {}", [fitName(basename(files[0]!), 100)]));
      }
    }
    if (notices.length) queue.notice(plugin.i18n.list(notices, "semicolons"));
  });

  /** C18: replace the list with a list file's entries (after confirmation when not empty). */
  const openList = guard(async () => {
    if (listBusy) return;
    listBusy = true;
    try {
      const path = await plugin.dialogs.openFile({ title: "Open Playlist File" });
      if (!path) return;
      const ext = extensionOf(path);
      if (!LIST_FILES.has(ext)) return queue.notice("Unsupported list format");
      let parsed;
      try {
        if ((await stat(path)).size > LIST_SIZE_LIMIT) return queue.notice("Cannot open the list: the file is larger than 16 MB");
        parsed = parseList(decodeList(await readFile(path), ext), ext, dirname(path));
      } catch (error) {
        return queue.notice("Cannot open the list: {}", [fitName(errorText(error))]);
      }
      if (
        queue.rows.length &&
        !(await ask({ message: "Replace the current list with {}?", confirm: "Replace", args: [fitName(basename(path))] }))
          .confirmed
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
          missing: true,
        })),
      );
      if (result.fresh[0]) queue.selected = result.fresh[0].rowId;
      prober.scan();
      lastList = listName(path);
      const skipped = parsed.skipped + (parsed.entries.length - entries.length) + result.duplicate + result.refused;
      const name = fitName(lastList, 100);
      if (skipped)
        queue.notice("Opened list: {name} ({count} items), skipped: {skipped}", { name, count: result.added, skipped });
      else queue.notice("Opened list: {name} ({count} items)", { name, count: result.added });
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
        title: "Save Playlist",
        // TRANSLATORS: the default file name; translate only the part before ".m3u8".
        fileName: lastList === null ? text("Playlist.m3u8") : `${lastList}.m3u8`,
      });
      if (!chosen) return;
      const { path, ext } = savePath(chosen);
      try {
        const rows = queue.rows.map((row) => ({ kind: row.kind, source: row.source, title: row.title, duration: lengthOf(row) }));
        writeAtomic(path, formatList(rows, path, ext));
      } catch (error) {
        return queue.notice("Cannot save the list: {}", [fitName(errorText(error))]);
      }
      lastList = listName(path);
      queue.notice("Saved list: {}", [fitName(basename(path))]);
    } finally {
      listBusy = false;
    }
  });

  // Palette categories (player 0.7.6): the palette reads "Category: Title". Each is the
  // label of the submenu or popup the command sits in (shared texts, same translations).
  const categories = {
    playlist: text("Playlist"),
    repeat: text("Repeat"),
    openMode: text("When Opening a File"),
    sort: text("Sort"),
  };
  /** Where a command's control lives: the playlist panel, or its settings page. */
  const PANEL = { home: "playlist.visibility" };
  const SETTINGS = { home: "playlist.settings" };
  /**
   * The commands that take rows out of the list (`destructive`, SDK 1.7.9): a run through
   * another plugin's `commands.run` (Ask Vestra) is confirmed by the player first. A menu pick,
   * a key or the palette runs them as before. Open List File… is not marked: it asks
   * "Replace…?" itself, and a second question would ask the user twice.
   */
  const DESTRUCTIVE = { destructive: true };
  const reason = (enabled: () => boolean) => (): Reason | null =>
    enabled() ? null : player.busy() ? "Opening, or the result is not known yet" : "Nothing to act on";
  /** A list command: not in the main menu; its keys work while the list has focus. */
  const listCommand = (
    id: string,
    title: ListTitle,
    enabled: () => boolean,
    run: () => unknown,
    bindings?: string[],
    extra: CommandSetup = {},
  ) =>
    plugin.commands.add({
      id: `playlist.${id}`,
      title,
      category: categories.playlist,
      ...PANEL,
      enabled,
      reason: extra.reason ?? reason(enabled),
      ...(bindings ? { bindings, focus: "panel" as const } : {}),
      run,
      ...extra,
    });

  // Main menu (allocation table: Open ▸ 110); the former Playback ▸ Playlist ▸ items (230–249) are palette only.
  // Playlist (F6) is palette only too (user ruling 2026-11-08): the menu's Sidebar (the player's) opens
  // the sidebar at its last tab.
  plugin.commands.add({
    id: "playlist.visibility",
    title: "Playlist",
    category: categories.playlist,
    bindings: ["F6"],
    // With tabs: checked while the sidebar is shown on the playlist's tab; F6 brings the tab
    // forward from another tab, hides the sidebar on it, shows the sidebar on it when hidden.
    checked: () => (list.state ? list.state.preferred && list.state.active : (sidebar.state?.preferred ?? true)),
    reason: () => (sidebar.state?.reason === "space" ? "Not enough room; enlarge the window to show it" : null),
    run: () => (typeof list.toggle === "function" ? list.toggle() : sidebar.toggle()),
  });
  plugin.commands.add({
    id: "playlist.open-folder",
    path: ["@open", "Open Folder…"],
    category: categories.playlist,
    group: "open",
    order: 110,
    bindings: ["F2"],
    busy: () => scanning,
    reason: () => (scanning ? "Scanning folder" : null),
    run: openFolder,
  });
  const playlistMenu = (id: string, title: PlaylistItem, order: number, options: CommandSetup) =>
    plugin.commands.add({
      id: `playlist.${id}`,
      title,
      order,
      category: categories.playlist,
      run: () => {},
      ...options,
      reason: options.reason ?? null,
    });
  const addingFiles = { enabled: () => !adding, busy: () => adding };
  const scanningFolder = { busy: () => scanning, reason: (): Reason | null => (scanning ? "Scanning folder" : null) };
  const listFile = { busy: () => listBusy, group: "list", ...PANEL };
  playlistMenu("add", "Add Files…", 230, { ...addingFiles, ...PANEL, run: addFiles });
  playlistMenu("link", "Add Link…", 231, { ...PANEL, run: guard(() => link.show()) });
  playlistMenu("add-folder", "Add Folder…", 232, { ...scanningFolder, ...PANEL, run: addFolderOnly });
  // The options shared by the menu and the settings page (IA-PL-02).
  const setRepeat = (mode: Repeat) => {
    queue.repeat = mode;
    queue.notice(
      mode === "none"
        ? "Repeat: Off"
        : mode === "all"
          ? "Repeat: All"
          : mode === "one"
            ? "Repeat: One"
            : "Repeat: Stop After Current",
    );
  };
  /** The bar's Repeat button: Off → All → One → Stop After Current → Off. */
  const cycleRepeat = () => {
    const index = repeats.findIndex(([mode]) => mode === queue.repeat);
    setRepeat(repeats[(index + 1) % repeats.length]![0]);
  };
  const setShuffle = (on: boolean) => {
    queue.setShuffle(on);
    queue.notice(queue.shuffle ? "Shuffle: On" : "Shuffle: Off");
  };
  const setFast = (on: boolean) => {
    settings.gapless = on;
    player.setFast(settings.gapless);
    queue.notice(settings.gapless ? "Fast advance to next: On" : "Fast advance to next: Off");
  };
  const setRemember = (on: boolean) => {
    settings.remember = on;
    void flush();
    queue.notice(settings.remember ? "The list will be remembered" : "The list will no longer be remembered");
  };
  const setConfirmClear = (on: boolean) => {
    settings.confirmClear = on;
    queue.notice(settings.confirmClear ? "Confirm before removing all: On" : "Confirm before removing all: Off");
  };
  // 233–235, then Stop After Current at 245 (236 and on were taken before it came).
  repeats.forEach(([mode, title], index) =>
    playlistMenu(`repeat-${mode}`, title, index < 3 ? 233 + index : 245, {
      category: categories.repeat,
      ...SETTINGS,
      radio: true,
      checked: () => queue.repeat === mode,
      run: () => setRepeat(mode),
    }),
  );
  playlistMenu("shuffle", "Shuffle", 236, {
    ...SETTINGS,
    checked: () => queue.shuffle,
    run: () => setShuffle(!queue.shuffle),
  });
  playlistMenu("open-list", "Open List File…", 237, { ...listFile, run: openList });
  playlistMenu("save-list", "Save List…", 238, {
    ...listFile,
    enabled: hasRows,
    reason: reason(hasRows),
    run: saveList,
  });
  openModes.forEach(([mode, title], index) =>
    playlistMenu(`open-mode-${mode}`, title, 239 + index, {
      group: "open-mode",
      category: categories.openMode,
      ...SETTINGS,
      radio: true,
      checked: () => settings.openMode === mode,
      run: () => {
        settings.openMode = mode;
      },
    }),
  );
  playlistMenu("fast-advance", "Fast Advance to Next", 242, {
    group: "options",
    ...SETTINGS,
    checked: () => settings.gapless,
    run: () => setFast(!settings.gapless),
  });
  playlistMenu("remember", "Remember List", 243, {
    group: "options",
    ...SETTINGS,
    checked: () => settings.remember,
    run: () => setRemember(!settings.remember),
  });
  // Turns the Remove All question back on after "Don't ask again" (or off).
  playlistMenu("confirm-clear", "Confirm Before Removing All", 244, {
    group: "options",
    ...SETTINGS,
    checked: () => settings.confirmClear,
    run: () => setConfirmClear(!settings.confirmClear),
  });

  // List commands (context menu, bottom-bar popups and keys while the list has focus).
  // Coordinator ruling 2026-11-08: while the list has focus it uses the navigation keys
  // (↑ ↓ PgUp PgDn Home End, handled in sidebar.slint) and Esc (gives the keys back to the
  // video); no other list shortcut shadows a player key. The list commands whose keys were the
  // player's (P, Ctrl+I, Ctrl+O, Ctrl+U, Backspace, Ctrl+T, Ctrl+Home, F2, F3, F5, F6) have none
  // and stay in the list menu and the palette. Enter (not a player key) is caught by the list
  // itself (play, then the keys go back to the video); its binding here names the key in the
  // list menu.
  const play = listCommand("play", "Play", canPlay, playSelected, ["Enter"], {
    busy: () => player.busy(),
  });
  const add = listCommand("list-add", "Add Files…", () => !adding, addFiles, undefined, { busy: () => adding });
  const addFolderCommand = listCommand("list-add-folder", "Add Folder…", () => true, addFolderOnly, undefined, scanningFolder);
  const addLink = listCommand("list-link", "Add Link…", () => true, guard(() => link.show()));
  const remove = listCommand("remove", "Remove Selected", () => queue.selection.size > 0, removeSelected, ["Delete"], DESTRUCTIVE);
  const removeOthers = listCommand(
    "remove-unselected",
    "Remove Unselected",
    () => queue.canRemoveUnselected(),
    removeUnselected,
    ["Ctrl+Delete"],
    DESTRUCTIVE,
  );
  const removeInvalidCommand = listCommand("remove-invalid", "Remove Invalid Items", hasRows, removeInvalid, undefined, {
    ...DESTRUCTIVE,
    busy: () => checking,
  });
  const removeAll = listCommand("clear", "Remove All", hasRows, clearAll, ["Alt+Delete"], DESTRUCTIVE);
  const hasLocal = () => queue.marked().some((row) => row.kind === "file");
  const revealCommand = listCommand("reveal", "Show in Folder", hasLocal, revealSelected, ["Ctrl+F"], {
    reason: () => (hasLocal() ? null : queue.selection.size ? "Links have no local location" : "Nothing to act on"),
  });
  const moveReason = (where: Placement) => (): Reason | null =>
    queue.canPlace(where) ? null : queue.filtering ? "The order cannot change while searching" : "Nothing to act on";
  const move = (id: string, title: ListTitle, where: Placement, binding: string | null) =>
    listCommand(id, title, () => queue.canPlace(where), place(where), binding ? [binding] : undefined, {
      reason: moveReason(where),
    });
  const moves: Record<Placement, Command> = {
    top: move("move-top", "Move to Top", "top", null),
    up: move("move-up", "Move Up", "up", "Ctrl+ArrowUp"),
    down: move("move-down", "Move Down", "down", "Ctrl+ArrowDown"),
    bottom: move("move-bottom", "Move to Bottom", "bottom", "Ctrl+End"),
  };
  const sortable = () => queue.rows.length > 1;
  const sortState = { busy: () => statting, category: categories.sort };
  const sorts = [
    listCommand("sort-ascending", "Ascending", () => true, setDirection(false), ["Ctrl+1"], {
      ...sortState,
      radio: true,
      checked: () => !queue.descending,
    }),
    listCommand("sort-descending", "Descending", () => true, setDirection(true), ["Ctrl+2"], {
      ...sortState,
      radio: true,
      checked: () => queue.descending,
    }),
    ...sortKeys.map(([key, title, binding]) =>
      listCommand(`sort-${key}`, title, sortable, sortBy(key), binding ? [binding] : undefined, sortState),
    ),
  ];
  const selectAll = listCommand("select-all", "Select All", () => queue.visible().length > 0, () => queue.selectAll(), [
    "Ctrl+A",
  ]);
  const search = listCommand("search", "Search", () => true, () => void focusSearch++, ["Ctrl+S"]);
  const openListCommand = listCommand("list-open-list", "Open List File…", () => true, openList, undefined, {
    busy: () => listBusy,
  });
  const saveListCommand = listCommand("list-save-list", "Save List…", hasRows, saveList, undefined, {
    busy: () => listBusy,
  });
  // "Close List" in the list menu (F6 is the player-wide Playlist toggle, which hides it too).
  const close = listCommand("close", "Close List", () => true, () => sidebar.setVisible(false));
  // IA-PL-02: Playlist Settings… in the list menu (F5 stays the player's Options… everywhere).
  listCommand("settings", "Playlist Settings…", () => true, guard(() => plugin.settings.show(SETTINGS_PAGE)));

  // IA-PL-02: Options ▸ Playback ▸ Playlist, in step with the menu items.
  const page = plugin.settings.page("ui/settings.slint", {
    id: SETTINGS_PAGE,
    category: "playback",
    title: "Playlist",
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
  const listArea = { scroll: 0, height: Number.POSITIVE_INFINITY, top: LIST_TOP };
  /** The rows' height in px (`ui/sidebar.slint`): taller beside a thumbnail. */
  const rowHeight = () => (settings.thumbnails ? THUMB_ROW_HEIGHT : ROW_HEIGHT);
  const scroller = { request: 0, to: 0, sentAt: Number.NEGATIVE_INFINITY, userAt: Number.NEGATIVE_INFINITY };
  let lastPlaying: string | null = null;
  // The panel is created anew each time it shows (the player builds it when it appears), so
  // it opens on the playing row: centred while hidden (`show-y`),
  // then once more when it shows (`appeared.pending`), and again while its height settles.
  const appeared = { at: Number.NEGATIVE_INFINITY, pending: false, reported: false };
  const appear = () => {
    Object.assign(appeared, { at: Date.now(), pending: true, reported: false });
    plugin.update();
  };
  const settling = () => Date.now() - appeared.at < SHOW_SETTLE_MS;
  const hidden = () => list.state?.effective === false;
  /**
   * The scroll this update asks for (`force`: ask even when the plugin believes the list is
   * there, since a new panel may not have reported yet): the cursor after a key; the playing
   * row centred while the list appears; else a newly playing row out of view.
   */
  const scrollTarget = (shown: Row[]): { to: number; force: boolean } | null => {
    const playing = queue.binding?.rowId ?? null;
    const playingChanged = playing !== lastPlaying;
    lastPlaying = playing;
    const reveal = revealCursor;
    revealCursor = false;
    if (!Number.isFinite(listArea.height)) return null;
    const at = (rowId: string | null) => shown.findIndex((row) => row.rowId === rowId);
    if (reveal) {
      const index = at(queue.selected);
      if (index >= 0)
        return { to: ensureVisible(index, listArea.scroll, listArea.height, shown.length, rowHeight()), force: false };
    }
    if (hidden() || appeared.pending) {
      const index = playing ? at(playing) : -1;
      const to = index < 0 ? null : centered(index, listArea.height, shown.length, rowHeight());
      // Nothing is on screen while hidden: only the next panel's start moves (no request).
      if (hidden()) {
        if (to !== null) listArea.scroll = to;
        return null;
      }
      appeared.pending = false;
      return to === null ? null : { to, force: true };
    }
    // Not while the user browses (scrolled lately), nor for a row the search hides.
    if (!playingChanged || !playing || Date.now() - scroller.userAt < USER_SCROLL_HOLD_MS) return null;
    const index = at(playing);
    if (index < 0 || inView(index, listArea.scroll, listArea.height, rowHeight())) return null;
    return { to: centered(index, listArea.height, shown.length, rowHeight()), force: false };
  };
  let wasShown = !hidden();
  // Shown again (F6, its tab, the fullscreen pop-out), hidden (centre it for the next
  // panel), or moved to or from its own window (a new panel too).
  let detachedNow = list.state?.detached ?? false;
  list.onChange((state) => {
    const shownNow = !!state?.effective;
    const moved = !!state && state.detached !== detachedNow;
    if (shownNow && (!wasShown || moved)) appear();
    else if (shownNow !== wasShown) plugin.update();
    wasShown = shownNow;
    if (state) detachedNow = state.detached;
  });

  /** Per row: the thumbnail placeholder's kind and the line under the title (a row's source never changes). */
  const looks = new WeakMap<Row, { media: string; detail: string }>();
  const lookOf = (row: Row) => {
    let look = looks.get(row);
    if (!look) {
      look = { media: row.kind === "url" ? "link" : (mediaKind(row.source) ?? ""), detail: detailOf(row) };
      looks.set(row, look);
    }
    return look;
  };

  /** One row as the list shows it (`ui/sidebar.slint` `PlaylistRow`). */
  type ViewRow = {
    id: string;
    number: number;
    label: string;
    duration: string;
    tooltip: string;
    problem: string;
    selected: boolean;
    current: boolean;
    played: boolean;
    failed: boolean;
    media: string;
    detail: string;
    thumb: ImageRef | null;
  };
  /** Every field of `ViewRow`: the type check fails when one is missing (or unknown). */
  const viewKeys = Object.keys({
    id: true,
    number: true,
    label: true,
    duration: true,
    tooltip: true,
    problem: true,
    selected: true,
    current: true,
    played: true,
    failed: true,
    media: true,
    detail: true,
    thumb: true,
  } satisfies Record<keyof ViewRow, true>) as (keyof ViewRow)[];
  /**
   * The object sent last for each row: an unchanged row is the same object on the next pass
   * (0.2.1), and it is frozen (0.2.3), which lets SDK 1.5.47 reuse its encoding instead of
   * encoding it again (at 20,000 rows most of a pass's cost). A changed row is a new frozen
   * object. Its only nested value, the picture, is an image reference (a leaf to the SDK).
   */
  const sentRows = new WeakMap<Row, Readonly<ViewRow>>();
  const viewRow = (row: Row, next: ViewRow): Readonly<ViewRow> => {
    const last = sentRows.get(row);
    if (last && viewKeys.every((key) => last[key] === next[key])) return last;
    const frozen = Object.freeze(next);
    sentRows.set(row, frozen);
    return frozen;
  };

  // 0.2.0 (large lists P3): `rows` is every shown row, drawn by a `ListView` (only the rows in
  // view have elements). Each row carries its `id`: on player 0.6.0 the SDK sends what changed
  // as row operations matched by it (a move is one operation, a changed row one `set`), and
  // the player applies them to the live list.
  let onScreen = "";
  list.bind(() => {
    persistSoon();
    player.prepare();
    const shown = queue.visible();
    const target = scrollTarget(shown);
    if (target !== null && (target.force || target.to !== listArea.scroll)) {
      scroller.request++;
      scroller.to = target.to;
      scroller.sentAt = Date.now();
      listArea.scroll = target.to;
    }
    const height = rowHeight();
    const pictures = settings.thumbnails;
    // Durations (and thumbnails) are read for the rows on screen only (never for the whole
    // list up front); with thumbnails, also for `THUMB_MARGIN` rows above and below.
    const top = Math.floor(Math.max(0, listArea.scroll) / height);
    const sized = Number.isFinite(listArea.height);
    const rowsOnScreen = sized ? Math.ceil(listArea.height / height) + 1 : 50;
    const margin = pictures && sized ? THUMB_MARGIN : 0;
    const ids = shown.slice(top, top + rowsOnScreen).map((row) => row.rowId);
    const near = margin
      ? [
          ...ids,
          ...shown.slice(top + rowsOnScreen, top + rowsOnScreen + margin).map((row) => row.rowId),
          ...shown.slice(Math.max(0, top - margin), top).map((row) => row.rowId),
        ]
      : ids;
    if (near.join("\n") !== onScreen) {
      onScreen = near.join("\n");
      prober.want(near);
    }
    // Thumbnails only while the list is on screen and has its size.
    thumbs.want(pictures && sized && !hidden() ? near : []);
    // Pictures go to the rows near the view only (the view's image budget); the others
    // hold `null` (no picture).
    const sendFrom = top - THUMB_SEND_MARGIN,
      sendTo = top + rowsOnScreen + THUMB_SEND_MARGIN;
    // While a search filters, a row's number is its place in the whole list (sent with it);
    // otherwise the list numbers rows by their index.
    const index = queue.filtering ? queue.positions() : null;
    let sum = 0,
      known = 0,
      partial = false;
    for (const row of shown) {
      const seconds = lengthOf(row);
      if (seconds !== null) {
        sum += seconds;
        known++;
      } else if (row.probe !== "pending") partial = true;
    }
    // "+": some row's length is not known (not read: off screen, ffprobe failed, or a URL that has not played).
    const total = known ? clock(sum) + (partial ? "+" : "") : "";
    return {
      rows: shown.map((row: Row, at) => {
        const look = lookOf(row);
        return viewRow(row, {
          id: row.rowId,
          number: index ? index.get(row.rowId)! + 1 : 0,
          label: row.title,
          duration: lengthOf(row) !== null ? clock(lengthOf(row)!) : row.probe === "failed" ? "--:--" : "",
          tooltip: row.error?.kind === "message" ? `${row.source}\n${row.error.text}` : row.source,
          problem: row.error && row.error.kind !== "message" ? row.error.kind : "",
          selected: queue.selection.has(row.rowId),
          current: row.rowId === queue.binding?.rowId,
          played: row.played,
          failed: row.status === "failed",
          // The thumbnail's placeholder ("video", "audio", "link" or ""), and the line under the
          // title: the file's folder, or the link's host. Sent in both row styles (0.2.1), so
          // the thumbnail toggle changes only the pictures near the view.
          media: look.media,
          detail: look.detail,
          // `null`: no picture (an empty image, player 0.6.0).
          thumb: (pictures && at >= sendFrom && at < sendTo && thumbs.picture(row.rowId)) || null,
        });
      }),
      thumbnails: pictures,
      total,
      filtering: queue.filtering,
      focusSearch,
      detached: sidebar.state?.detached ?? false,
      canAdd: !adding,
      canRemove: queue.selection.size > 0,
      canSort: sortable(),
      canDrag: !queue.filtering,
      // The bar's Repeat button: 0 Off, 1 All, 2 One, 3 Stop After Current.
      repeat: repeats.findIndex(([mode]) => mode === queue.repeat),
      scrollToY: scroller.to,
      scrollRequest: scroller.request,
      showY: listArea.scroll,
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
  /** Enter in the list (the list gives the keys back to the video right after). */
  list.on("playSelected", () => playSelected());
  list.on("selectStep", (delta: number, extend: boolean) => {
    queue.selectNeighbor(delta, extend);
    revealCursor = true;
  });
  list.on("dropRow", (rowId: string, index: number) => queue.drop(rowId, index));
  list.on("cycleRepeat", () => cycleRepeat());
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
        // Move: the bar has no move buttons (design B2); Panel.showMenu is one level, so the
        // four moves are a group of their own here.
        moves.top,
        moves.up,
        moves.down,
        moves.bottom,
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
  // 0.1.11: the bottom bar's thumbnail toggle (saved with the settings). The row at the
  // top of the view stays there while the rows change height: the list keeps it itself
  // (0.2.1, `changed row-height` in `ui/sidebar.slint`); a scroll request in the same
  // frame as the new heights could land on another row. The plugin expects the scroll the
  // list will report, so that report is not taken for the user's.
  list.on("toggleThumbnails", () => {
    const before = rowHeight();
    settings.thumbnails = !settings.thumbnails;
    const after = rowHeight();
    if (!settings.thumbnails) thumbs.want([]);
    if (!Number.isFinite(listArea.height)) return;
    const to = limit(Math.floor(listArea.scroll / before) * after, listArea.height, queue.visible().length, after);
    scroller.to = to;
    scroller.sentAt = Date.now();
    listArea.scroll = to;
  });
  list.on("toggleDetached", () => sidebar.setDetached(!(sidebar.state?.detached ?? false)));
  // Only typing in the search box holds the panel (the bar or fullscreen pop-out stays while
  // the user types); list focus alone does not (G-01 acceptance 4: it collapses when the pointer leaves).
  const interact = () => list.setInteraction({ focused: focus.search });
  // List focus alone does not lock the panel (see above).
  list.on("focusChanged", () => {});
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
  // `top`: where the list starts in the panel (below the panel's own tab strip, 0 under the
  // player's); checked, since a stale build of the panel may report only two values.
  list.on("listArea", (scroll: number, height: number, top: number) => {
    const next = Number.isFinite(scroll) ? scroll : 0;
    const area = Number.isFinite(height) && height > 0 ? height : Number.POSITIVE_INFINITY;
    const resized = Math.abs(area - listArea.height) >= 1;
    // A new panel's first report, and a resize while it settles, are layout, not the user.
    const settled = settling() && (!appeared.reported || resized);
    appeared.reported = true;
    // Within a row of the scroll asked for or expected: the list keeps the top row's partial
    // offset (the thumbnail toggle, 0.2.2), so its report can differ by less than a row.
    const ours = (Math.abs(next - scroller.to) < rowHeight() && Date.now() - scroller.sentAt < 500) || settled;
    if (Math.abs(next - listArea.scroll) >= 1 && !ours) scroller.userAt = Date.now();
    listArea.scroll = next;
    const known = Number.isFinite(listArea.height);
    listArea.height = area;
    // The first height after start-up: the list has just appeared (a remembered list).
    if (!known && Number.isFinite(area)) appear();
    else if (settled && resized) appeared.pending = true;
    if (typeof top === "number" && Number.isFinite(top) && top >= 0) listArea.top = top;
  });

  // A15: drops from the file manager. On the video area (the player groups by extension): G-11
  // replace the list ("Append" option: add to the end) and play the first item; Option only adds.
  // On the list (G-12): add to the end, or before the row under the drop point with
  // "Insert at the drop position"; never play. One drop at a time, in order.
  let dropping: Promise<void> = Promise.resolve();
  const dropped = (
    paths: string[],
    how: { play: boolean; clear?: boolean; expand?: boolean; at?: { x: number; y: number } },
  ) =>
    (dropping = dropping.then(
      guard(async () => {
        const expanded = await expandDrop(paths, { signal: stop.signal });
        if (stop.signal.aborted) return;
        // A15 / IA-PL-01: a single media file dropped on the video area brings its
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
        if (!expanded.entries.length) return queue.notice(nothingToAdd(plugin.i18n, expanded));
        let before: number | undefined;
        if (how.at) {
          const shown = queue.visible();
          const row = dropRow(how.at.y, listArea.scroll, listArea.height, shown.length, listArea.top, rowHeight());
          if (row !== null) before = queue.rows.indexOf(shown[row]!);
        }
        // G-11: replaced without asking; not while another source opens (only added then).
        if (how.clear && queue.rows.length && !player.busy()) removing(() => queue.clear(false));
        const wasEmpty = queue.rows.length === 0;
        const result = await queue.addEntries(expanded.entries, before === undefined ? {} : { before });
        prober.scan();
        const summary = queue.summary(result, dropProblems(expanded));
        if (!how.play) {
          queue.selectFirstAdded(result, wasEmpty);
          return queue.notice(summary);
        }
        // The dropped file when it brought its siblings; else the first new row, or (all
        // duplicates) the existing row of the first item.
        const at = start === null ? -1 : expanded.entries.findIndex((entry) => entry.source === start);
        const target = (at >= 0 ? result.rows[at] : undefined) ?? result.fresh[0] ?? result.rows.find((row) => !!row);
        if (!target) return queue.notice(summary);
        if (player.busy()) return queue.notice(plugin.i18n.list([summary, text("opening, added to the list")], "semicolons"));
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

  // Thumbnails never compete with an open: ffmpeg waits while a source loads and for
  // `HOLD_AFTER_OPEN_MS` after (cache hits go on).
  let opening = false;
  const holdThumbs = (phase: string) => {
    if (phase === "loading") {
      if (!opening) thumbs.hold(Number.POSITIVE_INFINITY);
      opening = true;
    } else if (opening) {
      opening = false;
      thumbs.hold(Date.now() + HOLD_AFTER_OPEN_MS);
    }
  };
  holdThumbs(plugin.player.state.phase);
  plugin.player.onChange((state) => holdThumbs(state.phase));
}
