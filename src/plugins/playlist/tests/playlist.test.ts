import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { filesPending, settleFiles } from "./settle.ts";
import activate from "../main.ts";
import { Prober, type ProbeRunner } from "../probe.ts";
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
async function start(probe = fakeProbe(), locale: "en" | "zh-Hans" = "zh-Hans") {
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
    assert.deepEqual(app.command("fast-advance").display.path, ["Playback", "Playlist", "Fast Advance to Next"]);
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
    // As a Simplified Chinese user sees the menus (the declarations are English, see below).
    const declared = [...app.host.commands.values()].map(({ declaration: d, display }) => [
      display.path.join("/") || `(${display.title})`,
      d.order,
      d.group,
      d.bindings.join(" "),
      d.focus ?? "",
    ]);
    assert.deepEqual(declared, [
      ["播放列表", 800, "windows", "F6", ""],
      ["打开/打开文件夹…", 110, "open", "F2", ""],
      ["播放/播放列表/添加文件…", 230, "domain", "", ""],
      ["播放/播放列表/添加链接…", 231, "domain", "", ""],
      ["播放/播放列表/添加文件夹…", 232, "domain", "", ""],
      ["播放/播放列表/循环/不循环", 233, "domain", "", ""],
      ["播放/播放列表/循环/全部", 234, "domain", "", ""],
      ["播放/播放列表/循环/单个", 235, "domain", "", ""],
      ["播放/播放列表/随机", 236, "domain", "", ""],
      ["播放/播放列表/打开列表文件…", 237, "list", "", ""],
      ["播放/播放列表/保存列表…", 238, "list", "", ""],
      ["播放/播放列表/打开文件时/仅打开该文件", 239, "open-mode", "", ""],
      ["播放/播放列表/打开文件时/加入相似文件", 240, "open-mode", "", ""],
      ["播放/播放列表/打开文件时/加入同文件夹全部文件", 241, "open-mode", "", ""],
      ["播放/播放列表/快速推进到下一个", 242, "options", "", ""],
      ["播放/播放列表/记住列表", 243, "options", "", ""],
      ["播放/播放列表/移除全部前确认", 244, "options", "", ""],
      ["(播放)", 0, "domain", "Enter P", "panel"],
      ["(添加文件…)", 0, "domain", "Ctrl+I", "panel"],
      ["(添加文件夹…)", 0, "domain", "Ctrl+O", "panel"],
      ["(添加链接…)", 0, "domain", "Ctrl+U", "panel"],
      ["(移除所选)", 0, "domain", "Delete Backspace", "panel"],
      ["(移除未选)", 0, "domain", "Ctrl+Delete", "panel"],
      ["(移除无效项)", 0, "domain", "Ctrl+T", "panel"],
      ["(移除全部)", 0, "domain", "Alt+Delete", "panel"],
      ["(在 Finder 中显示)", 0, "domain", "Ctrl+F", "panel"],
      ["(置顶)", 0, "domain", "Ctrl+Home", "panel"],
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
      ["(打开列表文件…)", 0, "domain", "F3", "panel"],
      ["(保存列表…)", 0, "domain", "F2", "panel"],
      ["(关闭列表)", 0, "domain", "F6", "panel"],
      ["(播放列表设置…)", 0, "domain", "F5", "panel"],
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

test("F6, the tab-strip close button and the menu check share the sidebar preference; detach toggles", async () => {
  const app = await start();
  try {
    assert.equal(app.command("visibility").state.checked, true);
    await app.emit("hide");
    assert.equal(app.host.layout.sidebar?.preferred, false);
    assert.equal(app.command("visibility").state.checked, false);
    await app.host.press("F6");
    await app.host.settle();
    assert.equal(app.host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
    await app.press("F6"); // with list focus F6 closes the list
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
    // Focus in the list: F6 (关闭列表) and the own close button hide the whole sidebar.
    await app.press("F6");
    assert.equal(host.layout.sidebar?.preferred, false);
    await app.run("visibility");
    assert.equal(host.layout.sidebar?.preferred, true);
    await app.emit("hide");
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
    // P plays too; a double-click plays the clicked row.
    await app.click(1);
    await app.press("P");
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
    assert.equal(app.prop("can-up"), true);
    assert.equal(app.prop("can-down"), false);
    await app.press("Ctrl+ArrowUp");
    assert.deepEqual(app.names(), ["a.mp4", "c.mp4", "b.mp4"]);
    await app.press("Ctrl+Home");
    assert.deepEqual(app.names(), ["c.mp4", "a.mp4", "b.mp4"]);
    assert.equal(app.command("move-top").state.enabled, false);
    await app.press("Ctrl+End");
    assert.deepEqual(app.names(), ["a.mp4", "b.mp4", "c.mp4"]);
    await app.click(0);
    await app.emit("place", "down");
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
    // 13 commands and 6 separators, single level (Panel.showMenu: at most 32).
    assert.deepEqual(app.host.menus.at(-1), {
      items: [
        "playlist.play",
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

test("errors, stops and other sources never advance; another open of the same path is not taken over or announced", async () => {
  const app = await start();
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
