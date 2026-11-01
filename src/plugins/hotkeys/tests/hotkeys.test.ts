import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { Locale } from "@vestra-player/sdk";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { exempt, fromSlintKey, inputOf, label, sameKey } from "../keys.ts";
import activate from "../main.ts";
import { timing } from "../store.ts";

Object.assign(timing, { reload: 5, write: 5 });
const WAIT = 40;

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-hotkeys-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const running = new Set<() => Promise<void>>();
/** Start in English (the source texts) unless a locale is given. */
async function start(host = new TestHost(), dataDirectory?: string, locale: Locale = "en") {
  // A playing file, so the core actions bound to keys are enabled.
  host.openElsewhere("/media/movie.mkv");
  host.setSource({ duration: 600, position: 10 });
  const started = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
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

/** The settings page for a short name ("keys", "mouse"). */
const page = (host: TestHost, id: string) =>
  host.settingsPage(id === "keys" ? "hotkeys.keyboard" : id === "mouse" ? "hotkeys.mouse" : id);
const key = (text: string, mods: Partial<{ control: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) =>
  fromSlintKey({ text, control: false, alt: false, shift: false, meta: false, ...mods });
const overlay = (host: TestHost) => Object.fromEntries(host.bindings.overlay);
type Row = { key: string; command: string; keys: string; state: number; selected: boolean; conflicts: unknown[] };
const PLAY = "Playback ▸ Play/Pause";
const VOLUME_UP = "Audio ▸ Volume +";
const MUTE = "Audio ▸ Mute";
const KEYS = "Settings ▸ Keyboard Shortcuts…";
const DEFAULT = 0;
const CHANGED = 1;
const CONFLICT = 2;
const IGNORED = 3;
const ORPHAN = 4;
const message = (host: TestHost, id = "keys") => page(host, id).props.message as { kind: string; text: string; n: number; warn: boolean };
const rows = (host: TestHost, id = "keys") => page(host, id).props.rows as Row[];
const row = (host: TestHost, command: string) => rows(host).find((r) => r.command === command);

async function openKeys(host: TestHost) {
  await host.command("keys").run();
  await host.settle(WAIT);
  return page(host, "keys");
}
async function emit(host: TestHost, view: string, callback: string, ...args: (string | number | boolean)[]) {
  page(host, view).emit(callback, ...args);
  await host.settle(WAIT);
}
/** Select a command in J01 and capture a key for it (Slint key event). */
async function capture(host: TestHost, ref: string, text: string, mods: Partial<{ control: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) {
  await emit(host, "keys", "select", ref);
  await emit(host, "keys", "addKey");
  assert.equal(page(host, "keys").props.capturing, true);
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
  assert.deepEqual(key("é"), { error: "invalid" });
});

test("labels, same key, safe exits", () => {
  assert.equal(label("Ctrl+ArrowLeft"), "Ctrl+←");
  assert.equal(label("Numpad5"), "Num5");
  assert.equal(label("Plus"), "+");
  assert.equal(label("Ctrl+Mouse:WheelUp"), "Ctrl+Wheel up");
  assert.equal(label("Mouse:LeftClick"), "Left click");
  assert.deepEqual(inputOf("Ctrl+Mouse:WheelUp"), { prefix: "Ctrl+", mouse: 7, text: "" });
  assert.deepEqual(inputOf("Ctrl+O", "⌘O"), { prefix: "", mouse: -1, text: "⌘O" });
  assert.ok(sameKey("Ctrl+Q", "Cmd+Q"));
  assert.ok(sameKey("Space", "Space"));
  assert.ok(!sameKey("Ctrl+Q", "Alt+Q"));
  assert.ok(!sameKey("Q", "Ctrl+Q"));
  assert.ok(exempt({ plugin: "host", command: "quit" }));
  assert.ok(exempt({ plugin: "host", command: "exit-fullscreen" }));
  // Esc is Toggle Fullscreen's default (player 0.6.0): it stays, so fullscreen can be left.
  assert.ok(exempt({ plugin: "host", command: "fullscreen" }));
  assert.ok(!exempt({ plugin: "host", command: "enter-fullscreen" }));
  assert.ok(!exempt({ plugin: "host", command: "play-pause" }));
  assert.ok(!exempt({ plugin: "vestra.other", command: "fullscreen" }));
});

// ------------------------------------------------------------------ menu

test("menu: Settings ▸ Keyboard Shortcuts… / Mouse… / Ignore Default Shortcuts, no bindings of its own", async () => {
  const host = new TestHost();
  await start(host);
  const decl = (id: string) => host.command(id).declaration;
  assert.deepEqual(decl("keys").path, ["@settings", "Keyboard Shortcuts…"]);
  assert.deepEqual(decl("mouse").path, ["@settings", "Mouse…"]);
  assert.deepEqual(decl("ignore-defaults").path, ["@settings", "Ignore Default Shortcuts"]);
  assert.deepEqual(host.command("keys").display.path, ["Settings", "Keyboard Shortcuts…"]);
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
  // J01, J02 are pages of the options window (category input).
  for (const [id, title, order] of [["hotkeys.keyboard", "Keyboard Shortcuts", 10], ["hotkeys.mouse", "Mouse", 20]] as const) {
    const registered = host.settingsPage(id);
    assert.deepEqual([registered.category, registered.title, registered.order], ["input", title, order]);
  }
});

test("menu and pages in Simplified Chinese, as before the English source texts", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  assert.deepEqual(host.command("keys").display.path, ["设置", "快捷键…"]);
  assert.deepEqual(host.command("mouse").display.path, ["设置", "鼠标…"]);
  assert.deepEqual(host.command("ignore-defaults").display.path, ["设置", "忽略默认快捷键"]);
  assert.equal(host.settingsPage("hotkeys.keyboard").title, "快捷键");
  assert.equal(host.settingsPage("hotkeys.mouse").title, "鼠标");
});

test("the Simplified Chinese catalog covers every text with the old words", () => {
  const po = readFileSync(new URL("../locales/zh-Hans.po", import.meta.url), "utf8");
  const pot = readFileSync(new URL("../locales/messages.pot", import.meta.url), "utf8");
  const ids = (text: string) => [...text.matchAll(/^msgid (".*")$/gm)].map((m) => JSON.parse(m[1]!) as string);
  assert.deepEqual(ids(po), ids(pot));
  assert.equal([...po.matchAll(/^msgstr(?:\[0\])? ""\n\n/gm)].length, 0); // every entry translated
  for (const [english, chinese] of [
    ["{0} is taken by “{1}” and has no effect", "{0} 被「{1}」占用，未生效"],
    ["Press Esc to cancel; to bind Esc itself, click “Esc” next to the box", "按 Esc 取消；要绑定 Esc 本身，点旁边的「Esc」"],
    ["Wheel up", "滚轮上推"],
    ["Player", "宿主"],
  ])
    assert.ok(po.includes(`msgid ${JSON.stringify(english)}\nmsgstr ${JSON.stringify(chinese)}`), english);
});

// ------------------------------------------------------------------ J01

test("J01: the list shows every command with keys, scope, source, status", async () => {
  const host = new TestHost();
  await start(host);
  const window = await openKeys(host);
  assert.deepEqual(host.settings.shows, ["hotkeys.keyboard"]);
  assert.equal(host.settings.current, "hotkeys.keyboard");
  const play = row(host, PLAY)!;
  assert.equal(play.keys, "Space");
  assert.equal(play.state, DEFAULT);
  assert.equal(row(host, VOLUME_UP)!.keys, "ArrowUp"); // mouse bindings are not keys
  const own = row(host, KEYS)!;
  assert.equal(own.keys, "");
  // The library drop-down shows its model: the words arrive translated, plugin ids as they are.
  assert.deepEqual(
    (window.props["source-model"] as string[]).slice(0, 3),
    ["All sources", "Player", "vestra.hotkeys"],
  );
  assert.equal((window.props["source-model"] as string[]).at(-1), "Plugins not loaded");
  assert.equal(window.props.loading, false);
  // Sorted by command text; the header flips the direction.
  const commands = rows(host).map((r) => r.command);
  assert.deepEqual(commands, [...commands].sort((a, b) => a.localeCompare(b, "zh-Hans-CN")));
  // The selected command: path, source, scope (ui/keys.slint words the title).
  await emit(host, "keys", "select", "host/play-pause");
  assert.deepEqual(
    ["edit-command", "edit-source", "edit-panel", "edit-loaded", "orphan"].map((name) => window.props[name]),
    [PLAY, "host", false, true, false],
  );
  await emit(host, "keys", "sortBy", "command");
  assert.equal(window.props.descending, true);
  assert.deepEqual(rows(host).map((r) => r.command), [...commands].reverse());
});

test("J01: add a key; it runs the command, shows on the row, survives as an override", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await capture(host, "host/play-pause", "k");
  assert.deepEqual(overlay(host), { "host/play-pause": ["Space", "Mouse:LeftClick", "K"] });
  assert.equal(row(host, PLAY)!.keys, "Space, K");
  assert.equal(row(host, PLAY)!.state, CHANGED);
  assert.equal(page(host, "keys").props.capturing, false);
  assert.deepEqual(
    (page(host, "keys").props.tags as { input: string }[]).map((t) => t.input),
    ["Space", "Mouse:LeftClick", "K"],
  );
  assert.equal(host.osd.at(-1), `Shortcut updated: ${PLAY} = Space, K`);
  assert.deepEqual([message(host).kind, message(host).text], ["updated", "Space, K"]);
  await host.press("K");
  assert.equal(host.coreRuns.at(-1), "play-pause");
  // Already there; a modifier alone keeps waiting; Esc cancels.
  await capture(host, "host/play-pause", "K", { shift: false });
  assert.equal(message(host).kind, "already");
  await emit(host, "keys", "addKey");
  await emit(host, "keys", "key", "\u{10}", false, false, true, false);
  assert.equal(page(host, "keys").props.capturing, true);
  await emit(host, "keys", "key", "\u{1b}", false, false, false, false);
  assert.equal(page(host, "keys").props.capturing, false);
  assert.deepEqual(overlay(host), { "host/play-pause": ["Space", "Mouse:LeftClick", "K"] });
});

test("J01: a key in use asks first; Replace moves it, Cancel leaves both", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await capture(host, "host/mute", " ");
  const window = page(host, "keys");
  assert.equal(window.props.pending, true);
  assert.deepEqual(
    ["pending-kind", "pending-key", "pending-command"].map((name) => window.props[name]),
    ["used", "Space", PLAY],
  );
  assert.deepEqual(overlay(host), {});
  await emit(host, "keys", "cancelReplace");
  assert.equal(window.props.pending, false);
  assert.deepEqual(overlay(host), {});

  await capture(host, "host/mute", " ");
  await emit(host, "keys", "replace");
  // The mouse default (player 0.5.2) stays; only the key moves.
  assert.deepEqual(overlay(host), {
    "host/play-pause": ["Mouse:LeftClick"],
    "host/mute": ["M", "Space"],
  });
  await host.press("Space");
  assert.equal(host.coreRuns.at(-1), "mute");
  assert.equal(row(host, PLAY)!.state, CHANGED);
  assert.equal(row(host, PLAY)!.keys, "");
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
  assert.deepEqual([message(host).kind, message(host).warn], ["invalid", true]);
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
  assert.equal(host.osd.at(-1), `Shortcut updated: ${VOLUME_UP} = (none)`);
  assert.equal(message(host).kind, "updated-none");
  await emit(host, "keys", "resetOne");
  assert.deepEqual(overlay(host), {});
  assert.equal(row(host, VOLUME_UP)!.state, DEFAULT);

  await capture(host, "host/stop", "q");
  assert.deepEqual(overlay(host), { "host/stop": ["F4", "Q"] });
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await emit(host, "keys", "resetAll");
  assert.deepEqual(overlay(host), { "host/stop": ["F4", "Q"] });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "keys", "resetAll");
  assert.deepEqual(overlay(host), {});
  assert.equal(host.confirms.at(-1)!.destructive, true);
  assert.equal(host.confirms.at(-1)!.message, "Restore all default shortcuts?");
  assert.equal(host.osd.at(-1), "Shortcuts restored to defaults");
  assert.equal(message(host).kind, "restored");
});

test("J01: at most 8 bindings; an invalid key is refused", async () => {
  const host = new TestHost();
  host.bindings.overlay.set("host/mute", ["M", "1", "2", "3", "4", "5", "6", "7"]);
  await start(host);
  await openKeys(host);
  await capture(host, "host/mute", "9");
  assert.deepEqual([message(host).kind, message(host).n, message(host).warn], ["max", 8, true]);
  assert.deepEqual(overlay(host)["host/mute"], ["M", "1", "2", "3", "4", "5", "6", "7"]);
  await capture(host, "host/stop", "é");
  assert.equal(message(host).kind, "invalid");
});

test("J01: filters by text (Ctrl+), status and source; orphans can be deleted", async () => {
  const host = new TestHost();
  host.bindings.overlay.set("host/stop", ["Ctrl+W"]);
  host.bindings.overlay.set("gone.plugin/thing", ["Ctrl+T"]);
  await start(host);
  const window = await openKeys(host);
  window.emit("search", "ctrl+");
  await host.settle(WAIT);
  assert.deepEqual(rows(host).map((r) => r.command), ["Close"]);
  window.emit("search", "");
  await emit(host, "keys", "statusFilter", 1);
  assert.deepEqual(rows(host).map((r) => r.command), ["Close"]);
  await emit(host, "keys", "statusFilter", 3);
  assert.ok(rows(host).every((r) => r.keys === ""));
  assert.ok(rows(host).some((r) => r.command === KEYS));
  await emit(host, "keys", "statusFilter", 0);
  const sources = window.props["source-model"] as string[];
  await emit(host, "keys", "sourceFilter", sources.indexOf("vestra.hotkeys"));
  assert.deepEqual(
    rows(host).map((r) => r.command).sort(),
    [KEYS, "Settings ▸ Ignore Default Shortcuts", "Settings ▸ Mouse…"].sort(),
  );
  await emit(host, "keys", "sourceFilter", sources.indexOf("Plugins not loaded"));
  assert.deepEqual(rows(host).map((r) => [r.command, r.keys, r.state]), [["gone.plugin/thing", "Ctrl+T", ORPHAN]]);
  await emit(host, "keys", "select", rows(host)[0]!.key);
  assert.equal(window.props.orphan, true);
  assert.equal(window.props["edit-loaded"], false);
  await emit(host, "keys", "resetOne");
  assert.deepEqual([message(host).kind, message(host).text], ["deleted", "gone.plugin/thing"]);
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
  assert.equal(go.state, CONFLICT);
  assert.deepEqual(go.conflicts, [{ input: { prefix: "", mouse: -1, text: "Space" }, by: "播放 ▸ 播放/暂停" }]);
  await emit(host, "keys", "select", "community.x/go");
  assert.equal(message(host).kind, "conflict");
  assert.deepEqual((page(host, "keys").props.message as { conflicts: unknown }).conflicts, go.conflicts);
});

test("J01: keys bound now read as the player labels them (macOS ⌘ style)", async () => {
  const host = new TestHost();
  host.handle("bindings.list", () => ({
    commands: [
      { plugin: "host", command: "open", title: "打开文件…", path: ["打开文件…"], scope: "global", defaults: ["Ctrl+O"], current: ["Ctrl+O"], overridden: false, shadowed: [], labels: ["⌘O"] },
      { plugin: "community.x", command: "go", title: "Go", path: null, scope: "global", defaults: ["Ctrl+O", "Ctrl+G"], current: ["Ctrl+O", "Ctrl+G"], overridden: false, shadowed: [{ input: "Ctrl+O", by: { plugin: "host", command: "open" } }], labels: ["⌘O", "⌘G"] },
    ],
    orphans: [],
  }));
  await start(host);
  await openKeys(host);
  const go = row(host, "Go")!;
  assert.equal(go.keys, "⌘O, ⌘G");
  assert.deepEqual(go.conflicts, [{ input: { prefix: "", mouse: -1, text: "⌘O" }, by: "打开文件…" }]);
});

test("J01: export the list as CSV or text", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  const csv = join(scratch, "keys.csv");
  host.dialogAnswers.push([csv]);
  await emit(host, "keys", "export");
  const text = readFileSync(csv, "utf8");
  assert.ok(text.startsWith("\u{feff}Command,Shortcut,Scope,Source,Status\n"));
  assert.ok(text.includes(`\n${PLAY},Space,Global,Player,Default\n`));
  const txt = join(scratch, "keys.txt");
  host.dialogAnswers.push([txt]);
  await emit(host, "keys", "export");
  assert.ok(readFileSync(txt, "utf8").includes(`${PLAY}\tSpace\tGlobal\tPlayer\tDefault`));
  host.dialogAnswers.push(null);
  await emit(host, "keys", "export");
});

test("J01: the exported list in Simplified Chinese has the words of before", async () => {
  const host = new TestHost();
  host.bindings.overlay.set("host/mute", ["N"]);
  await start(host, undefined, "zh-Hans");
  await openKeys(host);
  const csv = join(scratch, "keys-zh.csv");
  host.dialogAnswers.push([csv]);
  await emit(host, "keys", "export");
  assert.deepEqual(host.fileDialogs.at(-1), { kind: "saveFile", title: "导出快捷键列表", fileName: "Vestra Player 快捷键.csv" });
  const text = readFileSync(csv, "utf8");
  assert.ok(text.startsWith("\u{feff}命令,快捷键,范围,来源,状态\n"));
  assert.match(text, /\n播放 ▸ 播放\/暂停,Space,全局,宿主,默认\n/);
  assert.match(text, /,N,全局,宿主,已修改\n/);
  const txt = join(scratch, "keys-zh.txt");
  host.dialogAnswers.push([txt]);
  await emit(host, "keys", "export");
  const plain = readFileSync(txt, "utf8");
  assert.ok(plain.startsWith("Vestra Player 快捷键列表\n\n命令\t快捷键\t范围\t来源\t状态\n"));
  assert.match(plain, /播放 ▸ 播放\/暂停\tSpace\t全局\t宿主\t默认/);
});

test("J01: the list follows changes made elsewhere; the selection stays", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  await emit(host, "keys", "select", "host/mute");
  host.bindings.overlay.set("host/mute", ["N"]);
  host.emit("bindings.changed", null);
  await host.settle(WAIT);
  assert.equal(row(host, MUTE)!.keys, "N");
  assert.equal(row(host, MUTE)!.selected, true);
});

// ------------------------------------------------------------------ J02

test("J02: mouse rows show the command each input runs; choosing moves the input", async () => {
  const host = new TestHost();
  await start(host);
  await host.command("mouse").run();
  await host.settle(WAIT);
  const window = page(host, "mouse");
  assert.deepEqual(host.settings.shows, ["hotkeys.mouse"]);
  type MouseRow = { input: number; command: string; bound: boolean; resettable: boolean };
  const mouseRows = () => window.props.rows as MouseRow[];
  assert.equal(mouseRows().length, 11);
  assert.deepEqual(mouseRows()[7], { input: 7, command: VOLUME_UP, bound: true, resettable: false });
  // Left click: Play/Pause by default (player 0.5.2); double click: nothing.
  assert.deepEqual([mouseRows()[0]!.command, mouseRows()[0]!.bound], [PLAY, true]);
  assert.deepEqual([mouseRows()[1]!.command, mouseRows()[1]!.bound], ["", false]);
  assert.deepEqual(window.props.modifiers, ["None", "Ctrl", "Alt", "Shift", "Cmd"]);

  await emit(host, "mouse", "pick", 7);
  assert.equal(window.props.picking, true);
  const items = window.props["pick-items"] as { key: string; text: string; selected: boolean }[];
  assert.deepEqual(items[0], { key: "", text: "", selected: false });
  assert.deepEqual(window.props["pick-input"], { prefix: "", mouse: 7, text: "" });
  assert.ok(items.find((i) => i.key === "host/volume-up")!.selected);
  window.emit("filter", "forward");
  await host.settle(WAIT);
  const texts = (window.props["pick-items"] as { text: string }[]).map((i) => i.text);
  assert.equal(texts[0], "");
  assert.ok(texts.length >= 2 && texts.slice(1).every((t) => t.toLowerCase().includes("forward")));
  await emit(host, "mouse", "choose", "host/seek-forward");
  assert.equal(window.props.picking, false);
  assert.deepEqual(overlay(host), {
    "host/volume-up": ["ArrowUp"],
    "host/seek-forward": ["ArrowRight", "Mouse:WheelUp"],
  });
  assert.equal(host.osd.at(-1)!.startsWith("Mouse action updated: Wheel up = Playback ▸ Seek ▸ "), true);
  const updated = window.props.message as { kind: string; input: unknown; text: string };
  assert.deepEqual([updated.kind, updated.input], ["mouse-updated", { prefix: "", mouse: 7, text: "" }]);
  assert.ok(updated.text.startsWith("Playback ▸ Seek ▸ "));
  await host.press("Mouse:WheelUp");
  assert.equal(host.coreRuns.at(-1), "seek-forward");
  assert.equal(mouseRows()[7]!.resettable, true);

  // 默认: back to 音量 +, and both commands back to their defaults.
  await emit(host, "mouse", "setDefault", 7);
  assert.deepEqual(overlay(host), {});
  assert.equal(mouseRows()[7]!.command, VOLUME_UP);
});

test("J02: left click and a modifier; (None) only takes the input", async () => {
  const host = new TestHost();
  await start(host);
  await host.command("mouse").run();
  await host.settle(WAIT);
  await emit(host, "mouse", "pick", 0);
  await emit(host, "mouse", "choose", "host/stop");
  assert.deepEqual(overlay(host)["host/stop"], ["F4", "Mouse:LeftClick"]);
  // Player 0.5.2 binds a left click to Play/Pause by default; the click
  // moves to Close and Play/Pause keeps Space (older players: no default).
  assert.deepEqual(overlay(host)["host/play-pause"] ?? ["Space"], ["Space"]);
  await host.press("Mouse:LeftClick");
  assert.equal(host.coreRuns.at(-1), "stop");

  await emit(host, "mouse", "setModifier", 1); // Ctrl
  assert.equal((page(host, "mouse").props.rows as { bound: boolean }[])[0]!.bound, false);
  await emit(host, "mouse", "pick", 7);
  await emit(host, "mouse", "choose", "host/mute");
  assert.deepEqual(overlay(host)["host/mute"], ["M", "Ctrl+Mouse:WheelUp"]);
  await host.press("Ctrl+Mouse:WheelUp");
  assert.equal(host.coreRuns.at(-1), "mute");

  await emit(host, "mouse", "setModifier", 0);
  await emit(host, "mouse", "pick", 8); // Wheel down
  await emit(host, "mouse", "choose", "");
  assert.equal(host.osd.at(-1), "Mouse action updated: Wheel down = (none)");
  assert.equal((page(host, "mouse").props.message as { kind: string }).kind, "mouse-updated-none");
  assert.deepEqual(overlay(host)["host/volume-down"], ["ArrowDown"]);
  await assert.rejects(host.press("Mouse:WheelDown"), /nothing is bound to Mouse:WheelDown/, "(None) unbinds the input");
});

test("J02: the mouse OSD message in Simplified Chinese names the input in Chinese", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  await host.command("mouse").run();
  await host.settle(WAIT);
  await emit(host, "mouse", "setModifier", 1); // Ctrl
  await emit(host, "mouse", "pick", 7);
  await emit(host, "mouse", "choose", "host/mute");
  assert.match(host.osd.at(-1)!, /^鼠标动作已更新：Ctrl\+滚轮上推 = .+静音/);
  await emit(host, "mouse", "setModifier", 0);
  await emit(host, "mouse", "pick", 8);
  await emit(host, "mouse", "choose", "");
  assert.equal(host.osd.at(-1), "鼠标动作已更新：滚轮下推 = （无）");
});

/** A test host that reports `os` from `plugin.app.system()`. */
function hostOn(os: "macos" | "windows" | "linux") {
  const host = new TestHost();
  host.systemInfo = { ...host.systemInfo, os };
  return host;
}
const MODS = ["None", "Ctrl", "Alt", "Shift"];

test("J02: Cmd is a modifier on macOS only (Windows key elsewhere, as the menus)", async () => {
  for (const [os, modifiers] of [["macos", [...MODS, "Cmd"]], ["windows", MODS], ["linux", MODS]] as const) {
    const host = hostOn(os);
    await start(host);
    await host.command("mouse").run();
    await host.settle(WAIT);
    assert.deepEqual(page(host, "mouse").props.modifiers, modifiers, os);
    assert.equal(page(host, "mouse").props.modifier, 0, os);
    // An index past the list is ignored; on macOS index 4 is Cmd.
    await emit(host, "mouse", "setModifier", 4);
    assert.equal(page(host, "mouse").props.modifier, os === "macos" ? 4 : 0, os);
    await emit(host, "mouse", "setModifier", 3); // Shift
    await emit(host, "mouse", "pick", 7);
    await emit(host, "mouse", "choose", "host/mute");
    assert.deepEqual(overlay(host)["host/mute"], ["M", "Shift+Mouse:WheelUp"], os);
    for (const stop of [...running]) await stop();
  }
});

test("J02: a Cmd mouse binding kept on Windows is shown and can be changed", async () => {
  const host = hostOn("windows");
  host.bindings.overlay.set("host/mute", ["M", "Cmd+Mouse:WheelUp"]);
  await start(host);
  await host.command("mouse").run();
  await host.settle(WAIT);
  const window = page(host, "mouse");
  type MouseRow = { command: string; bound: boolean };
  assert.deepEqual(window.props.modifiers, [...MODS, "Cmd"]);
  assert.equal(window.props.modifier, 0, "None stays chosen");
  await emit(host, "mouse", "setModifier", 4);
  assert.equal(window.props.modifier, 4);
  assert.deepEqual([(window.props.rows as MouseRow[])[7]!.command, (window.props.rows as MouseRow[])[7]!.bound], [MUTE, true]);
  // (None) takes it; the radio stays while chosen, then goes.
  await emit(host, "mouse", "pick", 7);
  await emit(host, "mouse", "choose", "");
  assert.equal(overlay(host)["host/mute"], undefined, "back to its default, M");
  assert.deepEqual([window.props.modifiers, window.props.modifier], [[...MODS, "Cmd"], 4]);
  assert.equal((window.props.rows as MouseRow[])[7]!.bound, false);
  await emit(host, "mouse", "setModifier", 1);
  assert.deepEqual([window.props.modifiers, window.props.modifier], [MODS, 1]);
});

test("J01: on Windows the Windows key records nothing; Ctrl is Ctrl", async () => {
  assert.equal(fromSlintKey({ text: "x", control: false, alt: false, shift: false, meta: true }, false), null);
  assert.equal(fromSlintKey({ text: "\u{18}", control: false, alt: false, shift: false, meta: true }, false), null);
  assert.deepEqual(fromSlintKey({ text: "x", control: true, alt: false, shift: false, meta: false }, false), { input: "Ctrl+X" });
  const host = hostOn("windows");
  await start(host);
  await openKeys(host);
  await capture(host, "host/mute", "x", { meta: true });
  assert.equal(page(host, "keys").props.capturing, true, "still waiting");
  await emit(host, "keys", "key", "x", true, false, false, false);
  assert.equal(page(host, "keys").props.capturing, false);
  assert.deepEqual(overlay(host)["host/mute"], ["M", "Ctrl+X"]);
  // macOS: Slint's meta is ⌃, a Ctrl binding.
  const mac = hostOn("macos");
  await start(mac);
  await openKeys(mac);
  await capture(mac, "host/mute", "x", { meta: true });
  assert.deepEqual(overlay(mac)["host/mute"], ["M", "Ctrl+X"]);
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
  assert.equal(host.osd.at(-1), "Ignore default shortcuts: On");
  assert.equal(host.confirms.at(-1)!.message, "Ignore default shortcuts?");
  assert.deepEqual(overlay(host)["host/play-pause"], []);
  assert.deepEqual(overlay(host)["host/volume-up"], []);
  await assert.rejects(host.press("Space"), /nothing is bound to Space/, "the default is ignored");
  await assert.rejects(host.press("Mouse:WheelUp"), /nothing is bound to Mouse:WheelUp/, "the default is ignored");
  const record = JSON.parse(readFileSync(join(host.dataDirectory, "ignore-defaults.json"), "utf8"));
  assert.equal(record.enabled, true);
  assert.equal(record.written.length, 14); // the 14 core actions (hotkeys declares none)

  // J01 shows Ignored; a key added meanwhile is the user's.
  await openKeys(host);
  assert.equal(row(host, MUTE)!.state, IGNORED);
  assert.equal(row(host, MUTE)!.keys, "M"); // ui/keys.slint puts it in brackets
  assert.equal(page(host, "keys").props.ignore, true);
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
  assert.equal(host.osd.at(-1), "Ignore default shortcuts: Off");
  await host.press("M");
  assert.equal(host.coreRuns.at(-1), "mute");
});

test("OSD messages and dialogs in Simplified Chinese, as before", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await host.command("ignore-defaults").run();
  await host.settle(WAIT);
  assert.equal(host.confirmTexts.at(-1)!.message, "忽略默认快捷键？");
  assert.equal(host.confirmTexts.at(-1)!.confirm, "忽略");
  assert.equal(host.osd.at(-1), "忽略默认快捷键：开");
  await host.command("ignore-defaults").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "忽略默认快捷键：关");

  await openKeys(host);
  await capture(host, "host/play-pause", "k");
  assert.equal(host.osd.at(-1), "快捷键已更新：播放 ▸ 播放/暂停 = Space, K");
  await emit(host, "keys", "unbindAll");
  assert.equal(host.osd.at(-1), "快捷键已更新：播放 ▸ 播放/暂停 = （无）");
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "keys", "resetAll");
  assert.deepEqual(
    [host.confirmTexts.at(-1)!.message, host.confirmTexts.at(-1)!.confirm],
    ["恢复全部默认快捷键？", "恢复全部默认"],
  );
  assert.equal(host.osd.at(-1), "快捷键已恢复默认");
});

test("J01: the check box stays clear while the question is open; declined or failed, it stays clear", async () => {
  const host = new TestHost();
  await start(host);
  await openKeys(host);
  const keys = page(host, "keys");
  assert.equal(keys.props.ignore, false);
  // `self-toggle: false`: the check box only asks; `ignore` changes when the plugin says so.
  let answer!: (reply: { confirmed: boolean; checked: boolean }) => void;
  host.handle("dialogs.confirm", () => new Promise((resolve) => (answer = resolve)));
  keys.emit("toggleIgnore");
  for (let i = 0; i < 100 && !answer; i++) await new Promise((resolve) => setTimeout(resolve, 5));
  assert.ok(answer, "the question is open");
  assert.equal(keys.props.ignore, false, "not checked while the question is open");
  answer({ confirmed: false, checked: false });
  await host.settle(WAIT);
  assert.equal(host.command("ignore-defaults").state.checked, false);
  assert.equal(keys.props.ignore, false);
  // A failed question still pushes the state back (a stale `true` is overwritten).
  host.handle("dialogs.confirm", () => {
    throw new HostError("unavailable", "no dialog");
  });
  keys.props.ignore = true;
  await emit(host, "keys", "toggleIgnore");
  assert.equal(host.command("ignore-defaults").state.checked, false);
  assert.equal(keys.props.ignore, false, "sent back after the error");
  host.reset("dialogs.confirm");
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "keys", "toggleIgnore");
  assert.equal(host.command("ignore-defaults").state.checked, true);
  assert.equal(keys.props.ignore, true);
});

test("J01, J02: the drop-down and radio words arrive translated", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  await openKeys(host);
  const sources = page(host, "keys").props["source-model"] as string[];
  assert.deepEqual([sources[0], sources[1], sources[2], sources.at(-1)], ["全部来源", "宿主", "vestra.hotkeys", "未加载的插件"]);
  await host.command("mouse").run();
  await host.settle(WAIT);
  assert.equal((page(host, "mouse").props.modifiers as string[])[0], "无");
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
