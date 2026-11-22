import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { filesPending, settleFiles } from "./settle.ts";
import activate from "../main.ts";
import { Prober, type ProbeRunner } from "../probe.ts";
import { playback } from "../playback.ts";
import { Queue } from "../queue.ts";
import { chineseTexts } from "./texts.ts";

type ViewRow = {
  id: string;
  number: number;
  label: string;
  duration: string;
  tooltip: string;
  selected: boolean;
  current: boolean;
  played: boolean;
  failed: boolean;
};
type Intent = { kind: string; source?: string; startPaused?: boolean; expected?: Json };
const pause = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

/** A fake ffprobe behind `host.tools` (`path` is the probed file's name): each call waits until the test answers. */
function fakeProbe() {
  const calls: { args: string[]; path: string; answer: (seconds: number) => void; fail: () => void }[] = [];
  const handler = (args: string[]) =>
    new Promise<{ stdout: string }>((resolve, reject) => {
      calls.push({
        args,
        path: basename(args.at(-1)!),
        answer: (seconds) => resolve({ stdout: JSON.stringify({ format: { duration: String(seconds) } }) }),
        fail: () => reject(new HostError("failed", "ffprobe failed")),
      });
    });
  const call = (name: string) => {
    const found = calls.find((c) => c.path === name);
    assert.ok(found, `ffprobe was not run for ${name}`);
    return found;
  };
  /**
   * Wait until `count` processes were started, or until nothing more happens: no
   * new process and no file request (the probe stats the file first) for 200 ms.
   */
  const started = async (count: number) => {
    let seen = calls.length,
      quietSince = Date.now();
    for (const deadline = Date.now() + 15_000; calls.length < count && Date.now() < deadline; await pause(1)) {
      if (calls.length !== seen || filesPending()) {
        seen = calls.length;
        quietSince = Date.now();
      } else if (Date.now() - quietSince > 200) break;
    }
    await pause(5);
    return calls.map((c) => c.path).sort();
  };
  return { handler, calls, call, started };
}

/** Checks the texts as a Simplified Chinese user sees them (the plugin's old texts); `"en"` for the English source. */
/** `hidden`: the list is closed after the start, so the plugin hears the source events, not a stream. */
async function start(probe = fakeProbe(), locale: "en" | "zh-Hans" = "zh-Hans", hidden = !!process.env.PLAYLIST_TEST_HIDDEN) {
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-"));
  const media = join(directory, "media");
  await mkdir(media);
  const a = join(media, "a.mp4"),
    b = join(media, "b.mp4"),
    c = join(media, "c.mp4"),
    // Another path to a, through a directory link ("junction" needs no privilege on Windows).
    alias = join(directory, "linked", "a.mp4");
  for (const file of [a, b, c]) await writeFile(file, "plugin test, no media decode claim");
  await symlink(media, join(directory, "linked"), "junction");
  const host = new TestHost();
  settleFiles(host);
  host.tools.handle("ffprobe", probe.handler);
  const { stop } = await startPlugin((plugin) => activate(plugin), {
    directory: new URL("..", import.meta.url),
    host,
    locale,
  });
  const panel = host.mounted();
  // `PLAYLIST_TEST_HIDDEN=1` runs every case so.
  if (hidden) {
    await host.command("playlist.close").run();
    await host.settle();
  }
  const app = {
    host,
    probe,
    a,
    b,
    c,
    alias,
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    names: () => app.rows().map((row) => row.label),
    prop: (name: string) => panel.props[name],
    command: (name: string) => host.command(`playlist.${name}`),
    /** The last on-screen notice. */
    osd: () => host.osd.at(-1),
    /** Intents of every source submission, oldest first. */
    opens: () => host.sent("source.submit").map((call) => call.intent as unknown as Intent),
    emit: async (callback: string, ...args: Json[]) => {
      panel.emit(callback, ...args);
      await host.settle();
    },
    run: async (name: string) => {
      await app.command(name).run();
      await host.settle();
    },
    /** A key while the list has focus. */
    press: async (binding: string) => {
      await host.press(binding, { panel: true });
      await host.settle();
    },
    click: (index: number, modifiers: { ctrl?: boolean; shift?: boolean } = {}) =>
      app.emit("select-row", app.rows()[index].id, !!modifiers.ctrl, !!modifiers.shift),
    /**
     * Wait (bounded) for something the plugin does on its own; not settle(), which
     * never returns while a test holds a reply back.
     */
    until: async (check: () => boolean, what: string) => {
      for (const deadline = Date.now() + 15_000; !check(); await pause(5))
        if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    },
    /** Double-click a row while an open is held back (settling would wait for it). */
    playNow: async (index = app.rows().findIndex((row) => row.selected)) => {
      panel.emit("play-row", app.rows()[index].id);
      await pause();
    },
    /**
     * 添加文件…: the plugin resolves each path on disk itself (settle() waits for host
     * calls only), so wait for the notice that ends the add, then for the window.
     */
    addFiles: async (paths: string[]) => {
      const notices = host.osd.length;
      host.dialogAnswers.push(paths);
      await app.run("add");
      if (!paths.length) return;
      for (const deadline = Date.now() + 15_000; host.osd.length === notices; await host.settle(10))
        if (Date.now() > deadline) throw new Error(`adding ${paths.length} files did not finish`);
      await host.settle();
    },
    addUrl: async (url: string) => {
      await app.run("link");
      host.window("playlist-link").emit("add-link", url);
      await host.settle();
    },
    /**
     * The pre-IA-PL-01 behaviour for sources opened elsewhere: 「仅打开该文件」 with
     * 「打开或拖入时追加」, so they never touch or take over the list (UI-PL-20).
     */
    leaveOutsideSources: async () => {
      host.settingsPage("playlist").emit("set-open-mode", 0);
      host.settingsPage("playlist").emit("set-append-on-open", true);
      await host.settle();
    },
    /** The current source plays to its natural end. */
    end: () => host.setSource({ phase: "ended", terminalReason: "natural" }),
    /** The user replays the current source (same source, new playthrough). */
    restart: () => {
      const { stamp, cycle } = host.source;
      host.setSource({ stamp: { ...stamp, intentVersion: stamp.intentVersion + 1 }, cycle: cycle + 1, phase: "ready" });
    },
    async close() {
      await stop();
      await rm(directory, { recursive: true, force: true });
    },
  };
  return app;
}

test("English is the source: menus, notices and confirmations in English by default", async () => {
  const app = await start(fakeProbe(), "en");
  try {
    // Menu sweep (B5): the former Playback ▸ Playlist ▸ items are palette only.
    assert.deepEqual([app.command("fast-advance").display.path, app.command("fast-advance").display.title], [[], "Fast Advance to Next"]);
    assert.deepEqual(app.command("open-folder").display.path, ["Open", "Open Folder…"]);
    assert.equal(app.command("remove").display.title, "Remove Selected");
    await app.addFiles([app.a, app.alias, app.b]);
    assert.equal(app.osd(), "Added: 2, duplicates skipped: 1");
    app.host.dialogAnswers.push({ confirmed: false, checked: false });
    await app.press("Alt+Delete");
    await app.host.settle(30);
    assert.equal(app.host.confirmTexts.at(-1)?.message, "Remove all 2 items from the playlist?");
  } finally {
    await app.close();
  }
});

test("menu positions follow the round allocation table; list keys only work with list focus", async () => {
  const app = await start();
  try {
    // As a Simplified Chinese user sees the menus (the declarations are English, see below). Menu
    // sweep (B5): the former Playback ▸ Playlist ▸ items (230–244) are palette only, shown as "(title)";
    // Playlist (F6) too (user ruling 2026-11-08: the player's Sidebar opens the sidebar).
    const declared = [...app.host.commands.values()].map(({ declaration: d, display }) => [
      display.path.join("/") || `(${display.title})`,
      d.order,
      d.group,
      d.bindings.join(" "),
      d.focus ?? "",
    ]);
    assert.deepEqual(declared, [
      ["(播放列表)", 0, "domain", "F6", ""],
      ["打开/打开文件夹…", 110, "open", "F2", ""],
      ["(添加文件…)", 230, "domain", "", ""],
      ["(添加链接…)", 231, "domain", "", ""],
      ["(添加文件夹…)", 232, "domain", "", ""],
      ["(不循环)", 233, "domain", "", ""],
      ["(全部)", 234, "domain", "", ""],
      ["(单个)", 235, "domain", "", ""],
      ["(播完当前停止)", 245, "domain", "", ""],
      ["(随机)", 236, "domain", "", ""],
      ["(打开列表文件…)", 237, "list", "", ""],
      ["(保存列表…)", 238, "list", "", ""],
      ["(仅打开该文件)", 239, "open-mode", "", ""],
      ["(加入相似文件)", 240, "open-mode", "", ""],
      ["(加入同文件夹全部文件)", 241, "open-mode", "", ""],
      ["(快速推进到下一个)", 242, "options", "", ""],
      ["(记住列表)", 243, "options", "", ""],
      ["(移除全部前确认)", 244, "options", "", ""],
      ["(播放)", 0, "domain", "Enter", "panel"],
      ["(添加文件…)", 0, "domain", "", ""],
      ["(添加文件夹…)", 0, "domain", "", ""],
      ["(添加链接…)", 0, "domain", "", ""],
      ["(移除所选)", 0, "domain", "Delete", "panel"],
      ["(移除未选)", 0, "domain", "Ctrl+Delete", "panel"],
      ["(移除无效项)", 0, "domain", "", ""],
      ["(移除全部)", 0, "domain", "Alt+Delete", "panel"],
      ["(在文件夹中显示)", 0, "domain", "Ctrl+F", "panel"],
      // 0.2.10: row menu only (`focus: "panel"` keeps it out of the palette).
      ["(在新窗口播放)", 0, "domain", "", "panel"],
      ["(置顶)", 0, "domain", "", ""],
      ["(上移)", 0, "domain", "Ctrl+ArrowUp", "panel"],
      ["(下移)", 0, "domain", "Ctrl+ArrowDown", "panel"],
      ["(置底)", 0, "domain", "Ctrl+End", "panel"],
      ["(正序)", 0, "domain", "Ctrl+1", "panel"],
      ["(反序)", 0, "domain", "Ctrl+2", "panel"],
      ["(按标题)", 0, "domain", "Ctrl+3", "panel"],
      ["(按文件名)", 0, "domain", "Ctrl+4", "panel"],
      ["(按扩展名)", 0, "domain", "Ctrl+5", "panel"],
      ["(按大小)", 0, "domain", "Ctrl+6", "panel"],
      ["(按时长)", 0, "domain", "Ctrl+7", "panel"],
      ["(按日期)", 0, "domain", "Ctrl+8", "panel"],
      ["(乱序)", 0, "domain", "Ctrl+9", "panel"],
      ["(按文件夹)", 0, "domain", "Ctrl+0", "panel"],
      ["(按路径排序)", 0, "domain", "", ""],
      ["(倒置当前顺序)", 0, "domain", "", ""],
      ["(全选)", 0, "domain", "Ctrl+A", "panel"],
      ["(搜索)", 0, "domain", "Ctrl+S", "panel"],
      ["(打开列表文件…)", 0, "domain", "", ""],
      ["(保存列表…)", 0, "domain", "", ""],
      ["(关闭列表)", 0, "domain", "", ""],
      ["(播放列表设置…)", 0, "domain", "", ""],
    ]);
    // Radio groups: repeat (off by default), open mode ("加入相似文件" by default, IA-PL-01), sort direction (ascending).
    for (const [id, on] of [
      ["open-mode-only", false],
      ["open-mode-similar", true],
      ["open-mode-folder", false],
      ["sort-ascending", true],
      ["sort-descending", false],
    ] as const) {
      assert.equal(app.command(id).declaration.radio, true, id);
      assert.equal(app.command(id).state.checked, on, id);
    }
    assert.equal(app.command("remember").state.checked, true, "记住列表 is on by default");
    assert.deepEqual(app.host.conflicts, []);
    // Repeat is a radio group, off by default; shuffle is a check item, off by default.
    assert.equal(app.command("repeat-none").declaration.radio, true);
    assert.equal(app.command("repeat-none").state.checked, true);
    assert.equal(app.command("repeat-all").state.checked, false);
    assert.equal(app.command("shuffle").state.checked, false);
    await assert.rejects(app.host.press("Delete"), /nothing is bound/, "Delete is a list key");
    assert.equal(app.host.mounted().slot, "sidebar");
    assert.equal(app.host.mounted().component, "PlaylistSidebar");
    assert.equal(app.host.sent("ui.mount")[0]!.title, "Playlist");
    assert.deepEqual(app.command("play").state, {
      enabled: false,
      checked: null,
      busy: false,
      reason: "Nothing to act on",
    });
    assert.equal(app.host.translate("Nothing to act on"), "当前没有可操作项");
  } finally {
    await app.close();
  }
});

test("every command has a palette category (its submenu or popup label) and a home where it has one", async () => {
  const app = await start();
  try {
    const commands = [...app.host.commands.entries()];
    assert.equal(commands.length, 50);
    const ids = new Set(commands.map(([id]) => id));
    const byCategory = new Map<string, string[]>();
    const homes = new Map<string, string[]>();
    for (const [id, { declaration, display }] of commands) {
      const category = display.category;
      assert.ok(typeof category === "string" && category.trim().length > 0, `${id} has a category`);
      assert.ok(declaration.category, `${id} declares a category`);
      byCategory.set(category, [...(byCategory.get(category) ?? []), id.replace("playlist.", "")]);
      if (declaration.home) {
        assert.ok(ids.has(declaration.home), `${id}: home ${declaration.home} is a command of this plugin`);
        homes.set(declaration.home, [...(homes.get(declaration.home) ?? []), id.replace("playlist.", "")]);
      }
    }
    // As a Simplified Chinese user sees them: the submenu and popup labels' own translations.
    assert.deepEqual(
      [...byCategory].map(([category, list]) => [category, list.length]),
      [
        ["播放列表", 31],
        ["循环", 4],
        ["打开文件时", 3],
        ["排序", 12],
      ],
    );
    assert.deepEqual(byCategory.get("排序"), [
      "sort-ascending",
      "sort-descending",
      "sort-title",
      "sort-filename",
      "sort-extension",
      "sort-size",
      "sort-duration",
      "sort-date",
      "sort-shuffle",
      "sort-folder",
      "sort-path",
      "sort-reverse",
    ]);
    // The options also on the settings page point there; the list's commands point at the list.
    assert.deepEqual(homes.get("playlist.settings"), [
      "repeat-none",
      "repeat-all",
      "repeat-one",
      "repeat-once",
      "shuffle",
      "open-mode-only",
      "open-mode-similar",
      "open-mode-folder",
      "fast-advance",
      "remember",
      "confirm-clear",
    ]);
    assert.equal(homes.get("playlist.visibility")?.length, 37);
    assert.equal(app.command("visibility").declaration.home, undefined);
    assert.equal(app.command("open-folder").declaration.home, undefined);
    assert.equal(app.command("sort-size").declaration.home, "playlist.visibility");
  } finally {
    await app.close();
  }
});

test("the commands that take rows out of the list declare destructive (SDK 1.7.9); no other command does", async () => {
  const app = await start();
  try {
    const destructive = [...app.host.commands.entries()]
      .filter(([, { declaration }]) => declaration.destructive === true)
      .map(([id]) => id.replace("playlist.", ""));
    assert.deepEqual(destructive, ["remove", "remove-unselected", "remove-invalid", "clear"]);
    // Every other command declares none (absent is false), so a run of it through `commands.run` is not asked.
    for (const [id, { declaration }] of app.host.commands) {
      if (!destructive.includes(id.replace("playlist.", ""))) assert.equal(declaration.destructive ?? false, false, id);
    }
  } finally {
    await app.close();
  }
});

test("F6, Close List and the menu check share the sidebar preference; detach toggles", async () => {
  const app = await start();
  try {
    assert.equal(app.command("visibility").state.checked, true);
    await app.run("close");
    assert.equal(app.host.layout.sidebar?.preferred, false);
    assert.equal(app.command("visibility").state.checked, false);
    await app.host.press("F6");
    await app.host.settle();
    assert.equal(app.host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
    await app.press("F6"); // with list focus F6 is the player-wide toggle too
    assert.equal(app.host.layout.sidebar?.preferred, false);
    await app.run("visibility");
    // Another plugin (the bar's list button) toggles the same slot.
    app.host.layout.sidebar!.preferred = false;
    app.host.emit("window.state", {});
    await app.host.settle();
    assert.equal(app.command("visibility").state.checked, false);
    app.host.layout.sidebar!.preferred = true;

    assert.equal(app.prop("detached"), false);
    await app.emit("toggle-detached");
    assert.equal(app.host.layout.sidebar?.detached, true);
    assert.equal(app.prop("detached"), true);
    await app.emit("toggle-detached");
    assert.equal(app.host.layout.sidebar?.detached, false);
    // List focus alone does not lock the panel (G-01: the fullscreen pop-out still collapses); a drag does.
    await app.emit("focus-changed", true);
    assert.equal((app.host.mounted().locks as { focused?: boolean } | null)?.focused ?? false, false);
    await app.emit("dragging", true);
    assert.deepEqual(app.host.mounted().locks, { focused: false, dragging: true, popupOpen: false });
  } finally {
    await app.close();
  }
});

/** SDK 1.5.2 TestHost sidebar tabs; older SDKs have one sidebar panel. */
type TabHost = TestHost & {
  mountOther(slot: "sidebar", plugin: string, options?: { title?: string }): void;
  selectTab(plugin: string): void;
  unmountOther(plugin: string): void;
};
const tabbedHost = (host: TestHost): TabHost | null =>
  typeof (host as Partial<TabHost>).mountOther === "function" ? (host as TabHost) : null;
const noTabs = typeof (TestHost.prototype as Partial<TabHost>).mountOther !== "function" && "SDK without sidebar tabs";

test("sidebar tabs: F6 brings the playlist's tab forward, hides the sidebar on it, shows it on it", { skip: noTabs }, async () => {
  const app = await start();
  try {
    const host = tabbedHost(app.host)!;
    host.mountOther("sidebar", "example.list", { title: "示例列表" });
    await host.settle();
    assert.equal((host.layout.sidebar as { panels?: unknown[] } | null)?.panels?.length, 2);
    // The plugin never writes `tabbed`: the player sets it while it draws its strip.
    assert.equal(app.prop("tabbed"), undefined);
    assert.equal(app.command("visibility").state.checked, true);
    // Another tab current: unchecked; F6 brings the playlist forward, the sidebar stays shown.
    host.selectTab("example.list");
    await host.settle();
    assert.equal(app.command("visibility").state.checked, false);
    await host.press("F6");
    await host.settle();
    assert.equal(host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
    // The playlist current: F6 hides the whole sidebar.
    await host.press("F6");
    await host.settle();
    assert.equal(host.layout.sidebar?.preferred, false);
    assert.equal(app.command("visibility").state.checked, false);
    // Hidden with another tab current: F6 shows the sidebar on the playlist's tab.
    host.selectTab("example.list");
    await host.settle();
    await host.press("F6");
    await host.settle();
    assert.equal(host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
    // Focus in the list: F6 (the toggle) and the list menu's Close List hide the whole sidebar.
    await app.press("F6");
    assert.equal(host.layout.sidebar?.preferred, false);
    await app.run("visibility");
    assert.equal(host.layout.sidebar?.preferred, true);
    await app.run("close");
    assert.equal(host.layout.sidebar?.preferred, false);
    // The other tab gone: back to the one-panel behaviour.
    host.unmountOther("example.list");
    await host.settle();
    await host.press("F6");
    await host.settle();
    assert.equal(host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
  } finally {
    await app.close();
  }
});

test("adding files dedups by realpath and never plays; Enter plays with startPaused false", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.alias, app.b]);
    assert.equal(app.rows().length, 2);
    assert.equal(app.osd(), "已添加 2 项，跳过 1 个重复");
    assert.equal(app.opens().length, 0);
    assert.equal(app.rows()[0].selected, true);
    app.host.dialogAnswers.push(null); // cancelled: nothing changes
    await app.run("add");
    assert.equal(app.host.osd.length, 1);
    await app.press("Enter");
    assert.deepEqual(app.opens()[0], { kind: "open", source: app.a, startPaused: false });
    assert.equal(app.osd(), "a.mp4", "switching shows the title on screen");
    assert.equal(app.command("play").state.busy, true, "open stays unsettled while loading");
    assert.deepEqual(app.host.sequence, { canNext: false, canPrevious: false });
    app.host.setSource({ phase: "ready", duration: 65 });
    await app.host.settle();
    assert.equal(app.rows()[0].current, true);
    assert.equal(app.rows()[0].duration, "01:05");
    assert.equal(app.rows()[1].duration, "");
    assert.equal(app.prop("total"), "01:05");
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: false });
    // Enter plays too; a double-click plays the clicked row.
    await app.click(1);
    await app.press("Enter");
    assert.equal(app.opens().at(-1)?.source, app.b);
    app.host.setSource({ phase: "ready", duration: 3600 });
    await app.host.settle();
    assert.deepEqual(
      app.rows().map(({ label, current, played }) => ({ label, current, played })),
      [
        { label: "a.mp4", current: false, played: true },
        { label: "b.mp4", current: true, played: true },
      ],
    );
    assert.equal(app.prop("total"), "1:01:05");
    assert.equal(app.rows()[0].tooltip, app.a);
  } finally {
    await app.close();
  }
});

test("the playlist provides next/previous to other plugins", async () => {
  const app = await start();
  try {
    assert.deepEqual(app.host.sequence, { canNext: false, canPrevious: false });
    await app.addFiles([app.a, app.b]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: false });
    assert.equal(app.host.source.sequence?.next, true, "the host merges the flags into the source state");
    await app.host.step("next");
    await app.host.settle();
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: app.b, startPaused: false });
    await assert.rejects(app.host.step("next"), { code: "unavailable" });
  } finally {
    await app.close();
  }
});

test("multi-select: Ctrl/Shift-click and Ctrl+A; Delete and DEL remove the selection", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.addUrl("https://example.org/d");
    await app.click(0);
    await app.click(2, { shift: true });
    assert.deepEqual(app.rows().map((row) => row.selected), [true, true, true, false]);
    await app.click(1, { ctrl: true });
    assert.deepEqual(app.rows().map((row) => row.selected), [true, false, true, false]);
    await app.press("Delete");
    assert.deepEqual(app.names(), ["b.mp4", "https://example.org/d"]);
    assert.deepEqual(app.rows().map((row) => row.selected), [true, false], "the cursor lands on the first remaining row");
    await app.emit("select-step", 1, true); // Shift+↓
    assert.deepEqual(app.rows().map((row) => row.selected), [true, true]);
    await app.click(0);
    await app.press("Ctrl+A");
    assert.deepEqual(app.rows().map((row) => row.selected), [true, true]);
    assert.equal(app.prop("can-remove"), true);
    await app.emit("remove"); // the DEL button
    assert.equal(app.rows().length, 0);
    assert.equal(app.prop("can-remove"), false);
    assert.equal(app.command("select-all").state.enabled, false);
  } finally {
    await app.close();
  }
});

test("Ctrl+arrows, the bottom-bar icons and dragging reorder the selection", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.click(2);
    assert.equal(app.command("move-up").state.enabled, true);
    assert.equal(app.command("move-down").state.enabled, false);
    await app.press("Ctrl+ArrowUp");
    assert.deepEqual(app.names(), ["a.mp4", "c.mp4", "b.mp4"]);
    await app.run("move-top");
    assert.deepEqual(app.names(), ["c.mp4", "a.mp4", "b.mp4"]);
    assert.equal(app.command("move-top").state.enabled, false);
    await app.press("Ctrl+End");
    assert.deepEqual(app.names(), ["a.mp4", "b.mp4", "c.mp4"]);
    await app.click(0);
    await app.run("move-down");
    assert.deepEqual(app.names(), ["b.mp4", "a.mp4", "c.mp4"]);
    await app.press("Ctrl+ArrowDown");
    assert.deepEqual(app.names(), ["b.mp4", "c.mp4", "a.mp4"]);
    // Drag b and c (selected together) below a.
    await app.click(0);
    await app.click(1, { shift: true });
    await app.emit("drop-row", app.rows()[0].id, 3);
    assert.deepEqual(app.names(), ["a.mp4", "b.mp4", "c.mp4"]);
    assert.deepEqual(app.rows().map((row) => row.selected), [false, true, true]);
    // Dragging an unselected row moves only that row.
    await app.emit("drop-row", app.rows()[0].id, 2);
    assert.deepEqual(app.names(), ["b.mp4", "a.mp4", "c.mp4"]);
    assert.deepEqual(app.rows().map((row) => row.selected), [false, true, false]);
  } finally {
    await app.close();
  }
});

test("the list menu, ADD and SORT popups list this plugin's commands; sorting reorders", async () => {
  const app = await start();
  try {
    await app.emit("context-menu", 40, 60);
    // 18 commands and 7 separators, single level (Panel.showMenu: at most 32); Play in New Window
    // (0.2.10) not on macOS yet.
    assert.deepEqual(app.host.menus.at(-1), {
      items: [
        "playlist.play",
        ...(process.platform === "darwin" ? [] : ["playlist.play-new-window"]),
        null,
        "playlist.list-add",
        "playlist.list-add-folder",
        "playlist.list-link",
        null,
        "playlist.remove",
        "playlist.remove-unselected",
        "playlist.remove-invalid",
        "playlist.clear",
        null,
        "playlist.move-top",
        "playlist.move-up",
        "playlist.move-down",
        "playlist.move-bottom",
        null,
        "playlist.reveal",
        null,
        "playlist.select-all",
        "playlist.search",
        null,
        "playlist.list-open-list",
        "playlist.list-save-list",
        null,
        "playlist.close",
      ],
      x: 40,
      y: 60,
    });
    await app.emit("add-menu", 10, 300);
    assert.deepEqual(app.host.menus.at(-1), {
      items: ["playlist.list-add", "playlist.list-add-folder", "playlist.list-link"],
      x: 10,
      y: 300,
    });
    await app.emit("sort-menu", 90, 300);
    assert.deepEqual(app.host.menus.at(-1)?.items, [
      "playlist.sort-ascending",
      "playlist.sort-descending",
      "playlist.sort-title",
      "playlist.sort-filename",
      "playlist.sort-extension",
      "playlist.sort-size",
      "playlist.sort-duration",
      "playlist.sort-date",
      "playlist.sort-shuffle",
      "playlist.sort-folder",
      "playlist.sort-path",
      "playlist.sort-reverse",
    ]);
    await app.addFiles([app.c, app.a, app.b]);
    assert.equal(app.prop("can-sort"), true);
    await app.run("sort-title");
    assert.deepEqual(app.names(), ["a.mp4", "b.mp4", "c.mp4"]);
    await app.run("sort-reverse");
    assert.deepEqual(app.names(), ["c.mp4", "b.mp4", "a.mp4"]);
    app.host.dialogAnswers.push({ confirmed: true, checked: false });
    await app.run("clear");
    assert.equal(app.rows().length, 0);
  } finally {
    await app.close();
  }
});

/** The link window's message line as `ui/link.slint` words it, translated as the player shows it. */
function linkMessage(host: TestHost) {
  const { notice, line, count, limit } = host.window("playlist-link").props as {
    notice: string;
    line: number;
    count: number;
    limit: number;
  };
  const [text, args]: [string, (string | number)[]] =
    notice === "added"
      ? count > 1
        ? ["{} links added, only selected; playing them needs an available source provider", [count]]
        : ["Link added, only selected; playing it needs an available source provider", []]
      : notice === "incomplete"
        ? line > 0
          ? ["Line {}: Enter a complete HTTP(S) link", [line]]
          : ["Enter a complete HTTP(S) link", []]
        : notice === "refused"
          ? line > 0
            ? ["Line {}: Only HTTP(S) links without a user name or password are accepted", [line]]
            : ["Only HTTP(S) links without a user name or password are accepted", []]
          : notice === "full"
            ? line > 0
              ? ["Line {}: The list is full at {} items", [line, limit]]
              : ["The list is full at {} items", [limit]]
            : ["Enter HTTP(S) links; adding only selects, it does not play", []];
  return host.translate(text, args);
}

test("添加链接…: one link per line, a bad line stops there and says which", async () => {
  const app = await start();
  try {
    const window = () => app.host.window("playlist-link");
    await app.addUrl(" https://example.org/a \n\nhttps://example.org/b\r\n");
    assert.deepEqual(app.names(), ["https://example.org/a", "https://example.org/b"]);
    assert.equal(window().props.notice, "added");
    assert.equal(linkMessage(app.host), "已添加 2 个链接，仅选中；显式播放需可用的来源提供者");
    assert.equal(window().visible, false);
    assert.equal(app.rows()[1].selected, true);
    await app.addUrl("https://example.org/c\nftp://example.org/x\nhttps://example.org/d");
    assert.deepEqual([window().props.notice, window().props.line], ["refused", 2]);
    assert.equal(linkMessage(app.host), "第 2 行：只接受不含账户密码的 HTTP(S) 链接");
    assert.equal(window().visible, true);
    assert.equal(app.names().length, 3);
    await app.addUrl("   ");
    assert.equal(linkMessage(app.host), "请输入完整 HTTP(S) 链接");
    assert.equal(app.opens().length, 0);
  } finally {
    await app.close();
  }
});

test("URL rows: adding only selects; the original URL goes to public routing; one open at a time", async () => {
  const app = await start();
  try {
    const a = "https://example.org/watch?v=one",
      b = "https://example.org/watch?v=two";
    await app.addUrl(a);
    assert.equal(linkMessage(app.host), "链接已添加，仅选中；显式播放需可用的来源提供者");
    assert.equal(app.host.window("playlist-link").visible, false);
    assert.equal(app.osd(), "已添加链接");
    await app.addUrl(b);
    await app.addUrl("ftp://example.org/x");
    assert.equal(linkMessage(app.host), "只接受不含账户密码的 HTTP(S) 链接");
    assert.equal(app.rows().length, 2);
    assert.equal(app.rows()[1].selected, true);
    assert.equal(app.opens().length, 0);
    await app.click(0);
    assert.equal(app.rows()[0].tooltip, a);

    // A resolver takes a while: play is refused meanwhile.
    let accept!: () => void;
    app.host.handle("source.submit", () =>
      new Promise<void>((resolve) => (accept = resolve)).then(() => {
        app.host.openElsewhere(a, "loading"); // stands in for the accepted URL open
        const { stamp, cycle } = app.host.source;
        return { status: "completed", ticket: { stamp, cycle, owner: 1 } };
      }),
    );
    await app.playNow(0);
    await app.until(() => accept !== undefined, "the open reached the host");
    await app.playNow(0);
    await app.playNow(1);
    assert.equal(app.host.sent("source.submit").length, 1);
    assert.deepEqual(app.opens()[0], { kind: "open", source: a, startPaused: false });
    assert.equal(app.command("play").state.enabled, false);
    accept();
    await app.host.settle();
    app.host.reset("source.submit");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[0].current, true);
  } finally {
    await app.close();
  }
});

test("clock ticks do not resend unchanged list models or command states", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready", duration: 1000 });
    await app.host.settle();
    const sends = () => app.host.sent("ui.setMount").length + app.host.sent("commands.update").length;
    const before = sends();
    for (let position = 0; position < 1000; position++) app.host.setSource({ position });
    await app.host.settle();
    assert.equal(sends(), before);
  } finally {
    await app.close();
  }
});

test("natural end advances once in current order, conditional on the ended state, keeping the selection", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/c");
    await app.addFiles([app.a, app.b]);
    // Rows are c, a, b; moving c down gives a, c, b.
    await app.click(0);
    await app.press("Ctrl+ArrowDown");
    assert.deepEqual(app.names(), ["a.mp4", "https://example.org/c", "b.mp4"]);
    const c = app.rows()[1].id;
    await app.click(0);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.click(2); // the user is looking at b
    const ended = { ...app.host.source.stamp };
    app.end();
    app.end(); // a repeated end event of the same playthrough
    await app.host.settle();
    assert.equal(app.opens().length, 2);
    assert.deepEqual(app.opens()[1], { kind: "open", source: "https://example.org/c", startPaused: false, expected: ended });
    assert.equal(app.osd(), "https://example.org/c");
    assert.equal(app.rows()[0].current, false);
    assert.equal(app.rows()[0].played, true, "the finished row is drawn as played");
    assert.equal(app.rows()[1].current, true);
    assert.equal(app.rows().find((row) => row.id === c)?.selected, false);
    assert.equal(app.rows()[2].selected, true, "auto-next keeps selection");
  } finally {
    await app.close();
  }
});

test("a replay renews end eligibility; the last row announces that the list is finished", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.end();
    await app.host.settle();
    assert.equal(app.osd(), "列表已播完");
    app.restart();
    await app.host.settle();
    assert.equal(app.rows()[0].current, true, "a replay keeps the row bound");
    app.end();
    await app.host.settle();
    assert.deepEqual(app.host.osd.slice(-2), ["列表已播完", "列表已播完"]);
    assert.equal(app.opens().length, 1);
  } finally {
    await app.close();
  }
});

test("repeat all wraps to the first row; repeat one replays the same media conditionally", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.run("repeat-all");
    assert.equal(app.command("repeat-all").state.checked, true);
    assert.equal(app.command("repeat-none").state.checked, false);
    assert.equal(app.osd(), "循环：全部");
    await app.click(1);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: true });
    app.end();
    await app.host.settle();
    assert.equal(app.opens().at(-1)?.source, app.a, "wrapped to the first row");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();

    await app.run("repeat-one");
    const ended = { ...app.host.source.stamp };
    app.end();
    await app.host.settle();
    assert.deepEqual(app.opens().at(-1), { kind: "restart", expected: ended });
    assert.equal(app.rows()[0].current, true, "the replayed row stays bound");
    app.end();
    await app.host.settle();
    assert.equal(app.opens().at(-1)?.kind, "restart", "and replays again");
    assert.equal(app.opens().filter((intent) => intent.kind === "open").length, 2);
  } finally {
    await app.close();
  }
});

test("shuffle plays every row once, then the list is finished", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.run("shuffle");
    assert.equal(app.command("shuffle").state.checked, true);
    assert.equal(app.osd(), "随机播放：开");
    await app.press("Enter");
    for (let i = 0; i < 3; i++) {
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      app.end();
      await app.host.settle();
    }
    const opened = app.opens().map((intent) => intent.source);
    assert.equal(opened.length, 3);
    assert.deepEqual([...opened].sort(), [app.a, app.b, app.c].sort());
    assert.equal(app.osd(), "列表已播完");
  } finally {
    await app.close();
  }
});

test("the bar's Repeat button cycles Off → All → One → Stop After Current → Off, in step with the commands", async () => {
  const app = await start();
  try {
    assert.equal(app.prop("repeat"), 0);
    const steps = [
      [1, "repeat-all", "循环：全部"],
      [2, "repeat-one", "循环：单个"],
      [3, "repeat-once", "循环：播完当前停止"],
      [0, "repeat-none", "循环：不循环"],
    ] as const;
    for (const [index, id, notice] of steps) {
      await app.emit("cycle-repeat");
      assert.equal(app.prop("repeat"), index, id);
      assert.equal(app.command(id).state.checked, true, id);
      assert.equal(app.osd(), notice);
    }
    // The palette's radio item and the settings page set the same mode the button shows.
    assert.equal(app.command("repeat-once").declaration.radio, true);
    await app.run("repeat-once");
    assert.equal(app.prop("repeat"), 3);
    assert.equal(app.host.settingsPage("playlist").props.repeat, 3);
    app.host.settingsPage("playlist").emit("set-repeat", 1);
    await app.host.settle();
    assert.equal(app.prop("repeat"), 1);
    assert.equal(app.command("repeat-all").state.checked, true);
  } finally {
    await app.close();
  }
});

for (const shuffle of [false, true]) {
  test(`Stop After Current stops at the end of the current item without advancing${shuffle ? " (shuffle on)" : ""}; Next and Previous still work`, async () => {
    const app = await start();
    try {
      await app.addFiles([app.a, app.b, app.c]);
      if (shuffle) await app.run("shuffle");
      await app.run("repeat-once");
      await app.click(1);
      await app.press("Enter");
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      const notices = app.host.osd.length;
      app.end();
      await app.host.settle();
      assert.equal(app.opens().length, 1, "nothing else opens");
      assert.equal(app.rows()[1].current, true, "the current row stays");
      assert.deepEqual(app.names(), ["a.mp4", "b.mp4", "c.mp4"]);
      assert.equal(app.host.osd.length, notices, "no notice: playback simply stops");
      // A repeated end event of the same playthrough does not advance either.
      app.end();
      await app.host.settle();
      assert.equal(app.opens().length, 1);
      // Manual Next and Previous work in every mode.
      await app.host.step("next");
      await app.host.settle();
      assert.equal(app.opens().length, 2);
      const next = app.opens().at(-1)!.source;
      assert.notEqual(next, app.b);
      if (!shuffle) assert.equal(next, app.c);
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      // Shuffle may have picked the first row, which has no previous one.
      await app.host.step(shuffle && !app.host.sequence?.canPrevious ? "next" : "previous");
      await app.host.settle();
      assert.equal(app.opens().length, 3);
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      app.end();
      await app.host.settle();
      assert.equal(app.opens().length, 3, "and stops again at the end");
    } finally {
      await app.close();
    }
  });
}

test("Stop After Current on the last item stops without the end-of-list notice", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.run("repeat-once");
    await app.click(2);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    const notices = app.host.osd.length;
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 1);
    assert.equal(app.host.osd.length, notices, "no notice");
    assert.ok(!app.host.osd.includes("列表已播完"));
    assert.equal(app.rows()[2].current, true);
  } finally {
    await app.close();
  }
});

for (const [mode, from, to] of [
  ["repeat-none", 0, 1],
  ["repeat-all", 2, 0],
] as const) {
  test(`switching Stop After Current to ${mode} while an item plays advances at its end`, async () => {
    const app = await start();
    try {
      await app.addFiles([app.a, app.b, app.c]);
      await app.run("repeat-once");
      await app.click(from);
      await app.press("Enter");
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      await app.run(mode);
      app.end();
      await app.host.settle();
      assert.equal(app.opens().length, 2);
      assert.equal(app.opens().at(-1)?.source, [app.a, app.b, app.c][to]);
    } finally {
      await app.close();
    }
  });
}

test("Stop After Current: removing the playing item opens nothing at its end", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.run("repeat-once");
    await app.click(1);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.click(1);
    await app.press("Delete");
    assert.deepEqual(app.names(), ["a.mp4", "c.mp4"]);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 1, "nothing else opens");
    assert.deepEqual(app.names(), ["a.mp4", "c.mp4"]);
  } finally {
    await app.close();
  }
});

test("B26: with Fast Advance to Next on, nothing is prepared in Stop After Current", async () => {
  const host = new TestHost();
  settleFiles(host);
  const { plugin, stop } = await startPlugin(() => {}, { directory: new URL("..", import.meta.url), host, locale: "en" });
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-plan-"));
  try {
    const [a, b] = [join(directory, "a.mp4"), join(directory, "b.mp4")];
    for (const file of [a, b]) await writeFile(file, "x");
    const queue = new Queue(plugin.i18n);
    const player = playback(plugin, queue);
    await queue.addFiles([a, b]);
    player.setFast(true);
    await player.play(queue.rows[0]!.rowId);
    host.setSource({ phase: "ready" });
    await host.settle();
    const prepared = async () => {
      player.prepare();
      for (const deadline = Date.now() + 15_000; player.planned === null; await pause(5))
        if (Date.now() > deadline) throw new Error("the next row was not prepared");
      return player.planned;
    };
    assert.equal(await prepared(), queue.rows[1]!.rowId, "Off: the next row is prepared");
    queue.repeat = "once";
    player.prepare();
    assert.equal(player.planned, null, "Stop After Current: no plan");
    queue.repeat = "all";
    assert.equal(await prepared(), queue.rows[1]!.rowId, "All again: prepared");
  } finally {
    await stop();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the bottom bar has no move buttons and has the Repeat button; the list menu's Move items run each move", async () => {
  const slint = await readFile(new URL("../ui/sidebar.slint", import.meta.url), "utf8");
  const code = slint.replace(/\/\/.*$/gm, "");
  const bar = code.slice(code.indexOf("bar := Rectangle {"), code.indexOf("search-keys := FocusScope {"));
  assert.ok(bar.length > 0);
  assert.doesNotMatch(bar, /move-(top|up|down|bottom)|PlaylistBridge\.place\(/, "no move buttons");
  assert.doesNotMatch(code, /can-(top|up|down|bottom)|callback place\(/);
  assert.match(bar, /clicked => \{ PlaylistBridge\.cycle-repeat\(\); \}/);
  // The button keeps the bar's colour in every mode: the four modes differ by their glyph only.
  const repeat = bar.slice(bar.lastIndexOf("BarButton {", bar.indexOf("cycle-repeat")));
  assert.doesNotMatch(repeat, /Theme\.accent|color:|background:/);
  const literal = (name: string) => {
    const found = code.match(new RegExp(`out property <string> ${name}: "([^"]+)";`));
    assert.ok(found, name);
    return found![1];
  };
  const glyphs = ["repeat-off", "repeat-all", "repeat-one", "stop-after"].map((name) => {
    assert.match(repeat, new RegExp(`PlaylistGlyphs\\.${name}\\b`), name);
    if (name !== "repeat-off") return literal(name);
    // Off: the loop with a slash (user decision 2026-11-09).
    assert.match(code, /out property <string> repeat-off: PlaylistGlyphs\.repeat-off-slash;/);
    assert.doesNotMatch(code, /repeat-off-list/);
    return literal("repeat-off-slash");
  });
  assert.equal(new Set(glyphs).size, 4, "four distinct glyphs");
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    await app.emit("context-menu", 5, 5);
    const items = app.host.menus.at(-1)!.items;
    const at = items.indexOf("playlist.move-top");
    assert.deepEqual(items.slice(at - 1, at + 5), [
      null,
      "playlist.move-top",
      "playlist.move-up",
      "playlist.move-down",
      "playlist.move-bottom",
      null,
    ]);
    assert.equal(app.host.translate("Move to Top"), "置顶");
    await app.click(2);
    for (const [id, order] of [
      ["move-top", ["c.mp4", "a.mp4", "b.mp4"]],
      ["move-down", ["a.mp4", "c.mp4", "b.mp4"]],
      ["move-bottom", ["a.mp4", "b.mp4", "c.mp4"]],
      ["move-up", ["a.mp4", "c.mp4", "b.mp4"]],
    ] as const) {
      await app.host.command(`playlist.${id}`).run();
      await app.host.settle();
      assert.deepEqual(app.names(), order, id);
    }
  } finally {
    await app.close();
  }
});

test("a replay that wins the race against auto-advance keeps the newer source", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.end();
    app.restart(); // while the next file's path is being checked
    await app.host.settle();
    assert.equal(app.opens().length, 2, "the conditional open was submitted");
    assert.equal(app.host.source.source, app.a);
    assert.equal(app.rows().some((row) => row.current), false);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 2, "the list no longer follows the replayed source");
    // Explicit play still works afterwards.
    assert.equal(app.command("play").state.enabled, true);
  } finally {
    await app.close();
  }
});

for (const hidden of [false, true])
  test(`errors, stops and other sources never advance; another open of the same path is not taken over or announced${hidden ? " (list hidden: source events)" : ""}`, async () => {
    const app = await start(undefined, undefined, hidden);
    try {
      await app.leaveOutsideSources();
      await app.addFiles([app.a, app.b]);
      await app.press("Enter");
      app.host.setSource({ phase: "error", terminalReason: { failed: "broken" } });
      await app.host.settle();
      assert.equal(app.rows()[0].failed, true);
      assert.equal(app.rows()[0].tooltip, `${app.a}\nbroken`);
      assert.equal(app.osd(), "无法打开 a.mp4：broken");
      assert.equal(app.opens().length, 1);

      await app.press("Enter");
      app.host.setSource({ phase: "ended", terminalReason: "stopped" });
      await app.host.settle();
      assert.equal(app.opens().length, 2);

      await app.press("Enter");
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      const notices = app.host.osd.length;
      app.host.openElsewhere(app.a);
      await app.host.settle();
      assert.equal(app.host.osd.length, notices, "UI-PL-20: no notice for an outside source");
      assert.equal(app.rows()[0].current, false);
      app.end();
      await app.host.settle();
      assert.equal(app.opens().length, 3);
    } finally {
      await app.close();
    }
  });

test("a failed open stays on its row, keeps the navigation anchor and can be retried", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/a");
    await app.addUrl("https://example.org/b");
    await app.addUrl("https://example.org/c");
    const [a, b, c] = app.rows().map((row) => row.id);
    await app.click(1);
    // Like the player: an accepted open whose resolution failed leaves the source in `error`
    // (`backend` with an empty source is a resolution cancelled by a close, §11).
    app.host.handle("source.submit", (p) => {
      app.host.openElsewhere(String((p.intent as { source: string }).source), "loading");
      app.host.setSource({ phase: "error", terminalReason: { failed: "resolverUnavailable" } });
      return { status: "failed", error: { backend: "resolverUnavailable" } };
    });
    await app.press("Enter");
    assert.equal(app.rows()[1].failed, true);
    assert.match(app.rows()[1].tooltip, /resolverUnavailable/);
    assert.equal(app.osd(), "无法打开 https://example.org/b：resolverUnavailable");
    await app.click(0);
    app.host.reset("source.submit");
    await app.host.step("next"); // from the failed row b, not from the selection a
    await app.host.settle();
    assert.equal(app.opens()[1].source, "https://example.org/c");
    assert.equal(app.rows().find((row) => row.id === c)?.failed, false);
    app.host.setSource({ phase: "ready" });
    await app.emit("select-row", b, false, false);
    await app.run("play"); // retry
    assert.equal(app.opens()[2].source, "https://example.org/b");
    assert.equal(app.rows().length, 3);
    assert.equal(app.rows()[1].id, b);
    assert.equal(app.rows()[1].failed, false);
    assert.ok(a);
  } finally {
    await app.close();
  }
});

test("a late accepted reply cannot reclaim a newer external source or resurrect a removed row", async () => {
  const app = await start();
  try {
    await app.leaveOutsideSources();
    await app.addFiles([app.a, app.b]);
    // The host accepts our open at once, but the reply arrives after a newer source.
    const deferOpen = () => {
      let accept: (() => void) | undefined;
      app.host.handle("source.submit", () => {
        app.host.openElsewhere(app.a, "loading");
        const { stamp, cycle } = app.host.source;
        return new Promise<void>((resolve) => (accept = resolve)).then(() => ({
          status: "completed",
          ticket: { stamp, cycle, owner: 1 },
        }));
      });
      return {
        /** The open reaches the host only after the plugin's own work: wait for it. */
        reached: () => app.until(() => accept !== undefined, "the open reached the host"),
        release: () => accept!(),
      };
    };
    let open = deferOpen();
    await app.playNow(0);
    await open.reached();
    app.host.openElsewhere(app.a); // same path, opened elsewhere
    open.release();
    await app.host.settle();
    assert.equal(app.rows().some((row) => row.current), false);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 1);

    open = deferOpen();
    await app.playNow(0);
    await open.reached();
    app.host.openElsewhere(app.b);
    app.host.mounted().emit("remove");
    await app.until(() => app.osd() === "正在打开的媒体已移出列表", "notice");
    open.release();
    await app.host.settle();
    app.host.reset("source.submit");
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows().some((row) => row.current), false);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 2);
  } finally {
    await app.close();
  }
});

test("a lost open reply is resolved by the same operation, never resubmitted", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/a");
    app.host.handle("source.submit", () => {
      app.host.openElsewhere("https://example.org/a", "loading"); // the host did accept it
      throw new Error("reply lost");
    });
    const { stamp, cycle } = { stamp: { generation: 1, intentVersion: 1 }, cycle: 1 };
    app.host.handle("source.operation", () => ({ status: "completed", ticket: { stamp, cycle, owner: 1 } }));
    await app.emit("play-row", app.rows()[0].id);
    await app.emit("play-row", app.rows()[0].id);
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[0].current, true);
    assert.equal(app.host.sent("source.submit").length, 1);
    assert.deepEqual(
      app.host.sent("source.operation").map((call) => call.operationId),
      [app.host.sent("source.submit")[0].operationId],
    );
  } finally {
    await app.close();
  }
});

test("removing the current row and clearing keep the public media playing", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.press("Delete");
    assert.equal(app.osd(), "当前媒体已移出列表");
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows()[0].selected, true);
    app.host.dialogAnswers.push({ confirmed: true, checked: false });
    await app.run("clear");
    assert.equal(app.osd(), "列表已清空，当前媒体继续");
    assert.equal(app.rows().length, 0);
    assert.equal(app.command("clear").state.enabled, false);
    assert.equal(app.host.source.phase, "ready");
    assert.equal(app.opens().filter((intent) => intent.kind !== "open").length, 0);
  } finally {
    await app.close();
  }
});

test("an accepted URL open replaced by another source while resolving is not a row failure", async () => {
  const app = await start();
  try {
    await app.leaveOutsideSources();
    await app.addUrl("https://example.org/a");
    await app.addUrl("https://example.org/b");
    const [, b] = app.rows().map((row) => row.id);
    // Accepted (pending while the resolver runs); then another source takes over.
    let outcome: Json = null;
    app.host.handle("source.submit", async (params) => {
      app.host.reset("source.submit");
      const done = (await app.host.request("source.submit", params)) as { ticket: Json };
      app.host.handle("source.operation", () => outcome ?? { status: "pending", ticket: done.ticket });
      return { status: "pending", ticket: done.ticket };
    });
    await app.playNow(0);
    await app.until(() => app.command("play").state.busy === true, "play busy");
    app.host.openElsewhere("/elsewhere.mp4");
    outcome = { status: "failed", error: { backend: "source resolution cancelled, expired or superseded" } };
    await app.host.settle(600);
    app.host.reset("source.operation");
    assert.equal(app.rows()[0].failed, false);
    assert.equal(app.command("play").state.enabled, true); // the open is released
    // No failed anchor on row a: "next" navigates from the selection, not from a.
    await app.emit("select-row", b, false, false);
    assert.deepEqual(app.host.sequence, { canNext: false, canPrevious: true });
  } finally {
    await app.close();
  }
});

test("ffprobe fills unplayed durations and the total; the player's duration overrides once a row plays", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    assert.deepEqual(await app.probe.started(2), ["a.mp4", "b.mp4"]);
    const [a, b] = [app.probe.call("a.mp4"), app.probe.call("b.mp4")];
    assert.deepEqual(a.args, ["-v", "error", "-show_entries", "format=duration", "-of", "json", await realpath(app.a)]);
    assert.deepEqual(app.rows().map((row) => row.duration), ["", ""], "pending rows show nothing yet");
    a.answer(61);
    b.answer(3600);
    await app.host.settle();
    assert.deepEqual(app.rows().map((row) => row.duration), ["01:01", "1:00:00"]);
    assert.equal(app.prop("total"), "1:01:01");
    assert.equal(app.host.osd.length, 1, "probing is silent");
    await app.press("Enter");
    app.host.setSource({ phase: "ready", duration: 65 });
    await app.host.settle();
    assert.equal(app.rows()[0].duration, "01:05");
    assert.equal(app.prop("total"), "1:01:05");
    // Cached by realpath + mtime: re-adding the same file after removal does not spawn again.
    await app.click(1);
    await app.press("Delete");
    await app.addFiles([app.b]);
    assert.deepEqual(await app.probe.started(3), ["a.mp4", "b.mp4"]);
    await app.host.settle();
    assert.equal(app.rows()[1].duration, "1:00:00");
  } finally {
    await app.close();
  }
});

test("URL rows are never probed and leave the total open", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.com/v.mp4");
    await app.addFiles([app.a]);
    assert.deepEqual(await app.probe.started(2), ["a.mp4"]);
    app.probe.call("a.mp4").answer(30);
    await app.host.settle();
    assert.deepEqual(app.rows().map((row) => row.duration), ["", "00:30"]);
    assert.equal(app.prop("total"), "00:30+");
  } finally {
    await app.close();
  }
});

test("removing a row or clearing the list aborts its probe", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    assert.deepEqual(await app.probe.started(3), ["a.mp4", "b.mp4"]);
    await app.press("Delete"); // a is selected
    assert.deepEqual(await app.probe.started(3), ["a.mp4", "b.mp4", "c.mp4"], "the freed slot goes to the next row");
    app.probe.call("a.mp4").answer(10); // a late answer for a removed row is dropped
    app.host.dialogAnswers.push({ confirmed: true, checked: false });
    await app.run("clear");
    for (const call of app.probe.calls) call.answer(5);
    await app.host.settle();
    assert.equal(app.rows().length, 0);
    assert.equal(app.prop("total"), "");
  } finally {
    await app.close();
  }
});

test("a missing or failing ffprobe shows --:-- and a partial total, without a notice", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.probe.started(2);
    const notices = app.host.osd.length;
    app.probe.call("a.mp4").fail();
    app.probe.call("b.mp4").answer(90);
    await app.host.settle();
    assert.deepEqual(app.rows().map((row) => row.duration), ["--:--", "01:30"]);
    assert.equal(app.prop("total"), "01:30+");
    assert.equal(app.host.osd.length, notices);
    assert.equal(app.rows()[0].failed, false, "an unknown duration is not a row error");
  } finally {
    await app.close();
  }
});

test("at most two ffprobe processes run at once", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b, app.c]);
    assert.deepEqual(await app.probe.started(3), ["a.mp4", "b.mp4"], "the third waits for a slot");
    app.probe.call("b.mp4").answer(10);
    assert.deepEqual(await app.probe.started(3), ["a.mp4", "b.mp4", "c.mp4"]);
    app.probe.call("a.mp4").answer(20);
    app.probe.call("c.mp4").answer(30);
    await app.host.settle();
    assert.equal(app.prop("total"), "01:00");
  } finally {
    await app.close();
  }
});

test("Prober: ffprobe runs with a 10 s timeout and removal aborts the run", async (t) => {
  const zh = await chineseTexts();
  t.after(() => zh.stop());
  const queue = new Queue(zh.texts);
  const runs: { args: string[]; signal: AbortSignal; timeoutMs: number }[] = [];
  const runner: ProbeRunner = (args, { signal, timeoutMs }) =>
    new Promise((_, reject) => {
      runs.push({ args, signal, timeoutMs });
      signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
    });
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-probe-"));
  try {
    const file = join(directory, "a.mp4");
    await writeFile(file, "x");
    await queue.addFiles([file]);
    const prober = new Prober(queue, runner, () => {});
    prober.scan();
    for (let i = 0; i < 200 && !runs.length; i++) await pause(1);
    assert.equal(runs[0]!.timeoutMs, 10_000);
    assert.deepEqual(runs[0]!.args.slice(0, -1), ["-v", "error", "-show_entries", "format=duration", "-of", "json"]);
    prober.cancel(queue.rows[0]!.rowId);
    assert.equal(runs[0]!.signal.aborted, true);
    assert.equal(prober.active, 0);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

// Focus rules (user ruling 2026-11-08): only a click on a row (or the search box's Esc/↓) gives
// the list the focus. While it has it, the list itself takes the standard list keys ↑ ↓ PgUp PgDn
// Home End (navigation), Enter (play) and Esc (keys back to the video), as list views do; its key
// bindings take no global key, and every other key goes on to the player.
test("list focus: the list takes ↑ ↓ PgUp PgDn Home End Enter Esc; its bindings take no global key", async () => {
  const app = await start();
  try {
    // List key bindings (`focus: "panel"`) contain no global key (coordinator ruling 2026-11-08):
    // neither the player's and other built-ins' default keys nor this plugin's own global ones.
    const commands = [...app.host.commands.values()].map(({ declaration: d }) => d);
    const panelKeys = commands.filter((d) => d.focus === "panel").flatMap((d) => d.bindings ?? []);
    const ownGlobal = commands.filter((d) => d.focus !== "panel").flatMap((d) => d.bindings ?? []);
    assert.deepEqual(ownGlobal.toSorted(), ["F2", "F6"], "Open Folder… and Playlist");
    assert.ok(panelKeys.includes("Enter") && panelKeys.includes("Delete") && panelKeys.includes("Ctrl+A"));
    // Global default keys of the player and the other built-ins (the ones a list key could clash with).
    const globalKeys = [
      ...["Space", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", "Escape"],
      ...["P", "Ctrl+I", "Ctrl+O", "F3", "Ctrl+U", "Backspace", "Ctrl+T", "Ctrl+Home", "F5"],
    ];
    for (const key of [...globalKeys, ...ownGlobal]) {
      assert.ok(!panelKeys.includes(key), `${key} is not a list binding`);
    }
    // Enter is a list binding but no global key (it only names the key in the list menu).
    assert.ok(!globalKeys.includes("Enter"));
    // The commands that lost their keys stay (list menu, palette).
    for (const id of ["list-add", "list-add-folder", "list-link", "remove-invalid", "move-top", "list-open-list", "list-save-list", "close", "settings"]) {
      assert.deepEqual(app.command(id).declaration.bindings ?? [], [], id);
    }
    // Enter from the list (`play-selected`) plays the selected row.
    await app.addFiles([app.a, app.b]);
    await app.click(1);
    await app.emit("play-selected");
    assert.equal(app.opens().at(-1)?.source, app.b);
  } finally {
    await app.close();
  }
  const slint = await readFile(new URL("../ui/sidebar.slint", import.meta.url), "utf8");
  const scope = slint.slice(slint.indexOf("list-focus := FocusScope {"));
  const handler = scope.slice(scope.indexOf("key-pressed(event) => {"), scope.indexOf("return reject;\n        }"));
  // The Slint list consumes exactly the navigation keys, Enter and Esc (unmodified; Shift extends
  // the selection) and rejects every other key, which goes on to the bindings and the player. Of
  // the global keys it covers only the navigation keys and Esc: standard list behaviour while the
  // list has focus, not a list shortcut.
  const consumed = [...handler.matchAll(/event\.text == Key\.(\w+)/g)].map((m) => m[1]);
  assert.deepEqual(consumed, ["Return", "Escape", "UpArrow", "DownArrow", "PageUp", "PageDown", "Home", "End"]);
  const navigation = ["UpArrow", "DownArrow", "PageUp", "PageDown", "Home", "End"];
  assert.deepEqual(consumed.filter((key) => key !== "Return" && key !== "Escape"), navigation);
  for (const key of navigation) {
    assert.match(handler, new RegExp(`Key\\.${key} \\{ PlaylistBridge\\.select-step\\(`), `${key} moves the selection`);
  }
  assert.match(scope.slice(handler.length + scope.indexOf(handler)), /^\s*return reject;\n\s*\}/, "the rest goes on");
  // Enter plays and Esc only gives the keys back to the video (`VestraFocus.release()`, at once).
  assert.match(slint, /^import \{ VestraFocus \} from "@vestra\/focus\.slint";$/m);
  assert.match(handler, /Key\.Return[^{]*\{\s+PlaylistBridge\.play-selected\(\);\s+VestraFocus\.release\(\);\s+return accept;/);
  assert.match(handler, /Key\.Escape[^{]*\{ VestraFocus\.release\(\); return accept; \}/);
  // A double-click plays, gives the keys back, and its release does not focus the list again.
  const double = slint.slice(slint.indexOf("double-clicked => {"));
  assert.match(
    double.slice(0, double.indexOf("list := ListView")),
    /PlaylistBridge\.play-row\(root\.press-row\);\s+VestraFocus\.release\(\);\s+root\.press-row = "";/,
  );
  assert.match(handler, /control \|\| event\.modifiers\.meta \|\| event\.modifiers\.alt \{ return reject; \}/);
  // A press no control takes does not focus the list, and none reaches the player's sidebar scope.
  assert.match(scope, /^\s+focus-on-click: false;/m);
  assert.match(scope, /^\s+TouchArea \{\}/m);
  // The list takes the focus in four places only: a row's release (no drag), a right press on a
  // row, and the search box's Esc and ↓; and it shows no focus ring (user ruling 2026-11-08).
  assert.equal([...slint.matchAll(/list-focus\.focus\(\)/g)].length, 4);
  assert.doesNotMatch(slint, /visible: list-focus\.has-focus;/);
});

test("reveal is titled Show in Folder on every system, worded as capture's notice; id and key unchanged", async () => {
  for (const [locale, title] of [["en", "Show in Folder"], ["zh-Hans", "在文件夹中显示"]] as const) {
    const app = await start(fakeProbe(), locale);
    try {
      const reveal = app.command("reveal");
      assert.equal(reveal.display.title, title, locale);
      assert.deepEqual(reveal.declaration.bindings, ["Ctrl+F"]);
      assert.equal(reveal.declaration.focus, "panel");
    } finally {
      await app.close();
    }
  }
  // Every language says it as capture's "Show in Folder" does; "Finder" is gone from the texts.
  const msgstr = (po: string) => po.match(/^msgid "Show in Folder"\nmsgstr "(.*)"$/m)?.[1];
  for (const lang of ["de", "es", "fr", "ja", "ko", "zh-Hans"]) {
    const own = await readFile(new URL(`../locales/${lang}.po`, import.meta.url), "utf8");
    const capture = await readFile(new URL(`../../capture/locales/${lang}.po`, import.meta.url), "utf8");
    assert.ok(msgstr(own), `${lang} translates Show in Folder`);
    assert.equal(msgstr(own), msgstr(capture), lang);
    assert.doesNotMatch(own, /Finder/, lang);
  }
});
