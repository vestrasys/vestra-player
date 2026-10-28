import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { aboutText, components, NO_RENDERING, pluginRows, renderingRow, renderingValue } from "../about.ts";
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
  id, version: "0.1.0", origin: "development" as const, state, error: null, capabilities: [], directory: `/plugins/${id}`,
});
async function start(locale?: "zh-Hans") {
  const host = new TestHost();
  host.app = app;
  host.packages = [entry("vestra.video"), entry("vestra.audio"), entry("vestra.info")];
  const plugin = await startPlugin(activate, { directory, host, ...(locale ? { locale } : {}) });
  return { host, i18n: plugin.plugin.i18n, stop: () => plugin.stop() };
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
        "Rendering: GPU",
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

test("I09 pure rules: unknown versions show Unknown; plugin rows sorted by id with origin and state codes", async () => {
  const { i18n, stop } = await start("zh-Hans");
  const en = await start();
  const bare = { version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "" } };
  const line = (words: typeof i18n) => ([label, value]: [unknown, unknown]) => words.resolve(words.text("{}: {}", [label, value] as never));
  assert.deepEqual(components(i18n, bare, null).map(line(i18n)), ["插件 API：vestra/1", "mpv：未知", "FFmpeg：未知", "Slint：未知", "Node：未知"]);
  assert.deepEqual(components(en.i18n, bare, null).map(line(en.i18n)), ["Plugin API: vestra/1", "mpv: Unknown", "FFmpeg: Unknown", "Slint: Unknown", "Node: Unknown"]);
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
  assert.match(aboutText(i18n, bare, null, rows), /^Vestra Player\n版本 0\.2\.0\n\n[\s\S]*\n已加载插件\ntest\.failed {2}1\.0\.0 {2}已安装 {2}失败\nvestra\.audio {2}未知 {2}开发加载 {2}已停用\nvestra\.info {2}0\.1\.0 {2}内置 {2}运行中\n/);
  assert.match(aboutText(i18n, bare, null, rows), /\nx\.file {2}1\.0\.0 {2}本地文件 {2}运行中\nx\.link {2}1\.0\.0 {2}链接 {2}运行中\nx\.repo {2}1\.0\.0 {2}仓库 {2}运行中\nx\.workshop {2}1\.0\.0 {2}插件工坊 {2}运行中\n$/);
  assert.match(aboutText(i18n, bare, null, null), /\n已加载插件\n未知（无法读取插件列表）\n$/);
  assert.match(aboutText(en.i18n, bare, null, rows), /^Vestra Player\nVersion 0\.2\.0\n\n[\s\S]*\nLoaded plugins\ntest\.failed {2}1\.0\.0 {2}Installed {2}Failed\nvestra\.audio {2}Unknown {2}/);
  assert.match(aboutText(en.i18n, bare, null, null), /\nLoaded plugins\nUnknown \(cannot read the plugin list\)\n$/);
  await stop();
  await en.stop();
});

test("I09 in Simplified Chinese: the window's words and the message of before", async () => {
  const { host, stop } = await start("zh-Hans");
  try {
    assert.deepEqual(host.command("info.about").display.path, ["关于 Vestra Player…"]);
    // What about.slint shows through @tr (the player rewrites the literals with the catalog).
    const words = ["About Vestra Player", "Loaded plugins", "Version", "Source", "State", "Copy Info", "Close", "Copy failed", "Unknown", "Plugin API", "Rendering", "GPU", "Software"];
    assert.deepEqual(words.map((w) => host.translate(w)), ["关于 Vestra Player", "已加载插件", "版本", "来源", "状态", "复制信息", "关闭", "复制失败", "未知", "插件 API", "渲染", "GPU", "软件"]);
    assert.equal(host.translate("{} — {}", ["软件", "reason"]), "软件——reason");
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
        "渲染：GPU",
        "",
        "已加载插件",
        "vestra.audio  0.1.0  开发加载  运行中",
        "vestra.info  0.1.0  开发加载  运行中",
        "vestra.video  0.1.0  开发加载  运行中",
        "",
      ].join("\n"),
    );
  } finally {
    await stop();
  }
});

/** Reasons in the shapes player 0.5.13 sends (`Reason::message`), and a code a newer player may add. */
const DENYLIST =
  "the graphics driver is on the player's denylist: AMD Radeon 780M driver 31.0.14005: GPU draws stall for tens of milliseconds (AMD Radeon 780M, driver 31.0.14005.21001; rule amd-780m-draw-stalls); update the graphics driver, or on a computer with two GPUs choose the other one for Vestra in Windows Settings > System > Display > Graphics";
const SOFTWARE_REASONS = [
  { code: "no-gpu-backend", message: "this player has no GPU renderer on Windows yet" },
  { code: "headless", message: "the player runs without windows" },
  { code: "env", message: "VESTRA_RENDER is set" },
  { code: "preference", message: "chosen in the player's settings" },
  { code: "denylist", message: DENYLIST },
  { code: "previous-crash", message: "GPU rendering crashed at a start on 2026-10-28; it is tried again after a driver or player update, or on 2026-11-04" },
  { code: "init-failed", message: "GPU rendering unavailable: no Metal device" },
  { code: "window-fallback", message: "a window could not show GPU frames" },
  { code: "some-future-code", message: "a reason a newer player gives" },
];

async function openAbout(host: TestHost) {
  await host.command("info.about").run();
  await host.settle();
  return host.window("about");
}
/** The Node and Rendering lines of Copy Info. */
async function copied(host: TestHost) {
  host.clipboard.text = "";
  host.window("about").emit("copy-info");
  await until(() => host.clipboard.text !== "");
  return host.clipboard.text.split("\n").filter((line) => /^(Node|Rendering|渲染)[:：]/.test(line));
}

test("I09 Rendering: software shows the player's reason as it is, for every reason shape", async () => {
  const { host, stop } = await start();
  try {
    host.videoRender.gpu = false;
    for (const reason of SOFTWARE_REASONS) {
      host.rendering.reason = reason;
      const window = await openAbout(host);
      assert.equal(window.props.rendering, "software", reason.code);
      assert.equal(window.props["rendering-reason"], reason.message, `${reason.code}: the message unchanged`);
      assert.deepEqual(await copied(host), [`Node: v${process.versions.node}`, `Rendering: Software — ${reason.message}`], reason.code);
      window.userClose();
      await host.settle();
    }
  } finally {
    await stop();
  }
});

test("I09 Rendering: GPU alone for default and pending; another GPU reason (VESTRA_RENDER) is shown", async () => {
  const { host, stop } = await start();
  try {
    for (const [reason, shown] of [
      [{ code: "default", message: "nothing rules GPU rendering out" }, ""],
      [{ code: "pending", message: "GPU rendering is starting" }, ""],
      [{ code: "env", message: "VESTRA_RENDER is set" }, "VESTRA_RENDER is set"],
    ] as const) {
      host.rendering.reason = { ...reason };
      const window = await openAbout(host);
      assert.equal(window.props.rendering, "gpu");
      assert.equal(window.props["rendering-reason"], shown, reason.code);
      assert.deepEqual((await copied(host)).slice(1), [shown ? `Rendering: GPU — ${shown}` : "Rendering: GPU"], reason.code);
      window.userClose();
      await host.settle();
    }
  } finally {
    await stop();
  }
});

test("I09 Rendering: read again at each opening (GPU, then software after a window fallback)", async () => {
  const { host, stop } = await start();
  try {
    let window = await openAbout(host);
    assert.equal(window.props.rendering, "gpu");
    window.userClose();
    await host.settle();
    host.videoRender.gpu = false;
    host.rendering.reason = { code: "window-fallback", message: "a window could not show GPU frames" };
    window = await openAbout(host);
    assert.equal(window.props.rendering, "software");
    assert.equal(window.props["rendering-reason"], "a window could not show GPU frames");
  } finally {
    await stop();
  }
});

test("I09 Rendering: left out before player 0.5.12 and on a failed read; the mode alone before 0.5.13", async () => {
  const { host, stop } = await start();
  try {
    // Player 0.5.11: neither field.
    host.handle("app.system", () => structuredClone(host.systemInfo));
    let window = await openAbout(host);
    assert.equal(window.props.rendering, "", "not reported: no row, not a guess");
    assert.equal(window.props["rendering-reason"], "");
    assert.deepEqual(await copied(host), [`Node: v${process.versions.node}`]);
    window.userClose();
    await host.settle();
    // Player 0.5.12: the mode without a reason.
    host.handle("app.system", () => ({ ...structuredClone(host.systemInfo), rendering: "software" }));
    window = await openAbout(host);
    assert.equal(window.props.rendering, "software");
    assert.equal(window.props["rendering-reason"], "");
    assert.deepEqual((await copied(host)).slice(1), ["Rendering: Software"]);
    window.userClose();
    await host.settle();
    // A failed read: no row; the rest of the window as usual.
    host.handle("app.system", () => {
      throw new HostError("failed", "refused");
    });
    window = await openAbout(host);
    assert.equal(window.props.rendering, "");
    assert.equal((window.props.plugins as unknown[]).length, 3);
    assert.deepEqual(await copied(host), [`Node: v${process.versions.node}`]);
  } finally {
    await stop();
  }
});

test("I09 Rendering pure rules and words (English, Simplified Chinese)", async () => {
  const zh = await start("zh-Hans");
  const en = await start();
  try {
    assert.deepEqual(renderingRow(null), NO_RENDERING);
    assert.deepEqual(renderingRow({}), NO_RENDERING);
    assert.deepEqual(renderingRow({ rendering: "other" as never }), NO_RENDERING, "an unknown mode is not guessed");
    assert.deepEqual(renderingRow({ rendering: "gpu" }), { mode: "gpu", reason: "" });
    assert.deepEqual(renderingRow({ rendering: "software", renderingReason: { code: "denylist", message: DENYLIST } }), { mode: "software", reason: DENYLIST });
    // A software reason is never hidden, even with a code that is quiet under GPU rendering.
    assert.deepEqual(renderingRow({ rendering: "software", renderingReason: { code: "default", message: "x" } }), { mode: "software", reason: "x" });
    const value = (i18n: typeof en.i18n, row: Parameters<typeof renderingValue>[1]) => i18n.resolve(renderingValue(i18n, row));
    assert.equal(value(en.i18n, { mode: "gpu", reason: "" }), "GPU");
    assert.equal(value(en.i18n, { mode: "software", reason: "this player has no GPU renderer on Windows yet" }), "Software — this player has no GPU renderer on Windows yet");
    assert.equal(value(zh.i18n, { mode: "software", reason: "this player has no GPU renderer on Windows yet" }), "软件——this player has no GPU renderer on Windows yet");
    const bare = { version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "" } };
    assert.match(aboutText(zh.i18n, bare, null, [], { mode: "software", reason: "r" }), /\nNode：未知\n渲染：软件——r\n\n已加载插件\n/);
    assert.match(aboutText(en.i18n, bare, null, []), /\nNode: Unknown\n\nLoaded plugins\n/, "no row without a renderer");
    // about.slint words both modes and the reason the same way, and measures the label.
    const slint = await readFile(new URL("../ui/about.slint", import.meta.url), "utf8");
    assert.match(slint, /mode == "gpu" \? @tr\("GPU"\) : @tr\("Software"\)/);
    assert.match(slint, /@tr\("\{\} — \{\}", root\.rendering-name\(mode\), reason\)/);
    const measure = slint.slice(slint.indexOf("label-measure :="), slint.indexOf("version-measure :="));
    assert.match(measure, /@tr\("Plugin API"\)[\s\S]*@tr\("Rendering"\)/, "the label column fits Rendering too");
  } finally {
    await zh.stop();
    await en.stop();
  }
});
