import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { aboutText, components, PLUGINS_UNKNOWN, pluginRows } from "../about.ts";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);
const app = {
  version: "0.1.0",
  build: "289b26b",
  components: { mpv: "mpv v0.41.0", libav: { ffmpeg: "9.0.2" }, slint: "1.16.1" },
};
const tick = () => new Promise((done) => setTimeout(done, 5));
async function until(predicate: () => boolean) {
  for (let i = 0; i < 600; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("timed out waiting for condition");
}
const entry = (id: string, state: "running" | "disabled" | "failed" = "running") => ({
  id, version: "0.1.0", origin: "development" as const, state, error: null, capabilities: [], directory: `/plugins/${id}`, previousVersion: null,
});
async function start() {
  const host = new TestHost();
  host.app = app;
  host.packages = [entry("vestra.video"), entry("vestra.audio"), entry("vestra.info")];
  const plugin = await startPlugin(activate, { directory, host });
  return { host, stop: () => plugin.stop() };
}

test("I09: top-level 关于 Vestra Player… (880, windows, F1) opens the about window with plugin.app versions", async () => {
  const { host, stop } = await start();
  try {
    assert.deepEqual(host.command("info.about").declaration, {
      title: "关于 Vestra Player…", path: ["关于 Vestra Player…"], order: 880, group: "windows", radio: false, bindings: ["F1"], focus: null, repeat: false,
    });
    assert.deepEqual(host.conflicts, []);
    await host.press("F1");
    await host.settle();
    const window = host.window("about");
    assert.equal(window.file, "ui/about.slint");
    const slint = await readFile(new URL("../ui/about.slint", import.meta.url), "utf8");
    const declared = [...slint.slice(slint.indexOf("export component")).matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
    assert.deepEqual([...window.callbacks].sort(), declared.sort(), "exactly the Slint callbacks");
    assert.doesNotMatch(slint, /#[0-9a-f]{3,8}\b/i, "colours come from Theme");
    assert.doesNotMatch(slint, /font-size:\s*\d/, "font sizes come from Theme");
    assert.match(slint, /@image-url\("logo\.png"\)/);
    await readFile(new URL("../ui/logo.png", import.meta.url)); // packaged with the plugin
    assert.equal(window.props.version, "版本 0.1.0（构建 289b26b）");
    assert.deepEqual(window.props.components, [
      { label: "插件 API", value: "vestra/1" },
      { label: "mpv", value: "mpv v0.41.0" },
      { label: "FFmpeg", value: "9.0.2" },
      { label: "Slint", value: "1.16.1" },
      { label: "Node", value: `v${process.versions.node}` },
    ]);
    assert.deepEqual(window.props.plugins, [
      { id: "vestra.audio", version: "0.1.0", origin: "开发加载", state: "运行中" },
      { id: "vestra.info", version: "0.1.0", origin: "开发加载", state: "运行中" },
      { id: "vestra.video", version: "0.1.0", origin: "开发加载", state: "运行中" },
    ]);
    assert.equal(window.props["plugins-note"], "");
    // Another plugin stopped: the open window follows packages.onChange at once.
    host.setPackages([entry("vestra.video"), entry("vestra.audio", "disabled"), entry("vestra.info")]);
    await host.settle();
    assert.equal((window.props.plugins as { state: string }[])[0]!.state, "已停用");
    await host.press("F1");
    await host.settle();
    assert.equal(host.window("about").visible, true, "brought to the front, not closed");
    window.emit("close-window");
    await host.settle();
    assert.equal(host.windows.has("about"), false, "关闭 closes the window");
    await host.command("info.about").run();
    await host.settle();
    host.window("about").userClose();
    await host.settle();
    assert.equal(host.windows.has("about"), false);
  } finally {
    await stop();
  }
});

test("I09: 复制信息 copies the whole window as text; a refusal shows 复制失败", async () => {
  const { host, stop } = await start();
  try {
    await host.command("info.about").run();
    await host.settle();
    const window = host.window("about");
    window.emit("copy-info");
    await until(() => host.osd.at(-1) === "已复制版本信息");
    assert.equal(
      host.clipboard.text,
      [
        "Vestra Player",
        "版本 0.1.0（构建 289b26b）",
        "",
        "插件 API：vestra/1",
        "mpv：mpv v0.41.0",
        "FFmpeg：9.0.2",
        "Slint：1.16.1",
        `Node：v${process.versions.node}`,
        "",
        "已加载插件",
        "vestra.audio  0.1.0  开发加载  运行中",
        "vestra.info  0.1.0  开发加载  运行中",
        "vestra.video  0.1.0  开发加载  运行中",
        "",
      ].join("\n"),
    );
    host.handle("clipboard.writeText", () => {
      throw new HostError("failed", "refused");
    });
    window.emit("copy-info");
    await until(() => window.props.status === "复制失败");
  } finally {
    await stop();
  }
});

test("I09: a failed packages.list shows 未知 in the list; the next opening reads it again", async () => {
  const { host, stop } = await start();
  try {
    host.handle("packages.list", () => {
      throw new HostError("failed", "refused");
    });
    await host.command("info.about").run();
    await host.settle();
    assert.deepEqual(host.window("about").props.plugins, []);
    assert.equal(host.window("about").props["plugins-note"], PLUGINS_UNKNOWN);
    host.reset("packages.list");
    host.window("about").userClose();
    await host.command("info.about").run();
    await host.settle();
    assert.equal((host.window("about").props.plugins as unknown[]).length, 3);
    assert.equal(host.window("about").props["plugins-note"], "");
  } finally {
    await stop();
  }
});

test("I09 pure rules: unknown versions show 未知; plugin rows sorted by id with Chinese origin and state", () => {
  const bare = { version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "" } };
  assert.deepEqual(components(bare, null), [
    ["插件 API", "vestra/1"],
    ["mpv", "未知"],
    ["FFmpeg", "未知"],
    ["Slint", "未知"],
    ["Node", "未知"],
  ]);
  const rows = pluginRows([
    { id: "vestra.video", version: "0.1.0", origin: "installed", state: "running" },
    { id: "vestra.audio", version: null, origin: "development", state: "disabled" },
    { id: "test.trial", version: "1.0.0", origin: "trial", state: "failed" },
  ]);
  assert.deepEqual(rows, [
    { id: "test.trial", version: "1.0.0", origin: "试载", state: "失败" },
    { id: "vestra.audio", version: "未知", origin: "开发加载", state: "已停用" },
    { id: "vestra.video", version: "0.1.0", origin: "已安装", state: "运行中" },
  ]);
  assert.match(aboutText(bare, null, rows), /^Vestra Player\n版本 0\.2\.0\n\n[\s\S]*\n已加载插件\ntest\.trial {2}1\.0\.0 {2}试载 {2}失败\n/);
});
