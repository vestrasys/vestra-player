import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { aboutText, components, pluginRows } from "../about.ts";
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
async function start(locale?: "zh-Hans") {
  const host = new TestHost();
  host.app = app;
  host.packages = [entry("vestra.video"), entry("vestra.audio"), entry("vestra.info")];
  const plugin = await startPlugin(activate, { directory, host, ...(locale ? { locale } : {}) });
  return { host, stop: () => plugin.stop() };
}

test("I09: top-level About Vestra Player… (880, windows, F1) opens the about window with plugin.app versions", async () => {
  const { host, stop } = await start();
  try {
    assert.deepEqual(host.command("info.about").declaration, {
      title: "About Vestra Player…", path: ["About Vestra Player…"], order: 880, group: "windows", radio: false, bindings: ["F1"], focus: null, repeat: false,
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
    assert.equal(window.props.version, "0.1.0");
    assert.equal(window.props.build, "289b26b");
    assert.deepEqual(
      [window.props.api, window.props.mpv, window.props.ffmpeg, window.props.slint, window.props.node],
      ["vestra/1", "mpv v0.41.0", "9.0.2", "1.16.1", `v${process.versions.node}`],
    );
    // Origin and state are codes; about.slint words them.
    assert.deepEqual(window.props.plugins, [
      { id: "vestra.audio", version: "0.1.0", origin: "development", state: "running" },
      { id: "vestra.info", version: "0.1.0", origin: "development", state: "running" },
      { id: "vestra.video", version: "0.1.0", origin: "development", state: "running" },
    ]);
    assert.equal(window.props["plugins-unknown"], false);
    // Another plugin stopped: the open window follows packages.onChange at once.
    host.setPackages([entry("vestra.video"), entry("vestra.audio", "disabled"), entry("vestra.info")]);
    await host.settle();
    assert.equal((window.props.plugins as { state: string }[])[0]!.state, "disabled");
    await host.press("F1");
    await host.settle();
    assert.equal(host.window("about").visible, true, "brought to the front, not closed");
    window.emit("close-window");
    await host.settle();
    assert.equal(host.windows.has("about"), false, "Close closes the window");
    await host.command("info.about").run();
    await host.settle();
    host.window("about").userClose();
    await host.settle();
    assert.equal(host.windows.has("about"), false);
  } finally {
    await stop();
  }
});

test("I09: Copy Info copies the whole window as text; a refusal shows Copy failed", async () => {
  const { host, stop } = await start();
  try {
    await host.command("info.about").run();
    await host.settle();
    const window = host.window("about");
    window.emit("copy-info");
    await until(() => host.osd.at(-1) === "Version info copied");
    assert.equal(
      host.clipboard.text,
      [
        "Vestra Player",
        "Version 0.1.0 (build 289b26b)",
        "",
        "Plugin API: vestra/1",
        "mpv: mpv v0.41.0",
        "FFmpeg: 9.0.2",
        "Slint: 1.16.1",
        `Node: v${process.versions.node}`,
        "",
        "Loaded plugins",
        "vestra.audio  0.1.0  Development  Running",
        "vestra.info  0.1.0  Development  Running",
        "vestra.video  0.1.0  Development  Running",
        "",
      ].join("\n"),
    );
    host.handle("clipboard.writeText", () => {
      throw new HostError("failed", "refused");
    });
    window.emit("copy-info");
    await until(() => window.props.status === "copy-failed");
  } finally {
    await stop();
  }
});

test("I09: a failed packages.list shows Unknown in the list; the next opening reads it again", async () => {
  const { host, stop } = await start();
  try {
    host.handle("packages.list", () => {
      throw new HostError("failed", "refused");
    });
    await host.command("info.about").run();
    await host.settle();
    assert.deepEqual(host.window("about").props.plugins, []);
    assert.equal(host.window("about").props["plugins-unknown"], true);
    host.reset("packages.list");
    host.window("about").userClose();
    await host.command("info.about").run();
    await host.settle();
    assert.equal((host.window("about").props.plugins as unknown[]).length, 3);
    assert.equal(host.window("about").props["plugins-unknown"], false);
  } finally {
    await stop();
  }
});

test("I09 pure rules: unknown versions show Unknown; plugin rows sorted by id with origin and state codes", () => {
  const bare = { version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "" } };
  assert.deepEqual(components(bare, null), [
    ["Plugin API", "vestra/1"],
    ["mpv", "Unknown"],
    ["FFmpeg", "Unknown"],
    ["Slint", "Unknown"],
    ["Node", "Unknown"],
  ]);
  const rows = pluginRows([
    { id: "vestra.video", version: "0.1.0", origin: "installed", state: "running" },
    { id: "vestra.audio", version: null, origin: "development", state: "disabled" },
    { id: "test.failed", version: "1.0.0", origin: "installed", state: "failed" },
    { id: "x.repo", version: "1.0.0", origin: "installed", state: "running", builtin: false, source: { kind: "repository", url: "https://x/index.json" } },
    { id: "x.file", version: "1.0.0", origin: "installed", state: "running", builtin: false, source: { kind: "file" } },
    { id: "x.link", version: "1.0.0", origin: "installed", state: "running", builtin: false, source: { kind: "link", url: "https://x/a.vxp" } },
    { id: "x.workshop", version: "1.0.0", origin: "installed", state: "running", builtin: false, source: { kind: "workshop" } },
    { id: "vestra.info", version: "0.1.0", origin: "installed", state: "running", builtin: true, source: { kind: "bundled" } },
  ]);
  assert.deepEqual(rows, [
    { id: "test.failed", version: "1.0.0", origin: "installed", state: "failed" },
    { id: "vestra.audio", version: "", origin: "development", state: "disabled" },
    { id: "vestra.info", version: "0.1.0", origin: "bundled", state: "running" },
    { id: "vestra.video", version: "0.1.0", origin: "installed", state: "running" }, // an older player without sources
    { id: "x.file", version: "1.0.0", origin: "file", state: "running" },
    { id: "x.link", version: "1.0.0", origin: "link", state: "running" },
    { id: "x.repo", version: "1.0.0", origin: "repository", state: "running" },
    { id: "x.workshop", version: "1.0.0", origin: "workshop", state: "running" },
  ]);
  assert.match(aboutText(bare, null, rows), /^Vestra Player\nVersion 0\.2\.0\n\n[\s\S]*\nLoaded plugins\ntest\.failed {2}1\.0\.0 {2}Installed {2}Failed\nvestra\.audio {2}Unknown {2}/);
  assert.match(aboutText(bare, null, null), /\nLoaded plugins\nUnknown \(cannot read the plugin list\)\n$/);
});

test("I09 in Simplified Chinese: the window's words and the message of before", async () => {
  const { host, stop } = await start("zh-Hans");
  try {
    assert.deepEqual(host.command("info.about").display.path, ["关于 Vestra Player…"]);
    // What about.slint shows through @tr (the player rewrites the literals with the catalog).
    const words = ["About Vestra Player", "Loaded plugins", "Version", "Source", "State", "Copy Info", "Close", "Copy failed", "Unknown", "Plugin API"];
    assert.deepEqual(words.map((w) => host.translate(w)), ["关于 Vestra Player", "已加载插件", "版本", "来源", "状态", "复制信息", "关闭", "复制失败", "未知", "插件 API"]);
    assert.equal(host.translate("Version {} (build {})", ["0.1.0", "289b26b"]), "版本 0.1.0（构建 289b26b）");
    assert.equal(host.translate("Version {}", ["0.2.0"]), "版本 0.2.0");
    assert.equal(host.translate("Unknown (cannot read the plugin list)"), "未知（无法读取插件列表）");
    const origins = ["Built-in", "Repository", "Link", "Local file", "Workshop", "Installed", "Development"].map((w) => host.translate(w));
    assert.deepEqual(origins, ["内置", "仓库", "链接", "本地文件", "插件工坊", "已安装", "开发加载"]);
    const states = ["Starting", "Running", "Disabled", "Failed", "Stopped"].map((w) => host.translate(w));
    assert.deepEqual(states, ["启动中", "运行中", "已停用", "失败", "已停止"]);
    // Every code pluginRows produces is worded in about.slint.
    const slint = await readFile(new URL("../ui/about.slint", import.meta.url), "utf8");
    for (const code of ["bundled", "repository", "link", "file", "workshop", "installed", "development", "starting", "running", "disabled", "failed", "stopped"]) {
      assert.match(slint, new RegExp(`code == "${code}" \\? @tr\\(`), code);
    }
    await host.command("info.about").run();
    await host.settle();
    host.window("about").emit("copy-info");
    await until(() => host.osd.at(-1) === "已复制版本信息");
  } finally {
    await stop();
  }
});
