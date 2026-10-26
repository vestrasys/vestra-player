import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { bytes, failure, sourceCode, sourceUrl } from "../labels.ts";
import activate from "../main.ts";
import { directorySize, timing } from "../sizes.ts";

Object.assign(timing, { sizeDebounce: 5 });
const WAIT = 40;

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-plugins-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

type Info = TestHost["packages"][number];
const info = (id: string, patch: Partial<Info> = {}): Info => ({
  id,
  version: "0.1.0",
  origin: "installed",
  state: "running",
  error: null,
  capabilities: ["commands"],
  directory: `/packages/${id}`,
  ...patch,
});
const REPOSITORY = "https://plugins.example.org/index.json";
/**
 * Data directories are per install instance and not named after the id
 * (the player's layout is not part of the API): the plugin must use
 * `info.dataDirectory`, never derive it.
 */
const LIST = (root: string) => [
  info("vestra.audio", {
    capabilities: ["commands", "processing.audio", "playback.read"],
    source: { kind: "repository", url: REPOSITORY },
    dataDirectory: join(root, "instance-7f3a"),
  }),
  info("vestra.plugins", { capabilities: ["commands", "plugins.manage"], builtin: true, source: { kind: "bundled" }, dataDirectory: join(root, "self") }),
  info("dev.thing", { origin: "development", version: null, dataDirectory: join(root, "development", "dev.thing") }),
  info("broken.one", { state: "failed", error: "activate threw: boom", source: { kind: "file" }, dataDirectory: join(root, "instance-01bc") }),
];

const running = new Set<() => Promise<void>>();
/** The plugin with its data folder in a fresh data root (so sibling folders can be measured). */
async function start(host = new TestHost(), extra: (root: string) => Info[] = () => [], locale?: "en" | "zh-Hans") {
  const root = mkdtempSync(join(scratch, "root-"));
  host.packages = [...LIST(root), ...extra(root)];
  const started = await startPlugin(activate, { directory, host, dataDirectory: join(root, "self"), ...(locale ? { locale } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  await host.command("manage").run();
  await host.settle(WAIT);
  return { root, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

type Row = { id: string; version: string; origin: string; state: string; size: string; dim: boolean; failed: boolean; selected: boolean };
const props = (host: TestHost) => host.settingsPage("plugins.manage").props;
const rows = (host: TestHost) => props(host).rows as Row[];
const row = (host: TestHost, id: string) => rows(host).find((r) => r.id === id);
type Detail = { label: string; kind: string; value: string; url: string; warn: boolean };
const details = (host: TestHost) => props(host).details as Detail[];
const message = (host: TestHost) => props(host).message as { kind: string; id: string; code: string; text: string; warn: boolean };
async function emit(host: TestHost, callback: string, ...args: string[]) {
  host.settingsPage("plugins.manage").emit(callback, ...args);
  await host.settle(WAIT);
}

test("helpers: sizes and byte text", async () => {
  const dir = mkdtempSync(join(scratch, "size-"));
  mkdirSync(join(dir, "sub"));
  writeFileSync(join(dir, "a"), Buffer.alloc(1000));
  writeFileSync(join(dir, "sub", "b"), Buffer.alloc(2000));
  assert.equal(await directorySize(dir), 3000);
  assert.equal(await directorySize(join(dir, "missing")), null);
  assert.equal(await directorySize(dir, -1), null); // past the time limit
  assert.equal(bytes(null), "—");
  assert.equal(bytes(512), "512 B");
  assert.equal(bytes(3000), "2.9 KB");
  assert.equal(bytes(25 * 1024 * 1024), "25 MB");
});

test("menu: Settings ▸ Plugin Manager…", async () => {
  const host = new TestHost();
  await start(host, undefined, "en");
  const declaration = host.command("manage").declaration;
  assert.deepEqual(declaration.path, ["@settings", "Plugin Manager…"]);
  assert.deepEqual(host.command("manage").display.path, ["Settings", "Plugin Manager…"]);
  assert.equal(declaration.order, 836);
  assert.equal(declaration.group, "settings-plugins");
  assert.deepEqual(declaration.bindings, []);
  const registered = host.settingsPage("plugins.manage");
  assert.deepEqual([registered.category, registered.title], ["plugins", "Plugin Manager"]);
  assert.deepEqual(host.settings.shows, ["plugins.manage"]);
});

test("menu in Simplified Chinese: 设置 ▶ 插件管理…", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  assert.deepEqual(host.command("manage").display.path, ["设置", "插件管理…"]);
  assert.equal(host.translate("Plugin Manager"), "插件管理");
});

test("J07: the list shows id, version, origin, state and data size", async () => {
  const host = new TestHost();
  const { root } = await start(host);
  assert.deepEqual(
    rows(host).map((r) => [r.id, r.version, r.origin, r.state]),
    [
      ["broken.one", "0.1.0", "file", "failed"],
      ["dev.thing", "", "development", "running"],
      ["vestra.audio", "0.1.0", "repository", "running"],
      ["vestra.plugins", "0.1.0", "bundled", "running"],
    ],
  );
  assert.equal(row(host, "broken.one")!.failed, true);
  // Sizes: measured when the window opens (a missing folder is 「—」).
  mkdirSync(join(root, "instance-7f3a"));
  writeFileSync(join(root, "instance-7f3a", "settings.json"), Buffer.alloc(2048));
  await host.command("manage").run();
  await host.settle(WAIT);
  assert.equal(row(host, "vestra.audio")!.size, "2.0 KB");
  assert.equal(row(host, "dev.thing")!.size, "—");
  assert.equal(row(host, "vestra.plugins")!.size, "0 B");
});

test("J07: details of the selected plugin", async () => {
  const host = new TestHost();
  const { root } = await start(host);
  await emit(host, "select", "vestra.audio");
  assert.deepEqual(
    details(host).map((d) => [d.label, d.kind, d.value, d.url]),
    [
      ["source", "source", "repository", REPOSITORY],
      ["package", "", "/packages/vestra.audio", ""],
      ["data", "", join(root, "instance-7f3a"), ""],
      ["capabilities", "capability", "commands", ""],
      ["", "capability", "processing.audio", ""],
      ["", "capability", "playback.read", ""],
    ],
  );
  await emit(host, "select", "broken.one");
  assert.deepEqual(details(host)[0], { label: "failure", kind: "", value: "activate threw: boom", url: "", warn: true });
  assert.deepEqual(details(host)[1], { label: "source", kind: "source", value: "file", url: "", warn: false });
  assert.equal(props(host)["can-reload"], true);
});

test("J07: a running plugin's warning (a refused panel position) shows as Warning", async () => {
  const host = new TestHost();
  const warning = "面板未显示：侧栏已被 vestra.playlist 使用";
  await start(host, () => [info("dev.panel", { origin: "development", version: null, error: warning })]);
  assert.deepEqual(row(host, "dev.panel"), {
    id: "dev.panel",
    version: "",
    origin: "development",
    state: "running",
    size: "—",
    dim: false,
    failed: true,
    selected: false,
  });
  assert.equal(row(host, "vestra.audio")!.failed, false);
  await emit(host, "select", "dev.panel");
  assert.deepEqual(details(host)[0], { label: "warning", kind: "", value: warning, url: "", warn: true });
  assert.ok(!details(host).some((d) => d.label === "failure"));
  // Still a running plugin: it can be disabled and reloaded.
  assert.equal(props(host)["toggle-enables"], false);
  assert.equal(props(host)["can-reload"], true);
});

test("J07: disable asks first, enable does not; the list follows the player", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  assert.equal(props(host)["toggle-enables"], false);
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls, []);
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.deepEqual(host.confirms.at(-1)!.args, ["vestra.audio"]);
  assert.equal(host.confirmTexts.at(-1)!.message, "Disable vestra.audio?");
  assert.match(String(host.confirmTexts.at(-1)!.detail), /It stays disabled after Vestra Player restarts/);
  assert.deepEqual(host.packageCalls, [{ method: "setEnabled", id: "vestra.audio", enabled: false }]);
  assert.equal(row(host, "vestra.audio")!.state, "disabled");
  assert.equal(row(host, "vestra.audio")!.dim, true);
  assert.equal(props(host)["toggle-enables"], true);
  assert.equal(props(host)["can-reload"], false);
  assert.equal(host.osd.at(-1), "Disabled vestra.audio");
  assert.deepEqual(message(host), { kind: "disabled", id: "vestra.audio", code: "", text: "", warn: false });
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls.at(-1), { method: "setEnabled", id: "vestra.audio", enabled: true });
  assert.equal(row(host, "vestra.audio")!.state, "running");
  assert.equal(host.osd.at(-1), "Enabled vestra.audio");

  // Reload = stop, then start.
  await emit(host, "reload");
  assert.deepEqual(host.packageCalls.slice(-2), [
    { method: "setEnabled", id: "vestra.audio", enabled: false },
    { method: "setEnabled", id: "vestra.audio", enabled: true },
  ]);
  assert.equal(host.osd.at(-1), "Reloaded vestra.audio");

  // Another plugin installed (workshop): it shows up; a removed one goes with the selection.
  host.setPackages([...host.packages.filter((p) => p.id !== "vestra.audio"), info("community.counter")]);
  await host.settle(WAIT);
  assert.ok(row(host, "community.counter"));
  assert.equal(row(host, "vestra.audio"), undefined);
  assert.equal(props(host)["has-selection"], false);
});

test("J07: its own row cannot be disabled or uninstalled", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.plugins");
  assert.equal(props(host)["can-toggle"], false);
  assert.equal(props(host)["can-uninstall"], false);
  assert.equal(props(host)["can-reload"], false);
  assert.equal(props(host)["toggle-tip"], "self");
  assert.equal(props(host)["uninstall-tip"], "self");
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls, []);
  assert.equal(props(host)["can-open"], true);
});

test("J07: development loads", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "dev.thing");
  assert.equal(props(host)["can-toggle"], true);
  assert.equal(props(host)["can-uninstall"], false);
  assert.equal(props(host)["uninstall-tip"], "not-installed");
  assert.ok(details(host).some((d) => d.label === "note" && d.kind === "development"));
  assert.deepEqual(details(host).find((d) => d.label === "source"), { label: "source", kind: "source", value: "development", url: "", warn: false });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.match(String(host.confirmTexts.at(-1)!.detail), /stays disabled only for this run/);
});

test("J07: uninstall asks, can remove the data, and reports", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  host.dialogAnswers.push({ confirmed: true, checked: true });
  await emit(host, "uninstall");
  const asked = host.confirms.at(-1)!;
  assert.equal(asked.destructive, true);
  assert.equal(asked.checkbox, "Also delete its settings and data?");
  assert.equal(host.confirmTexts.at(-1)!.message, "Uninstall vestra.audio?");
  assert.deepEqual(host.packageCalls, [{ method: "uninstall", id: "vestra.audio", removeData: true }]);
  assert.equal(row(host, "vestra.audio"), undefined);
  assert.equal(host.osd.at(-1), "Uninstalled vestra.audio");
  // Cancelled: nothing happens.
  await emit(host, "select", "broken.one");
  host.dialogAnswers.push({ confirmed: false, checked: true });
  await emit(host, "uninstall");
  assert.equal(host.packageCalls.length, 1);
});

test("J07: no Roll Back; failures are shown", async () => {
  const host = new TestHost();
  await start(host, (root) => [info("zysilm.youtube", { source: { kind: "file" }, dataDirectory: join(root, "instance-yt") })]);
  await emit(host, "select", "zysilm.youtube");
  assert.ok(!("can-rollback" in props(host)), "no Roll Back button");
  assert.equal(props(host)["can-uninstall"], true);
  assert.ok(!host.sent("packages.rollback").length);
  host.handle("packages.setEnabled", () => {
    throw new HostError("busy", "installing");
  });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.deepEqual(message(host), { kind: "failure", id: "", code: "busy", text: "", warn: true });
});

test("J07: open the data folder; a missing one is reported", async () => {
  const host = new TestHost();
  const { root } = await start(host);
  await emit(host, "select", "vestra.plugins");
  await emit(host, "openData");
  assert.deepEqual(host.system.opened, [join(root, "self")]);
  await emit(host, "select", "vestra.audio");
  await emit(host, "openData");
  assert.deepEqual(message(host), { kind: "no-data-folder", id: "", code: "", text: "", warn: true });
});

test("source: each kind as a code, and development loads by their origin", () => {
  const base = { ...info("x"), builtin: false, source: null, dataDirectory: null };
  assert.equal(sourceUrl({ source: { kind: "repository", url: REPOSITORY } }), REPOSITORY);
  assert.equal(sourceUrl({ source: { kind: "link", url: "https://example.org/x-1.0.0.vxp" } }), "https://example.org/x-1.0.0.vxp");
  assert.equal(sourceUrl({ source: { kind: "file" } }), "");
  assert.equal(sourceUrl({ source: null }), "");
  // The list's narrow column: the source kind without the address; bundled for built-ins.
  const short = (patch: Partial<Info>) => sourceCode({ ...base, ...patch });
  assert.deepEqual(
    [
      short({ source: { kind: "repository", url: REPOSITORY } }),
      short({ source: { kind: "link", url: "https://example.org/x-1.0.0.vxp" } }),
      short({ source: { kind: "file" } }),
      short({ source: { kind: "workshop" } }),
      short({ source: { kind: "bundled" }, builtin: true }),
      short({ builtin: true }),
      short({ origin: "development" }),
      short({}), // an older player without sources
    ],
    ["repository", "link", "file", "workshop", "bundled", "bundled", "development", "installed"],
  );
});

test("Simplified Chinese: the page's words are the old ones", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  const zh = (text: string, args?: string[]) => host.translate(text, args);
  // List columns (codes from the rows) and states.
  assert.deepEqual(
    ["Plugin", "Version", "Source", "Status", "Data", "Dev"].map((t) => zh(t)),
    ["插件", "版本", "来源", "状态", "数据", "开发"],
  );
  assert.deepEqual(
    ["Running", "Starting", "Disabled", "plugin state\u0004Failed", "Stopped"].map((t) => zh(t)),
    ["运行中", "启动中", "已停用", "启动失败", "已停止"],
  );
  assert.deepEqual(
    ["Repository", "Link", "Local file", "Plugin Workshop", "Built-in", "Installed", "Development"].map((t) => zh(t)),
    ["仓库", "链接", "本地文件", "插件工坊", "内置", "已安装", "开发加载"],
  );
  // Details.
  assert.equal(zh("Repository ({})", [REPOSITORY]), `仓库（${REPOSITORY}）`);
  assert.equal(zh("Link ({})", ["https://example.org/x-1.0.0.vxp"]), "链接（https://example.org/x-1.0.0.vxp）");
  assert.equal(zh("{0}  {1}", ["commands", zh("Menu commands and shortcuts")]), "commands  菜单命令与快捷键");
  assert.deepEqual(
    ["Error", "Warning", "Note", "Package", "Data folder", "Capabilities", "Unavailable", "(none)"].map((t) => zh(t)),
    ["失败原因", "提示", "说明", "包目录", "数据目录", "能力", "不可用", "（无）"],
  );
  assert.equal(zh("Built-in plugin: installed and updated with the player; cannot be uninstalled or disabled"), "内置插件：随播放器安装和更新，不能卸载，也不能停用");
  assert.equal(zh("Development load: disabling lasts only for this run; cannot be uninstalled"), "开发加载：停用只在本次运行有效；不能卸载");
  // Buttons and tips.
  assert.deepEqual(
    ["Enable", "Disable", "Reload", "Uninstall…", "Open Data Folder"].map((t) => zh(t)),
    ["启用", "停用", "重新加载", "卸载…", "打开数据目录"],
  );
  assert.equal(zh("Plugin Manager cannot disable itself"), "插件管理不能停用自身");
  assert.equal(zh("Built-in plugins come with the player and cannot be uninstalled or disabled"), "内置插件随播放器提供，不能卸载，也不能停用");
  assert.equal(zh("Only installed plugins can be uninstalled"), "只能卸载已安装的插件");
  // Messages.
  assert.equal(zh("Uninstall {0}: {1} ({2})", ["vestra.audio", zh("change status\u0004Failed"), "busy"]), "卸载 vestra.audio：失败（busy）");
  assert.equal(zh("Plugins are being installed or cleaned up, please try again later"), "正在安装或清理插件，请稍后再试");
  assert.equal(zh("Development-loaded plugins cannot be uninstalled"), "开发加载的插件不能卸载");

  // OSD and dialogs as a Chinese user sees them.
  await emit(host, "select", "vestra.audio");
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.equal(host.confirmTexts.at(-1)!.message, "停用 vestra.audio？");
  assert.match(String(host.confirmTexts.at(-1)!.detail), /重启 Vestra Player 后仍保持停用/);
  assert.equal(host.confirmTexts.at(-1)!.confirm, "停用");
  assert.equal(host.osd.at(-1), "已停用 vestra.audio");
  await emit(host, "toggle");
  assert.equal(host.osd.at(-1), "已启用 vestra.audio");
  await emit(host, "reload");
  assert.equal(host.osd.at(-1), "已重新加载 vestra.audio");
  host.dialogAnswers.push({ confirmed: true, checked: true });
  await emit(host, "uninstall");
  assert.deepEqual(host.confirmTexts.at(-1), {
    message: "卸载 vestra.audio？",
    detail: "将从已安装的插件中移除它。它的快捷键修改会保留，重新安装后恢复。",
    confirm: "卸载",
    cancel: host.confirmTexts.at(-1)!.cancel,
    checkbox: "同时删除它的设置与数据？",
  });
  assert.equal(host.osd.at(-1), "已卸载 vestra.audio");
});

test("built-in plugins: Disable and Uninstall… are off with a hint, no Roll Back; a disabled one can be enabled", async () => {
  const host = new TestHost();
  await start(host, (root) => [
    info("vestra.subtitles", {
      builtin: true,
      source: { kind: "bundled" },
      directory: "/Applications/Vestra Player.app/Contents/Resources/plugins/vestra.subtitles",
      dataDirectory: join(root, "instance-sub"),
    }),
  ]);
  assert.equal(row(host, "vestra.subtitles")!.origin, "bundled");
  await emit(host, "select", "vestra.subtitles");
  assert.equal(props(host)["can-uninstall"], false);
  assert.equal(props(host)["uninstall-tip"], "builtin");
  assert.equal(props(host)["toggle-enables"], false);
  assert.equal(props(host)["can-toggle"], false);
  assert.equal(props(host)["toggle-tip"], "builtin");
  assert.equal(props(host)["can-reload"], true);
  assert.ok(!("can-rollback" in props(host)));
  assert.equal(props(host)["can-open"], true);
  // The package folder is the installation's (built-ins run in place).
  assert.ok(details(host).some((d) => d.label === "package" && d.value.endsWith("Resources/plugins/vestra.subtitles")));
  assert.deepEqual(
    details(host)
      .slice(0, 2)
      .map((d) => [d.label, d.kind, d.value]),
    [
      ["note", "builtin", ""],
      ["source", "source", "bundled"],
    ],
  );
  // The callback (if it fired anyway) neither asks nor calls the player.
  const asked = host.confirms.length;
  await emit(host, "uninstall");
  assert.equal(host.confirms.length, asked);
  assert.deepEqual(host.packageCalls, []);
  // Nor does Disable.
  await emit(host, "toggle");
  assert.equal(host.confirms.length, asked);
  assert.deepEqual(host.packageCalls, []);
  // One left disabled (before built-ins could not be) can be enabled again.
  host.packages.find((p) => p.id === "vestra.subtitles")!.state = "disabled";
  host.setPackages(host.packages);
  await host.settle(WAIT);
  assert.equal(props(host)["toggle-enables"], true);
  assert.equal(props(host)["can-toggle"], true);
  assert.equal(props(host)["toggle-tip"], "");
  await emit(host, "toggle");
  assert.equal(host.confirms.length, asked);
  assert.deepEqual(host.packageCalls, [{ method: "setEnabled", id: "vestra.subtitles", enabled: true }]);
  // Its own row keeps its own hint, built in or not.
  await emit(host, "select", "vestra.plugins");
  assert.equal(props(host)["uninstall-tip"], "self");
});

test("uninstall refused as unsupported (the list was out of date): reported and re-read", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  assert.equal(props(host)["can-uninstall"], true);
  // The player now treats it as built in; no list change was sent yet.
  Object.assign(host.packages.find((p) => p.id === "vestra.audio")!, { builtin: true, source: { kind: "bundled" } });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "uninstall");
  assert.deepEqual(host.packageCalls, []);
  assert.deepEqual(message(host), { kind: "failure", id: "", code: "builtin", text: "", warn: true });
  assert.equal(props(host)["can-uninstall"], false);
  assert.equal(props(host)["uninstall-tip"], "builtin");
  assert.ok(row(host, "vestra.audio"));
  // The other unsupported cases keep their own text.
  assert.deepEqual(failure({ code: "unsupported" }, { ...info("d", { origin: "development" }), builtin: false, source: null, dataDirectory: null }), { code: "development", text: "" });
  assert.deepEqual(failure({ code: "invalid" }), { code: "self", text: "" });
  assert.deepEqual(failure(new Error("disk full")), { code: "error", text: "disk full" });
});

test("data directory: from the list; none from the player; a new one is measured", async () => {
  const host = new TestHost();
  const { root } = await start(host, () => [info("no.data", { dataDirectory: null })]);
  await emit(host, "select", "no.data");
  assert.equal(row(host, "no.data")!.size, "—");
  assert.equal(props(host)["can-open"], false);
  assert.deepEqual(details(host).find((d) => d.label === "data"), { label: "data", kind: "unavailable", value: "", url: "", warn: false });
  await emit(host, "openData");
  assert.deepEqual(host.system.opened, []);
  // Replaced from another source: a new, separately measured data directory.
  const replaced = join(root, "instance-new");
  mkdirSync(replaced);
  writeFileSync(join(replaced, "a"), Buffer.alloc(4096));
  host.setPackages(host.packages.map((p) => (p.id === "vestra.audio" ? { ...p, source: { kind: "file" as const }, dataDirectory: replaced } : p)));
  await host.settle(WAIT);
  assert.equal(row(host, "vestra.audio")!.size, "4.0 KB");
  await emit(host, "select", "vestra.audio");
  await emit(host, "openData");
  assert.deepEqual(host.system.opened, [replaced]);
});
