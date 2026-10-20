import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "../../../sdk/plugin/testing.ts";
import { exempt, fromSlintKey, label, sameKey } from "../keys.ts";
import activate from "../main.ts";
import { timing } from "../store.ts";

Object.assign(timing, { reload: 5, write: 5 });
const WAIT = 40;

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-hotkeys-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const running = new Set<() => Promise<void>>();
async function start(host = new TestHost(), dataDirectory?: string) {
  // A playing file, so the core actions bound to keys are enabled.
  host.openElsewhere("/media/movie.mkv");
  host.setSource({ duration: 600, position: 10 });
  const started = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  await host.settle(WAIT);
  return { ...started, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

const key = (text: string, mods: Partial<{ control: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) =>
  fromSlintKey({ text, control: false, alt: false, shift: false, meta: false, ...mods });
const overlay = (host: TestHost) => Object.fromEntries(host.bindings.overlay);
type Row = { key: string; command: string; keys: string; status: string; selected: boolean; tip: string };
const rows = (host: TestHost, id = "keys") => host.window(id).props.rows as Row[];
const row = (host: TestHost, command: string) => rows(host).find((r) => r.command === command);

async function openKeys(host: TestHost) {
  await host.command("keys").run();
  await host.settle(WAIT);
  return host.window("keys");
}
async function emit(host: TestHost, view: string, callback: string, ...args: (string | number | boolean)[]) {
  host.window(view).emit(callback, ...args);
  await host.settle(WAIT);
}
/** Select a command in J01 and capture a key for it (Slint key event). */
async function capture(host: TestHost, ref: string, text: string, mods: Partial<{ control: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) {
  await emit(host, "keys", "select", ref);
  await emit(host, "keys", "addKey");
  assert.equal(host.window("keys").props.capturing, true);
  await emit(host, "keys", "key", text, !!mods.control, !!mods.alt, !!mods.shift, !!mods.meta);
}

// ------------------------------------------------------------------ helpers

test("capture: Slint key events to canonical bindings", () => {
  assert.deepEqual(key("o", { control: true }), { input: "Ctrl+O" }); // ⌘O
  assert.deepEqual(key("\u{f}", { meta: true }), { input: "Ctrl+O" }); // ⌃O as a control code
  assert.deepEqual(key("\u{9}", { meta: true }), { input: "Ctrl+I" }); // ⌃I is a Tab code
  assert.deepEqual(key("\u{8}", { control: true }), { input: "Ctrl+Backspace" }); // ⌘⌫
  assert.deepEqual(key("\u{1b}", { control: true }), { input: "Ctrl+Escape" });
  assert.deepEqual(key("k"), { input: "K" });
  assert.deepEqual(key("E", { shift: true }), { input: "Shift+E" });
  assert.deepEqual(key("?", { shift: true }), { input: "?" }); // Shift+/
  assert.deepEqual(key(">", { shift: true }), { input: ">" });
  assert.deepEqual(key("+", { shift: true }), { input: "Plus" });
  assert.deepEqual(key("=", { control: true }), { input: "Ctrl+=" });
  assert.deepEqual(key("5"), { input: "5" });
  assert.deepEqual(key(" "), { input: "Space" });
  assert.deepEqual(key("\n"), { input: "Enter" });
  assert.deepEqual(key("\u{8}"), { input: "Backspace" });
  assert.deepEqual(key("\u{7f}"), { input: "Delete" });
  assert.deepEqual(key("\u{19}", { shift: true }), { input: "Shift+Tab" });
  assert.deepEqual(key("\u{F702}", { control: true }), { input: "Ctrl+ArrowLeft" });
  assert.deepEqual(key("\u{F708}"), { input: "F5" });
  assert.deepEqual(key("\u{F72C}", { alt: true }), { input: "Alt+PageUp" });
  // Option: US Option characters go back to their key; other symbols stay.
  assert.deepEqual(key("ø", { alt: true }), { input: "Alt+O" });
  assert.deepEqual(key("Ø", { alt: true, shift: true }), { input: "Alt+Shift+O" });
  assert.deepEqual(key("˘", { alt: true, shift: true }), { input: "Alt+>" });
  assert.deepEqual(key("“", { alt: true }), { input: "Alt+[" });
  assert.deepEqual(key("´", { alt: true }), { input: "Alt+E" }); // dead key
  assert.deepEqual(key("[", { alt: true }), { input: "[" }); // German ⌥5
  assert.deepEqual(key("@", { alt: true }), { input: "@" }); // German ⌥L
  assert.deepEqual(key("l", { alt: true }), { input: "Alt+L" });
  // Waiting, cancelling, refusing.
  assert.equal(key("\u{10}", { shift: true }), null); // Shift alone
  assert.equal(key("\u{11}", { control: true }), null); // ⌘ alone
  assert.equal(key(""), null); // dead key without text
  assert.deepEqual(key("\u{1b}"), { cancel: true });
  assert.deepEqual(key("é"), { error: "无法使用此按键" });
});

test("labels, same key, safe exits", () => {
  assert.equal(label("Ctrl+ArrowLeft"), "Ctrl+←");
  assert.equal(label("Numpad5"), "Num5");
  assert.equal(label("Plus"), "+");
  assert.equal(label("Ctrl+Mouse:WheelUp"), "Ctrl+滚轮上推");
  assert.equal(label("Mouse:LeftClick"), "左键单击");
  assert.ok(sameKey("Ctrl+Q", "Cmd+Q"));
  assert.ok(sameKey("Space", "Space"));
  assert.ok(!sameKey("Ctrl+Q", "Alt+Q"));
  assert.ok(!sameKey("Q", "Ctrl+Q"));
  assert.ok(exempt({ plugin: "host", command: "quit" }));
  assert.ok(exempt({ plugin: "host", command: "exit-fullscreen" }));
  assert.ok(!exempt({ plugin: "host", command: "fullscreen" }));
});

// ------------------------------------------------------------------ menu

test("menu: 设置 ▶ 快捷键… / 鼠标… / 忽略默认快捷键, no bindings of its own", async () => {
  const host = new TestHost();
  await start(host);
  const decl = (id: string) => host.command(id).declaration;
  assert.deepEqual(decl("keys").path, ["设置", "快捷键…"]);
  assert.deepEqual(decl("mouse").path, ["设置", "鼠标…"]);
  assert.deepEqual(decl("ignore-defaults").path, ["设置", "忽略默认快捷键"]);
  assert.deepEqual(
    ["keys", "mouse", "ignore-defaults"].map((id) => [decl(id).order, decl(id).group]),
    [
      [832, "settings-input"],
      [833, "settings-input"],
      [834, "settings-input"],
    ],
  );
  for (const id of ["keys", "mouse", "ignore-defaults"]) assert.deepEqual(decl(id).bindings, []);
  assert.equal(host.command("ignore-defaults").state.checked, false);
  assert.deepEqual(host.conflicts, []);
});

// ------------------------------------------------------------------ J01

test("J01: the list shows every command with keys, scope, source, status", async () => {
  const host = new TestHost();
  await start(host);
  const window = await openKeys(host);
  assert.equal(window.visible, true);
  const play = row(host, "播放 ▶ 播放/暂停")!;
  assert.equal(play.keys, "Space");
  assert.equal(play.status, "默认");
  assert.equal(row(host, "声音 ▶ 音量 +")!.keys, "ArrowUp"); // mouse bindings are not keys
  const own = row(host, "设置 ▶ 快捷键…")!;
  assert.equal(own.keys, "");
  assert.deepEqual(
    (window.props["source-model"] as string[]).slice(0, 3),
    ["全部来源", "宿主", "vestra.hotkeys"],
  );
  assert.equal((window.props["source-model"] as string[]).at(-1), "未加载的插件");
  // Sorted by command text; the header flips the direction.
  const commands = rows(host).map((r) => r.command);
  assert.deepEqual(commands, [...commands].sort((a, b) => a.localeCompare(b, "zh-Hans-CN")));
  await emit(host, "keys", "sortBy", "command");
  assert.equal(window.props.descending, true);
  assert.deepEqual(rows(host).map((r) => r.command), [...commands].reverse());
});

test("J01: add a key; it runs the command, shows on the row, survives as an override", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await capture(host, "host/play-pause", "k");
  assert.deepEqual(overlay(host), { "host/play-pause": ["Space", "K"] });
  assert.equal(row(host, "播放 ▶ 播放/暂停")!.keys, "Space, K");
  assert.equal(row(host, "播放 ▶ 播放/暂停")!.status, "已修改");
  assert.equal(host.window("keys").props.capturing, false);
  assert.deepEqual(
    (host.window("keys").props.tags as { input: string }[]).map((t) => t.input),
    ["Space", "K"],
  );
  assert.equal(host.osd.at(-1), "快捷键已更新：播放 ▶ 播放/暂停 = Space, K");
  await host.press("K");
  assert.equal(host.coreRuns.at(-1), "play-pause");
  // Already there; a modifier alone keeps waiting; Esc cancels.
  await capture(host, "host/play-pause", "K", { shift: false });
  assert.match(host.window("keys").props.message as string, /已经是这条命令的快捷键/);
  await emit(host, "keys", "addKey");
  await emit(host, "keys", "key", "\u{10}", false, false, true, false);
  assert.equal(host.window("keys").props.capturing, true);
  await emit(host, "keys", "key", "\u{1b}", false, false, false, false);
  assert.equal(host.window("keys").props.capturing, false);
  assert.deepEqual(overlay(host), { "host/play-pause": ["Space", "K"] });
});

test("J01: a key in use asks first; 替换 moves it, 取消 leaves both", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await capture(host, "host/mute", " ");
  const window = host.window("keys");
  assert.equal(window.props.pending, true);
  assert.equal(window.props["pending-text"], "Space 已用于「播放 ▶ 播放/暂停」");
  assert.deepEqual(overlay(host), {});
  await emit(host, "keys", "cancelReplace");
  assert.equal(window.props.pending, false);
  assert.deepEqual(overlay(host), {});

  await capture(host, "host/mute", " ");
  await emit(host, "keys", "replace");
  assert.deepEqual(overlay(host), { "host/play-pause": [], "host/mute": ["M", "Space"] });
  await host.press("Space");
  assert.equal(host.coreRuns.at(-1), "mute");
  assert.equal(row(host, "播放 ▶ 播放/暂停")!.status, "已修改");
  assert.equal(row(host, "播放 ▶ 播放/暂停")!.keys, "");
});

test("J01: when the second write fails the first is undone", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  host.handle("bindings.set", (p) => {
    if (p.command === "mute") throw new HostError("invalid", "rejected");
    host.bindings.overlay.set(`${p.plugin}/${p.command}`, [...(p.inputs as string[])]);
    host.emit("bindings.changed", null);
  });
  await capture(host, "host/mute", " ");
  await emit(host, "keys", "replace");
  await host.settle(WAIT);
  assert.deepEqual(overlay(host), {}); // play-pause back to its defaults
  assert.equal(host.window("keys").props.message, "无法使用此按键");
  assert.equal(host.window("keys").props["message-warn"], true);
});

test("J01: remove a tag, unbind all, restore one, restore all (asks)", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await emit(host, "keys", "select", "host/volume-up");
  await emit(host, "keys", "removeTag", "Mouse:WheelUp");
  assert.deepEqual(overlay(host), { "host/volume-up": ["ArrowUp"] });
  await emit(host, "keys", "unbindAll");
  assert.deepEqual(overlay(host), { "host/volume-up": [] });
  assert.equal(host.osd.at(-1), "快捷键已更新：声音 ▶ 音量 + = （无）");
  await emit(host, "keys", "resetOne");
  assert.deepEqual(overlay(host), {});
  assert.equal(row(host, "声音 ▶ 音量 +")!.status, "默认");

  await capture(host, "host/stop", "q");
  assert.deepEqual(overlay(host), { "host/stop": ["F4", "Q"] });
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await emit(host, "keys", "resetAll");
  assert.deepEqual(overlay(host), { "host/stop": ["F4", "Q"] });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "keys", "resetAll");
  assert.deepEqual(overlay(host), {});
  assert.equal(host.confirms.at(-1)!.destructive, true);
});

test("J01: at most 8 bindings; an invalid key is refused", async () => {
  const host = new TestHost();
  host.bindings.overlay.set("host/mute", ["M", "1", "2", "3", "4", "5", "6", "7"]);
  await start(host);
  await openKeys(host);
  await capture(host, "host/mute", "9");
  assert.match(host.window("keys").props.message as string, /最多 8 个/);
  assert.deepEqual(overlay(host)["host/mute"], ["M", "1", "2", "3", "4", "5", "6", "7"]);
  await capture(host, "host/stop", "é");
  assert.equal(host.window("keys").props.message, "无法使用此按键");
});

test("J01: filters by text (Ctrl+), status and source; orphans can be deleted", async () => {
  const host = new TestHost();
  host.bindings.overlay.set("host/stop", ["Ctrl+W"]);
  host.bindings.overlay.set("gone.plugin/thing", ["Ctrl+T"]);
  await start(host);
  const window = await openKeys(host);
  window.emit("search", "ctrl+");
  await host.settle(WAIT);
  assert.deepEqual(rows(host).map((r) => r.command), ["关闭"]);
  window.emit("search", "");
  await emit(host, "keys", "statusFilter", 1);
  assert.deepEqual(rows(host).map((r) => r.command), ["关闭"]);
  await emit(host, "keys", "statusFilter", 3);
  assert.ok(rows(host).every((r) => r.keys === ""));
  assert.ok(rows(host).some((r) => r.command === "设置 ▶ 快捷键…"));
  await emit(host, "keys", "statusFilter", 0);
  const sources = window.props["source-model"] as string[];
  await emit(host, "keys", "sourceFilter", sources.indexOf("vestra.hotkeys"));
  assert.deepEqual(rows(host).map((r) => r.command).sort(), ["设置 ▶ 快捷键…", "设置 ▶ 忽略默认快捷键", "设置 ▶ 鼠标…"].sort());
  await emit(host, "keys", "sourceFilter", sources.indexOf("未加载的插件"));
  assert.deepEqual(rows(host).map((r) => [r.command, r.keys, r.status]), [["gone.plugin/thing", "Ctrl+T", "未加载"]]);
  await emit(host, "keys", "select", rows(host)[0]!.key);
  assert.equal(window.props["reset-text"], "删除此条");
  await emit(host, "keys", "resetOne");
  assert.deepEqual(overlay(host), { "host/stop": ["Ctrl+W"] });
  assert.equal(rows(host).length, 0);
  // The filters are remembered (hotkeys.json).
  const saved = JSON.parse(readFileSync(join(host.dataDirectory, "hotkeys.json"), "utf8"));
  assert.equal(saved.source, "*orphans");
});

test("J01: a conflict between defaults is shown with its holder", async () => {
  const host = new TestHost();
  host.handle("bindings.list", () => ({
    commands: [
      { plugin: "host", command: "play-pause", title: "播放/暂停", path: ["播放", "播放/暂停"], scope: "global", defaults: ["Space"], current: ["Space"], overridden: false, shadowed: [], labels: ["Space"] },
      { plugin: "community.x", command: "go", title: "Go", path: null, scope: "global", defaults: ["Space", "G"], current: ["Space", "G"], overridden: false, shadowed: [{ input: "Space", by: { plugin: "host", command: "play-pause" } }], labels: ["Space", "G"] },
    ],
    orphans: [],
  }));
  await start(host);
  await openKeys(host);
  const go = row(host, "Go")!;
  assert.equal(go.status, "冲突");
  assert.equal(go.tip, "Space 被「播放 ▶ 播放/暂停」占用，未生效");
  await emit(host, "keys", "select", "community.x/go");
  assert.equal(host.window("keys").props.message, go.tip);
});

test("J01: export the list as CSV or text", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  const csv = join(scratch, "keys.csv");
  host.dialogAnswers.push([csv]);
  await emit(host, "keys", "export");
  const text = readFileSync(csv, "utf8");
  assert.ok(text.startsWith("\u{feff}命令,快捷键,范围,来源,状态\n"));
  assert.match(text, /\n播放 ▶ 播放\/暂停,Space,全局,宿主,默认\n/);
  const txt = join(scratch, "keys.txt");
  host.dialogAnswers.push([txt]);
  await emit(host, "keys", "export");
  assert.match(readFileSync(txt, "utf8"), /播放 ▶ 播放\/暂停\tSpace\t全局\t宿主\t默认/);
  host.dialogAnswers.push(null);
  await emit(host, "keys", "export");
});

test("J01: the list follows changes made elsewhere; the selection stays", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await emit(host, "keys", "select", "host/mute");
  host.bindings.overlay.set("host/mute", ["N"]);
  host.emit("bindings.changed", null);
  await host.settle(WAIT);
  assert.equal(row(host, "声音 ▶ 静音")!.keys, "N");
  assert.equal(row(host, "声音 ▶ 静音")!.selected, true);
  await emit(host, "keys", "close");
  assert.equal(host.windows.get("keys")?.visible ?? false, false);
});

// ------------------------------------------------------------------ J02

test("J02: mouse rows show the command each input runs; choosing moves the input", async () => {
  const host = new TestHost();
  await start(host);
  await host.command("mouse").run();
  await host.settle(WAIT);
  const window = host.window("mouse");
  type MouseRow = { label: string; command: string; resettable: boolean };
  const mouseRows = () => window.props.rows as MouseRow[];
  assert.equal(mouseRows().length, 11);
  assert.deepEqual(mouseRows()[7], { label: "滚轮上推", command: "声音 ▶ 音量 +", bound: true, resettable: false });
  assert.equal(mouseRows()[0]!.command, "（无）");

  await emit(host, "mouse", "pick", 7);
  assert.equal(window.props.picking, true);
  const items = window.props["pick-items"] as { key: string; text: string; selected: boolean }[];
  assert.equal(items[0]!.text, "（无）");
  assert.ok(items.find((i) => i.key === "host/volume-up")!.selected);
  window.emit("filter", "前进");
  await host.settle(WAIT);
  const texts = (window.props["pick-items"] as { text: string }[]).map((i) => i.text);
  assert.equal(texts[0], "（无）");
  assert.ok(texts.length >= 2 && texts.slice(1).every((t) => t.includes("前进")));
  await emit(host, "mouse", "choose", "host/seek-forward");
  assert.equal(window.props.picking, false);
  assert.deepEqual(overlay(host), {
    "host/volume-up": ["ArrowUp"],
    "host/seek-forward": ["ArrowRight", "Mouse:WheelUp"],
  });
  assert.equal(host.osd.at(-1)!.startsWith("鼠标动作已更新：滚轮上推 = 播放 ▶ 跳转 ▶ "), true);
  await host.press("Mouse:WheelUp");
  assert.equal(host.coreRuns.at(-1), "seek-forward");
  assert.equal(mouseRows()[7]!.resettable, true);

  // 默认: back to 音量 +, and both commands back to their defaults.
  await emit(host, "mouse", "setDefault", 7);
  assert.deepEqual(overlay(host), {});
  assert.equal(mouseRows()[7]!.command, "声音 ▶ 音量 +");
});

test("J02: left click and a modifier; （无） only takes the input", async () => {
  const host = new TestHost();
  await start(host);
  await host.command("mouse").run();
  await host.settle(WAIT);
  await emit(host, "mouse", "pick", 0);
  await emit(host, "mouse", "choose", "host/play-pause");
  assert.deepEqual(overlay(host), { "host/play-pause": ["Space", "Mouse:LeftClick"] });
  await host.press("Mouse:LeftClick");
  assert.equal(host.coreRuns.at(-1), "play-pause");

  await emit(host, "mouse", "setModifier", 1); // Ctrl
  assert.equal((host.window("mouse").props.rows as { command: string }[])[0]!.command, "（无）");
  await emit(host, "mouse", "pick", 7);
  await emit(host, "mouse", "choose", "host/mute");
  assert.deepEqual(overlay(host)["host/mute"], ["M", "Ctrl+Mouse:WheelUp"]);
  await host.press("Ctrl+Mouse:WheelUp");
  assert.equal(host.coreRuns.at(-1), "mute");

  await emit(host, "mouse", "setModifier", 0);
  await emit(host, "mouse", "pick", 8); // 滚轮下推
  await emit(host, "mouse", "choose", "");
  assert.deepEqual(overlay(host)["host/volume-down"], ["ArrowDown"]);
  await assert.rejects(host.press("Mouse:WheelDown"));
});

// ------------------------------------------------------------------ J03

test("J03: ignoring defaults (asks first), changed keys keep working, off restores", async () => {
  const host = new TestHost();
  await start(host);
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await host.command("ignore-defaults").run();
  await host.settle(WAIT);
  assert.equal(host.command("ignore-defaults").state.checked, false);
  assert.deepEqual(overlay(host), {});

  host.dialogAnswers.push({ confirmed: true, checked: false });
  await host.command("ignore-defaults").run();
  await host.settle(WAIT);
  assert.equal(host.command("ignore-defaults").state.checked, true);
  assert.equal(host.osd.at(-1), "忽略默认快捷键：开");
  assert.deepEqual(overlay(host)["host/play-pause"], []);
  assert.deepEqual(overlay(host)["host/volume-up"], []);
  await assert.rejects(host.press("Space"));
  await assert.rejects(host.press("Mouse:WheelUp"));
  const record = JSON.parse(readFileSync(join(host.dataDirectory, "ignore-defaults.json"), "utf8"));
  assert.equal(record.enabled, true);
  assert.equal(record.written.length, 14); // the 14 core actions (hotkeys declares none)

  // J01 shows 已忽略; a key added meanwhile is the user's.
  await openKeys(host);
  assert.equal(row(host, "声音 ▶ 静音")!.status, "已忽略");
  assert.equal(row(host, "声音 ▶ 静音")!.keys, "（M）");
  assert.equal(host.window("keys").props.ignore, true);
  await capture(host, "host/play-pause", "k");
  assert.deepEqual(overlay(host)["host/play-pause"], ["K"]);
  await host.press("K");
  assert.equal(host.coreRuns.at(-1), "play-pause");

  // A command that comes back on its defaults (a plugin loaded) is caught.
  host.bindings.overlay.delete("host/mute");
  host.emit("bindings.changed", null);
  await host.settle(WAIT);
  assert.deepEqual(overlay(host)["host/mute"], []);

  // Off (the J01 check box): only what J03 wrote is reset.
  await emit(host, "keys", "toggleIgnore");
  await host.settle(WAIT);
  assert.equal(host.command("ignore-defaults").state.checked, false);
  assert.deepEqual(overlay(host), { "host/play-pause": ["K"] });
  assert.equal(host.osd.at(-1), "忽略默认快捷键：关");
  await host.press("M");
  assert.equal(host.coreRuns.at(-1), "mute");
});

test("J03: checked at start; a half-finished switch-off is completed", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  writeFileSync(join(data, "ignore-defaults.json"), JSON.stringify({ version: 1, enabled: true, written: [] }));
  let host = new TestHost();
  let run = await start(host, data);
  await host.settle(WAIT);
  assert.deepEqual(overlay(host)["host/stop"], []);
  assert.equal(host.command("ignore-defaults").state.checked, true);
  await run.stop();

  // Restart: the switch was turned off but two overrides are still ours.
  writeFileSync(
    join(data, "ignore-defaults.json"),
    JSON.stringify({ version: 1, enabled: false, written: [{ plugin: "host", command: "stop" }, { plugin: "host", command: "mute" }] }),
  );
  host = new TestHost();
  host.bindings.overlay.set("host/stop", []);
  host.bindings.overlay.set("host/mute", ["N"]); // changed by the user meanwhile
  run = await start(host, data);
  await host.settle(WAIT);
  assert.deepEqual(overlay(host), { "host/mute": ["N"] });
  assert.deepEqual(JSON.parse(readFileSync(join(data, "ignore-defaults.json"), "utf8")).written, []);
});
