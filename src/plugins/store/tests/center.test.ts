/**
 * The plugin center (2026-10-26; Store v3 2026-11-15): tabs, a card's state
 * and its one button, the detail page's buttons and ⋯ menu by state, enable / disable / reload / data folder through
 * `packages`, built-ins that can be neither disabled nor uninstalled, the
 * details the Plugin Manager used to show, offline with a short wait and a
 * calm note, and translated display fields.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { parseIndex, translationFor } from "../index-format.ts";
import { timing } from "../store.ts";
import {
  actions,
  allRows,
  ALL,
  BUILTIN,
  choose,
  controls,
  FakeNet,
  info,
  type Info,
  INSTALLED,
  manage,
  OFFICIAL,
  openStore,
  props,
  publish,
  rows,
  showBuiltins,
  start,
  tab,
  until,
  UPDATES,
  yes,
} from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-center-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const fromRepo = { kind: "repository", url: OFFICIAL } as const;
const dataOf = (id: string) => {
  const path = join(scratch, "data", id);
  mkdirSync(path, { recursive: true });
  return path;
};
const one = (id: string, name: string, versions = [{ version: "1.0.0" }]) => ({ id, name, versions });

/** A built-in, a running, a disabled, a failed and an outdated third-party plugin, and one not installed. */
async function center() {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    one("a.run", "A Run"),
    one("b.off", "B Off"),
    one("c.bad", "C Bad"),
    one("d.old", "D Old", [{ version: "2.0.0" }, { version: "1.0.0" }]),
    one("e.new", "E New"),
  ]);
  const packages: Info[] = [
    { ...BUILTIN, dataDirectory: dataOf("vestra.video") },
    info("a.run", { version: "1.0.0", source: fromRepo, dataDirectory: dataOf("a.run") }),
    info("b.off", { version: "1.0.0", source: fromRepo, state: "disabled", dataDirectory: dataOf("b.off") }),
    info("c.bad", { version: "1.0.0", source: fromRepo, state: "failed", error: "boom", dataDirectory: dataOf("c.bad") }),
    info("d.old", { version: "1.0.0", source: fromRepo, dataDirectory: dataOf("d.old") }),
  ];
  const started = await start({ net, packages });
  await openStore(started.host);
  // The built-in row is part of the center: shown ("Hide built-in plugins" is on by default).
  await showBuiltins(started.host);
  await until(started.host, () => started.store.items.length === 6, "rows");
  return started;
}

const ids = (list: { id: string }[]) => list.map((each) => each.id);
const enables = (host: Awaited<ReturnType<typeof start>>["host"]) =>
  host.packageCalls.filter((c) => c.method === "setEnabled").map((c) => [c.id, c.enabled]);
const reloads = (host: Awaited<ReturnType<typeof start>>["host"]) =>
  host.packageCalls.filter((c) => c.method === "reload").map((c) => c.id);

test("tabs: All first (every plugin, installed ones with their state), Installed (built-ins first), Updates; counts", async () => {
  const { host, stop } = await center();
  try {
    assert.equal(props(host)["category"], ALL, "the first open is All");
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.state, r.stateTone]),
      [
        // The state in words (Store v3 §3.4): ✓ installed in grey, or why it does not run, or what waits.
        ["A Run", "✓ 已安装", 4],
        ["B Off", "已停用", 2],
        ["C Bad", "失败", 2],
        ["D Old", "有更新", 1],
        ["E New", "", 0],
        ["vestra.video", "随播放器提供", 4],
      ],
    );
    assert.equal(props(host)["all-count"], 6);
    assert.equal(props(host)["installed-count"], 5);
    assert.equal(props(host)["updates"], 1);
    assert.equal(props(host)["list-note"], "", "an index was read");
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video", "A Run", "B Off", "C Bad", "D Old"]);
    await tab(host, UPDATES);
    assert.deepEqual(rows(host).map((r) => r.name), ["D Old"]);
  } finally {
    await stop();
  }
});

test("card buttons and the detail page's buttons and ⋯ menu by state", async () => {
  const { host, stop } = await center();
  try {
    // One button at most on a card (Store v3 §3.4): Install or Update; everything else is on the detail page.
    const builtin = controls(host, "vestra.video");
    assert.equal(builtin.button, null, "built-ins have no button");
    let page = await manage(host, "vestra.video");
    // A running built-in is restarted through packages.reload (player 0.5.1, request STORE-RELOAD-BUILTIN).
    assert.deepEqual(page.buttons, [{ id: "reload", text: "重新加载", enabled: true }]);
    assert.deepEqual(page.more, [{ id: "openData", text: "打开数据目录", enabled: true }]);

    assert.deepEqual(builtin.menu, [], "built-ins have no ⋯ either");
    const running = controls(host, "A Run");
    assert.equal(running.button, null);
    // Its card's ⋯ (Store v4 §3): Disable, Uninstall…; Reload and Open Data Folder stay on the page.
    assert.deepEqual(running.menu, ["disable", "uninstall"]);
    assert.equal(running.dim, false);
    page = await manage(host, "A Run");
    // One fixed order (Disable/Enable, Uninstall…, Reload), Open Data Folder in the ⋯ menu.
    assert.deepEqual(page.buttons.map((a) => a.text), ["停用", "卸载…", "重新加载"]);
    assert.deepEqual(ids(page.more), ["openData"]);

    const disabled = controls(host, "B Off");
    // A disabled card's text button is Enable; its ⋯ reads Enable, Uninstall….
    assert.deepEqual(disabled.button, { id: "enable", text: "启用", enabled: true });
    assert.deepEqual(disabled.menu, ["enable", "uninstall"]);
    assert.equal(disabled.dim, true);
    page = await manage(host, "B Off");
    assert.deepEqual(ids(page.buttons), ["enable", "uninstall"], "a disabled plugin is not reloaded");
    assert.deepEqual(ids(page.more), ["openData"]);

    assert.equal(controls(host, "C Bad").button, null);
    assert.deepEqual(controls(host, "C Bad").menu, ["disable", "uninstall"], "no Reload on the card");
    assert.equal(rows(host).find((r) => r.name === "C Bad")!.tip, "boom", "the reason on hover");
    page = await manage(host, "C Bad");
    assert.deepEqual(ids(page.buttons), ["disable", "uninstall", "reload"]);

    // An update: the Update button on the card, its ⋯ beside it.
    assert.deepEqual(controls(host, "D Old").button, { id: "update", text: "更新", enabled: true });
    assert.deepEqual(controls(host, "D Old").menu, ["disable", "uninstall"]);

    await tab(host, ALL);
    assert.deepEqual(controls(host, "E New").button, { id: "install", text: "安装", enabled: true });
    assert.deepEqual(controls(host, "E New").menu, []);
    page = await manage(host, "E New");
    assert.deepEqual(page.buttons, [{ id: "install", text: "安装 1.0.0", enabled: true }]);
    assert.deepEqual(page.more, []);
  } finally {
    await stop();
  }
});

test("detail page: disable and enable through packages.setEnabled; the card dims", async () => {
  const { host, stop } = await center();
  try {
    await choose(host, "A Run");
    const window = host.window("store");
    window.emit("act", "disable");
    await until(host, () => controls(host, "A Run").dim, "dimmed");
    assert.deepEqual(enables(host), [["a.run", false]]);
    assert.equal(host.confirms.length, 0, "no question: it can be enabled again at once");
    assert.deepEqual(controls(host, "A Run").button, { id: "enable", text: "启用", enabled: true });
    assert.equal(controls(host, "A Run").state, "已停用");
    assert.equal(props(host)["status-text"], "已停用「A Run」");
    assert.ok(host.osd.includes("已停用「A Run」"));
    // Enable in Disable's place; Reload leaves while disabled.
    assert.deepEqual(actions(host).map((a) => a.text), ["启用", "卸载…"]);

    window.emit("act", "enable");
    await until(host, () => !controls(host, "A Run").dim, "enabled again");
    assert.deepEqual(enables(host), [
      ["a.run", false],
      ["a.run", true],
    ]);
    assert.equal(props(host)["status-text"], "已启用「A Run」");
    assert.deepEqual(actions(host).map((a) => a.text), ["停用", "卸载…", "重新加载"]);
  } finally {
    await stop();
  }
});

test("detail page: reload restarts a running plugin and starts a failed one (packages.reload); open the data folder", async () => {
  const { host, stop } = await center();
  try {
    const window = host.window("store");
    await choose(host, "A Run");
    window.emit("act", "reload");
    await until(host, () => props(host)["status-text"] === "已重新加载「A Run」", "reloaded");
    assert.deepEqual(reloads(host), ["a.run"]);
    assert.deepEqual(enables(host), [], "never through setEnabled: a reload leaves no disabled flag behind");
    await choose(host, "C Bad");
    window.emit("act", "reload");
    await until(
      host,
      () => props(host)["status-text"] === "已重新加载「C Bad」" && controls(host, "C Bad").state === "✓ 已安装",
      "failed one started",
    );
    assert.deepEqual(reloads(host), ["a.run", "c.bad"]);

    await choose(host, "A Run");
    window.emit("act", "openData");
    await until(host, () => host.system.opened.length === 1, "opened");
    assert.deepEqual(host.system.opened, [dataOf("a.run")]);
    host.handle("system.openPath", () => {
      throw new HostError("missing", "no such folder");
    });
    await choose(host, "B Off");
    window.emit("act", "openData");
    await until(host, () => props(host)["status-text"] === "它的数据目录还不存在", "missing folder");
    assert.equal(props(host)["status-warn"], true);

    // Uninstall… keeps today's question.
    host.dialogAnswers.push(yes);
    window.emit("act", "uninstall");
    await until(host, () => host.packageCalls.some((c) => c.method === "uninstall"), "uninstall");
    assert.equal(host.confirmTexts.at(-1)!.message, "卸载「B Off」？");
  } finally {
    await stop();
  }
});

test("built-ins: no disable and no uninstall anywhere; a refused disable says so", async () => {
  const { host, store, stop } = await center();
  try {
    await choose(host, "vestra.video");
    assert.deepEqual(ids(actions(host)), ["reload"], "only Reload under the title");
    // Even if asked (a stale window): the player refuses and the store words it.
    host.window("store").emit("cardAct", "\nvestra.video", "disable");
    await until(host, () => props(host)["status-text"] === "内置插件不能停用", "refused");
    assert.deepEqual(enables(host), []);
    assert.equal(store.items.find((i) => i.id === "vestra.video")?.installed?.state, "running");

    // A running built-in is restarted, a failed one started again: packages.reload either way.
    host.window("store").emit("act", "reload");
    await until(host, () => props(host)["status-text"] === "已重新加载「vestra.video」", "restarted");
    host.setPackages(host.packages.map((p) => (p.id === "vestra.video" ? { ...p, state: "failed", error: "crash" } : p)));
    await until(host, () => controls(host, "vestra.video").state === "失败", "failed");
    host.window("store").emit("act", "reload");
    await until(host, () => reloads(host).length === 2 && controls(host, "vestra.video").state === "随播放器提供", "started");
    assert.deepEqual(reloads(host), ["vestra.video", "vestra.video"]);
    assert.deepEqual(enables(host), []);
  } finally {
    await stop();
  }
});

test("details: status, package, data folder (opens) and data size", async () => {
  const data = join(scratch, "data-a");
  mkdirSync(join(data, "cache"), { recursive: true });
  writeFileSync(join(data, "settings.json"), "x".repeat(1000));
  writeFileSync(join(data, "cache", "blob"), "y".repeat(536));
  const net = new FakeNet();
  publish(net, OFFICIAL, [one("a.run", "A Run")]);
  const { host, stop } = await start({
    net,
    packages: [
      BUILTIN,
      info("a.run", { version: "1.0.0", source: fromRepo, dataDirectory: data, directory: "/packages/a.run" }),
      info("c.bad", { version: "1.0.0", state: "failed", error: "boom" }),
    ],
  });
  try {
    await openStore(host);
    await choose(host, "A Run");
    type Fact = { label: string; text: string; link: boolean };
    const facts = () => props(host)["facts"] as Fact[];
    const fact = (label: string) => facts().find((f) => f.label === label);
    assert.equal(fact("状态")?.text, "运行中");
    assert.equal(fact("包目录")?.text, "/packages/a.run");
    assert.deepEqual(fact("数据目录"), { label: "数据目录", text: data, link: true });
    await until(host, () => fact("数据大小")?.text === "1.5 KB", "measured");
    host.window("store").emit("factLink", facts().findIndex((f) => f.label === "数据目录"));
    await until(host, () => host.system.opened.length === 1);
    assert.deepEqual(host.system.opened, [data]);
    // The Plugin Manager's sentence is gone; the permissions keep their label.
    assert.equal(props(host)["manage"], undefined);

    await choose(host, "c.bad");
    assert.equal(fact("状态")?.text, "失败：boom");
    assert.equal(fact("数据目录")?.text, "—");
    assert.equal(fact("数据大小")?.text, "—");
  } finally {
    await stop();
  }
});

test("offline on open: a short wait for the headers, then a calm note; All shows the installed plugins, Installed works", async () => {
  const defaults = { ...timing };
  const net = new FakeNet();
  publish(net, OFFICIAL, [one("a.alpha", "Alpha")]);
  // A blackholed network: the server never answers.
  net.route(OFFICIAL, { delay: 60_000, body: "{}" });
  Object.assign(timing, { openHeaders: 300 });
  const side = info("s.side", { version: "1.0.0", source: { kind: "file" } });
  const { host, store, stop } = await start({ net, packages: [BUILTIN, side] });
  try {
    const opened = Date.now();
    await openStore(host);
    assert.equal(props(host)["category"], ALL);
    // All needs no network for its installed part; it counts what it can show.
    assert.deepEqual(rows(host).map((r) => r.name), ["s.side"]);
    assert.equal(props(host)["all-count"], 1);
    assert.equal(props(host)["list-note"], "正在读取仓库…");
    await until(host, () => !store.repos.official.refreshing, "gave up");
    assert.ok(Date.now() - opened < 10_000, "not the 15 s header deadline");
    assert.equal(props(host)["list-note"], "无法连接仓库，只显示已安装的插件。点「刷新」重试。");
    assert.equal(props(host)["empty-text"], "");
    assert.match(String(props(host)["status-text"]), /^无法连接 官方仓库/);
    assert.equal(props(host)["status-warn"], false, "calm, not the warning colour");
    // Its detail page keeps its buttons offline.
    assert.equal(controls(host, "s.side").button, null);
    assert.deepEqual(ids((await manage(host, "s.side")).buttons), ["disable", "uninstall", "reload"]);
    host.window("store").emit("back");
    await host.settle();
    await showBuiltins(host);
    assert.deepEqual(rows(host).map((r) => r.name), ["s.side", "vestra.video"]);
    assert.equal(props(host)["all-count"], 2);
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video", "s.side"], "the Installed tab needs no network");
    assert.equal(props(host)["list-note"], "");
    await tab(host, ALL);
    // Nothing installed to show: the empty list says it.
    host.window("store").emit("search", "alpha");
    await host.settle();
    assert.equal(props(host)["empty-text"], "没有符合条件的插件");
    host.window("store").emit("search", "");
    host.window("store").emit("setHideBuiltins", true);
    host.setPackages([BUILTIN]);
    await until(host, () => rows(host).length === 0, "only the hidden built-in");
    assert.equal(props(host)["empty-text"], "无法连接仓库。联网之前这里不显示内容；点「刷新」重试。");
    assert.equal(props(host)["list-note"], "");
    await tab(host, UPDATES);
    assert.equal(props(host)["empty-text"], "无法连接仓库。联网之前这里不显示内容；点「刷新」重试。");
  } finally {
    Object.assign(timing, defaults);
    await stop();
  }
});

test("index translations: the interface language, its base language, then English; search matches both", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    {
      id: "x.tube",
      name: "Tube Player",
      tags: ["video", "online"],
      translations: { "zh-Hans": { name: "管道播放器", punchline: "播放管道", tags: ["视频", "online"] }, ja: { name: "チューブ" } },
      versions: [{ version: "1.0.0" }],
    },
    { id: "x.base", name: "Base Only", translations: { zh: { name: "基础" } }, versions: [{ version: "1.0.0" }] },
    { id: "x.none", name: "No Translation", tags: ["audio"], versions: [{ version: "1.0.0" }] },
  ]);
  const { host, stop } = await start({ net });
  try {
    await openStore(host);
    await tab(host, ALL);
    await until(host, () => rows(host).length === 3);
    assert.deepEqual(rows(host).map((r) => r.name).sort(), ["No Translation", "基础", "管道播放器"].sort());
    assert.equal(rows(host).find((r) => r.name === "管道播放器")!.intro, "播放管道");
    // A fixed list of categories, matched by the tags as shown and the English ones.
    assert.deepEqual(props(host)["goal-model"], [
      "分类：全部",
      "画面",
      "声音",
      "字幕",
      "在线视频",
      "媒体库与播放列表",
      "工具",
      "游戏",
      "其他",
    ]);
    const window = host.window("store");
    for (const query of ["tube", "管道", "video"]) {
      window.emit("search", query);
      await host.settle();
      assert.deepEqual(rows(host).map((r) => r.name), ["管道播放器"], query);
    }
    window.emit("search", "");
    // Picture: "video".
    window.emit("setGoal", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["管道播放器"]);
    // Online video: "online"; Sound: "audio"; Other: no tag a category knows.
    window.emit("setGoal", 4);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["管道播放器"]);
    window.emit("setGoal", 2);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["No Translation"]);
    window.emit("setGoal", 8);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["基础"]);
  } finally {
    await stop();
  }

  // English: the index's own fields.
  const en = await start({ net, locale: "en" });
  try {
    await openStore(en.host);
    await tab(en.host, ALL);
    await until(en.host, () => rows(en.host).length === 3);
    assert.deepEqual(rows(en.host).map((r) => r.name), ["Base Only", "No Translation", "Tube Player"]);
  } finally {
    await en.stop();
  }
});

test("installed plugins show the player's translation (PluginInfo.display), English still found", async () => {
  const directory = join(scratch, "clock");
  mkdirSync(directory, { recursive: true });
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({ id: "x.clock", version: "1.0.0", name: "Clock", punchline: "Shows the time", tags: ["time"] }),
  );
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const clock = {
    ...info("x.clock", { directory }),
    display: { name: "时钟", punchline: "显示时间", description: null, tags: ["时间"] },
  } as Info;
  const plain = info("x.plain", { directory: join(scratch, "missing") });
  const { host, stop } = await start({ net, packages: [BUILTIN, clock, plain] });
  try {
    await openStore(host);
    await until(host, () => rows(host).some((r) => r.name === "时钟"), "translated");
    assert.equal(rows(host).find((r) => r.name === "时钟")!.intro, "显示时间");
    // No display and no manifest: the id.
    assert.ok(rows(host).some((r) => r.name === "x.plain"));
    host.window("store").emit("search", "clock");
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["时钟"]);
    host.window("store").emit("search", "");
    await host.settle();
    assert.equal((await allRows(host)).length, 2, "the built-in is hidden");
    await showBuiltins(host);
    assert.equal((await allRows(host)).length, 3);
  } finally {
    await stop();
  }
});

test("translations in index.json: English limits, invalid values dropped, field by field fallback", () => {
  const long = "长".repeat(100);
  const text = JSON.stringify({
    schema: 1,
    plugins: [
      {
        id: "x.t",
        name: "Name",
        tags: ["a"],
        versions: [
          {
            version: "1.0.0",
            apiVersion: "vestra/1",
            capabilities: [],
            downloadUrl: "https://x.test/x.vxp",
            sha256: "0".repeat(64),
            size: 1,
            published: "2026-10-21T00:00:00Z",
          },
        ],
        translations: {
          "zh-Hans": { name: long, punchline: 42, tags: ["一", "x".repeat(40), 7] },
          zh: { punchline: "基础简介", description: "说明" },
          "bad locale": { name: "no" },
          de: "not an object",
        },
      },
      {
        id: "x.u",
        name: "U",
        versions: [
          {
            version: "1.0.0",
            apiVersion: "vestra/1",
            capabilities: [],
            downloadUrl: "https://x.test/u.vxp",
            sha256: "0".repeat(64),
            size: 1,
            published: "2026-10-21T00:00:00Z",
          },
        ],
      },
    ],
  });
  const [withT, without] = parseIndex(text).index.plugins;
  assert.deepEqual(Object.keys(withT!.translations).sort(), ["zh", "zh-Hans"]);
  const zh = translationFor(withT!.translations, "zh-Hans");
  assert.equal([...zh.name!].length, 64, "name cut like the English one");
  assert.equal(zh.punchline, "基础简介", "a missing field comes from the base language");
  assert.equal(zh.description, "说明");
  assert.deepEqual(zh.tags, ["一", `${"x".repeat(23)}…`]);
  assert.deepEqual(translationFor(withT!.translations, "ja"), {}, "no entry: English");
  assert.deepEqual(without!.translations, {});
});

test("buttons keep their places: Disable and Enable swap only their words (VS Code's extension page)", async () => {
  const { host, stop } = await center();
  try {
    const detail = async (name: string) => {
      await choose(host, name);
      return ids(actions(host));
    };
    // Reload last (Store v3: every running or failed plugin has it), so disabling only takes Reload away.
    assert.deepEqual(await detail("A Run"), ["disable", "uninstall", "reload"], "enabled");
    assert.deepEqual(await detail("B Off"), ["enable", "uninstall"], "disabled");
    assert.deepEqual(await detail("C Bad"), ["disable", "uninstall", "reload"], "failed");
    assert.deepEqual(await detail("D Old"), ["update", "disable", "uninstall", "reload"], "update available");
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 2.0.0", "停用", "卸载…", "重新加载"]);
    host.window("store").emit("act", "reload");
    await until(host, () => reloads(host).includes("d.old"), "reloaded from the details");
    // Starting: Reload stays in its place, disabled.
    host.setPackages(host.packages.map((p) => (p.id === "d.old" ? { ...p, state: "starting" } : p)));
    await until(host, () => actions(host).at(-1)?.enabled === false, "starting");
    assert.deepEqual(ids(actions(host)), ["update", "disable", "uninstall", "reload"]);
    host.setPackages(host.packages.map((p) => (p.id === "d.old" ? { ...p, state: "running" } : p)));
    await until(host, () => actions(host).at(-1)?.enabled === true, "running again");

    // Disabled with an update: Update stays first, Enable takes Disable's place.
    host.setPackages(host.packages.map((p) => (p.id === "d.old" ? { ...p, state: "disabled" } : p)));
    await until(host, () => controls(host, "D Old").dim, "D Old disabled");
    assert.deepEqual(ids(actions(host)), ["update", "enable", "uninstall"]);
    // Update wins over Enable; the card says the update and stays dimmed (store-v4 §3).
    assert.deepEqual(controls(host, "D Old").button, { id: "update", text: "更新", enabled: true });
    assert.deepEqual(controls(host, "D Old").menu, ["enable", "uninstall"]);
    assert.equal(controls(host, "D Old").state, "有更新");
  } finally {
    await stop();
  }
});

test("plugins the player set aside at start (no directory, no version) keep their own names", async () => {
  // Player 0.9.0 lists an installed record it set aside (a package with `capabilities`) without a directory or a
  // version; the display cache was keyed by those two only, so every such row showed the first one's name.
  const aside = (id: string, name: string | null) =>
    info(id, {
      version: null,
      state: "failed",
      error: "the plugin library's record of this plugin was set aside: unreadable",
      directory: "",
      source: fromRepo,
      display: { name, punchline: null, description: null, tags: [] },
    } as Partial<Info>);
  const { host, store, stop } = await start({
    packages: [BUILTIN, aside("z.amd", "AMD Motion"), aside("z.auto", "Auto Frame"), aside("z.clip", null)],
  });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 4, "rows");
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name).sort(), ["AMD Motion", "Auto Frame", "z.clip"]);
    assert.equal(controls(host, "Auto Frame").state, "失败");
    // Nothing of it runs: no Disable, no Reload; Uninstall… (player 0.10.1 removes it completely) and Open Data Folder.
    const page = await manage(host, "Auto Frame");
    assert.deepEqual(ids(page.buttons), ["uninstall"]);
    assert.deepEqual(ids(page.more), ["openData"]);
  } finally {
    await stop();
  }
});
