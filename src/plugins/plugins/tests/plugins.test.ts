import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { bytes } from "../labels.ts";
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
  previousVersion: null,
  ...patch,
});
const LIST = () => [
  info("vestra.audio", { previousVersion: "0.0.9", capabilities: ["commands", "processing.audio", "playback.read"] }),
  info("vestra.plugins", { capabilities: ["commands", "plugins.manage"] }),
  info("dev.thing", { origin: "development", version: null }),
  info("trial.x", { origin: "trial" }),
  info("broken.one", { state: "failed", error: "activate threw: boom" }),
];

const running = new Set<() => Promise<void>>();
/** The plugin with its data folder in a fresh data root (so sibling folders can be measured). */
async function start(host = new TestHost()) {
  const root = mkdtempSync(join(scratch, "root-"));
  host.packages = LIST();
  const started = await startPlugin(activate, { directory, host, dataDirectory: join(root, "vestra.plugins") });
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

test("menu: 设置 ▶ 插件管理…", async () => {
  const host = new TestHost();
  await start(host);
  const declaration = host.command("manage").declaration;
  assert.deepEqual(declaration.path, ["设置", "插件管理…"]);
  assert.equal(declaration.order, 836);
  assert.equal(declaration.group, "settings-plugins");
  assert.deepEqual(declaration.bindings, []);
  const registered = host.settingsPage("plugins.manage");
  assert.deepEqual([registered.category, registered.title], ["plugins", "插件管理"]);
  assert.deepEqual(host.settings.shows, ["plugins.manage"]);
});

test("J07: the list shows id, version, origin, state and data size", async () => {
  const host = new TestHost();
  const { root } = await start(host);
  assert.deepEqual(
    rows(host).map((r) => [r.id, r.version, r.origin, r.state]),
    [
      ["broken.one", "0.1.0", "已安装", "启动失败"],
      ["dev.thing", "开发", "开发加载", "运行中"],
      ["trial.x", "0.1.0", "试载", "运行中"],
      ["vestra.audio", "0.1.0", "已安装", "运行中"],
      ["vestra.plugins", "0.1.0", "已安装", "运行中"],
    ],
  );
  assert.equal(row(host, "broken.one")!.failed, true);
  // Sizes: measured when the window opens (a missing folder is 「—」).
  mkdirSync(join(root, "vestra.audio"));
  writeFileSync(join(root, "vestra.audio", "settings.json"), Buffer.alloc(2048));
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
  const details = props(host).details as { label: string; text: string; warn: boolean }[];
  assert.deepEqual(
    details.map((d) => [d.label, d.text]),
    [
      ["上一版本", "0.0.9"],
      ["包目录", "/packages/vestra.audio"],
      ["数据目录", join(root, "vestra.audio")],
      ["能力", "commands  菜单命令与快捷键"],
      ["", "processing.audio  处理声音"],
      ["", "playback.read  读取播放状态"],
    ],
  );
  await emit(host, "select", "broken.one");
  const failed = (props(host).details as { label: string; text: string; warn: boolean }[])[0]!;
  assert.deepEqual(failed, { label: "失败原因", text: "activate threw: boom", warn: true });
  assert.equal(props(host)["can-reload"], true);
});

test("J07: disable asks first, enable does not; the list follows the player", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  assert.equal(props(host)["toggle-text"], "停用");
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls, []);
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.match(String(host.confirms.at(-1)!.detail), /重启 Vestra Player 后仍保持停用/);
  assert.deepEqual(host.packageCalls, [{ method: "setEnabled", id: "vestra.audio", enabled: false }]);
  assert.equal(row(host, "vestra.audio")!.state, "已停用");
  assert.equal(row(host, "vestra.audio")!.dim, true);
  assert.equal(props(host)["toggle-text"], "启用");
  assert.equal(props(host)["can-reload"], false);
  assert.equal(host.osd.at(-1), "已停用 vestra.audio");
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls.at(-1), { method: "setEnabled", id: "vestra.audio", enabled: true });
  assert.equal(row(host, "vestra.audio")!.state, "运行中");
  assert.equal(host.osd.at(-1), "已启用 vestra.audio");

  // 重新加载 = stop, then start.
  await emit(host, "reload");
  assert.deepEqual(host.packageCalls.slice(-2), [
    { method: "setEnabled", id: "vestra.audio", enabled: false },
    { method: "setEnabled", id: "vestra.audio", enabled: true },
  ]);

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
  assert.equal(props(host)["toggle-tip"], "插件管理不能停用自身");
  assert.equal(props(host)["uninstall-tip"], "插件管理不能停用自身");
  await emit(host, "toggle");
  assert.deepEqual(host.packageCalls, []);
  assert.equal(props(host)["can-open"], true);
});

test("J07: development loads and trials", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "dev.thing");
  assert.equal(props(host)["can-toggle"], true);
  assert.equal(props(host)["can-uninstall"], false);
  assert.equal(props(host)["can-rollback"], false);
  assert.equal(props(host)["uninstall-tip"], "只能卸载已安装的插件");
  assert.ok((props(host).details as { text: string }[]).some((d) => d.text.startsWith("开发加载：停用只在本次运行有效")));
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.match(String(host.confirms.at(-1)!.detail), /只在本次运行内停用/);
  await emit(host, "select", "trial.x");
  for (const name of ["can-toggle", "can-reload", "can-rollback", "can-uninstall", "can-open"])
    assert.equal(props(host)[name], false, name);
});

test("J07: uninstall asks, can remove the data, and reports", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  host.dialogAnswers.push({ confirmed: true, checked: true });
  await emit(host, "uninstall");
  const asked = host.confirms.at(-1)!;
  assert.equal(asked.destructive, true);
  assert.equal(asked.checkbox, "同时删除它的设置与数据");
  assert.deepEqual(host.packageCalls, [{ method: "uninstall", id: "vestra.audio", removeData: true }]);
  assert.equal(row(host, "vestra.audio"), undefined);
  assert.equal(host.osd.at(-1), "已卸载 vestra.audio");
  // Cancelled: nothing happens.
  await emit(host, "select", "broken.one");
  host.dialogAnswers.push({ confirmed: false, checked: true });
  await emit(host, "uninstall");
  assert.equal(host.packageCalls.length, 1);
});

test("J07: rollback follows the change; failures are shown", async () => {
  const host = new TestHost();
  await start(host);
  await emit(host, "select", "vestra.audio");
  assert.equal(props(host)["can-rollback"], true);
  host.handle("packages.rollback", (p) => ({ operationId: String(p.requestId), status: "committed" }));
  await emit(host, "rollback");
  assert.equal(props(host).message, "已回退 vestra.audio 到 0.0.9");
  host.handle("packages.rollback", (p) => ({ operationId: String(p.requestId), status: "failed", message: "manifest invalid" }));
  await emit(host, "rollback");
  assert.equal(props(host).message, "回退 vestra.audio：失败（manifest invalid）");
  assert.equal(props(host)["message-warn"], true);
  host.handle("packages.setEnabled", () => {
    throw new HostError("busy", "installing");
  });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await emit(host, "toggle");
  assert.equal(props(host).message, "正在安装或回退，请稍后再试");
});

test("J07: open the data folder; a missing one is reported", async () => {
  const host = new TestHost();
  const { root } = await start(host);
  await emit(host, "select", "vestra.plugins");
  await emit(host, "openData");
  assert.deepEqual(host.system.opened, [join(root, "vestra.plugins")]);
  await emit(host, "select", "vestra.audio");
  await emit(host, "openData");
  assert.match(String(props(host).message), /数据目录不存在/);
});
