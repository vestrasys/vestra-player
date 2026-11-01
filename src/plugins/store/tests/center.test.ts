/**
 * The plugin center (round 2026-10 UI polish, step 1): tabs, one button and a
 * … menu per row by state, enable / disable / reload / data folder through
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
  AVAILABLE,
  BUILTIN,
  choose,
  controls,
  FakeNet,
  info,
  type Info,
  INSTALLED,
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

test("tabs: Installed first (built-ins, then third-party), Available, Updates; counts", async () => {
  const { host, stop } = await center();
  try {
    assert.equal(props(host)["category"], INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video", "A Run", "B Off", "C Bad", "D Old"]);
    assert.equal(props(host)["installed-count"], 5);
    assert.equal(props(host)["available-count"], 1);
    assert.equal(props(host)["available-known"], true);
    assert.equal(props(host)["updates"], 1);
    await tab(host, AVAILABLE);
    assert.deepEqual(rows(host).map((r) => r.name), ["E New"]);
    await tab(host, UPDATES);
    assert.deepEqual(rows(host).map((r) => r.name), ["D Old"]);
  } finally {
    await stop();
  }
});

test("row buttons and … menus by state", async () => {
  const { host, stop } = await center();
  try {
    const builtin = controls(host, "vestra.video");
    assert.equal(builtin.primary, null, "built-ins have no button");
    assert.deepEqual(ids(builtin.menu), ["reload", "openData"]);
    // A running built-in is restarted through packages.reload (player 0.5.1, request STORE-RELOAD-BUILTIN).
    assert.deepEqual(builtin.menu.map((m) => m.enabled), [true, true]);
    assert.equal(builtin.badge, "内置");

    const running = controls(host, "A Run");
    assert.deepEqual(running.primary, { id: "uninstall", text: "卸载…", enabled: true });
    assert.deepEqual(ids(running.menu), ["disable", "reload", "openData"]);
    assert.deepEqual(running.menu.map((m) => m.text), ["停用", "重新加载", "打开数据目录"]);
    assert.equal(running.dim, false);

    const disabled = controls(host, "B Off");
    assert.deepEqual(disabled.primary, { id: "enable", text: "启用", enabled: true });
    assert.deepEqual(ids(disabled.menu), ["uninstall", "openData"]);
    assert.equal(disabled.dim, true);
    assert.equal(disabled.badge, "已停用");

    const failed = controls(host, "C Bad");
    assert.deepEqual(failed.primary, { id: "reload", text: "重新加载", enabled: true });
    assert.deepEqual(ids(failed.menu), ["disable", "uninstall", "openData"]);
    assert.equal(failed.badge, "失败");
    assert.equal((rows(host).find((r) => r.name === "C Bad") as { tip?: string }).tip, "boom", "the reason on hover");

    const outdated = controls(host, "D Old");
    assert.deepEqual(outdated.primary, { id: "update", text: "更新", enabled: true });
    assert.deepEqual(ids(outdated.menu), ["disable", "uninstall", "reload", "openData"]);

    await tab(host, AVAILABLE);
    const available = controls(host, "E New");
    assert.deepEqual(available.primary, { id: "install", text: "安装", enabled: true });
    assert.deepEqual(available.menu, []);
  } finally {
    await stop();
  }
});

test("… menu: disable and enable through packages.setEnabled; the row dims and its button follows", async () => {
  const { host, stop } = await center();
  try {
    const key = rows(host).find((r) => r.name === "A Run")!.key;
    const window = host.window("store");
    window.emit("rowAct", key, "disable");
    await until(host, () => controls(host, "A Run").dim, "dimmed");
    assert.deepEqual(enables(host), [["a.run", false]]);
    assert.equal(host.confirms.length, 0, "no question: it can be enabled again at once");
    assert.deepEqual(controls(host, "A Run").primary, { id: "enable", text: "启用", enabled: true });
    assert.equal(controls(host, "A Run").badge, "已停用");
    assert.equal(props(host)["status-text"], "已停用「A Run」");
    assert.ok(host.osd.includes("已停用「A Run」"));
    // The row's key selected it: the detail mirrors the row.
    assert.deepEqual(actions(host).map((a) => a.text), ["启用", "卸载…"]);

    window.emit("rowAct", key, "enable");
    await until(host, () => !controls(host, "A Run").dim, "enabled again");
    assert.deepEqual(enables(host), [
      ["a.run", false],
      ["a.run", true],
    ]);
    assert.equal(props(host)["status-text"], "已启用「A Run」");
    assert.deepEqual(actions(host).map((a) => a.text), ["卸载…", "停用"]);
  } finally {
    await stop();
  }
});

test("… menu: reload restarts a running plugin and starts a failed one (packages.reload); open the data folder", async () => {
  const { host, stop } = await center();
  try {
    const window = host.window("store");
    const key = (name: string) => rows(host).find((r) => r.name === name)!.key;
    window.emit("rowAct", key("A Run"), "reload");
    await until(host, () => props(host)["status-text"] === "已重新加载「A Run」", "reloaded");
    assert.deepEqual(reloads(host), ["a.run"]);
    assert.deepEqual(enables(host), [], "never through setEnabled: a reload leaves no disabled flag behind");
    window.emit("rowAct", key("C Bad"), "reload");
    await until(
      host,
      () => props(host)["status-text"] === "已重新加载「C Bad」" && controls(host, "C Bad").badge !== "失败",
      "failed one started",
    );
    assert.deepEqual(reloads(host), ["a.run", "c.bad"]);

    window.emit("rowAct", key("A Run"), "openData");
    await until(host, () => host.system.opened.length === 1, "opened");
    assert.deepEqual(host.system.opened, [dataOf("a.run")]);
    host.handle("system.openPath", () => {
      throw new HostError("missing", "no such folder");
    });
    window.emit("rowAct", key("B Off"), "openData");
    await until(host, () => props(host)["status-text"] === "它的数据目录还不存在", "missing folder");
    assert.equal(props(host)["status-warn"], true);

    // Uninstall from the … menu keeps today's question.
    host.dialogAnswers.push(yes);
    window.emit("rowAct", key("B Off"), "uninstall");
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
    assert.deepEqual(actions(host), [], "no button under the title");
    // Even if asked (a stale window): the player refuses and the store words it.
    host.window("store").emit("rowAct", "\nvestra.video", "disable");
    await until(host, () => props(host)["status-text"] === "内置插件不能停用", "refused");
    assert.deepEqual(enables(host), []);
    assert.equal(store.items.find((i) => i.id === "vestra.video")?.installed?.state, "running");

    // A running built-in is restarted, a failed one started again: packages.reload either way.
    host.window("store").emit("rowAct", "\nvestra.video", "reload");
    await until(host, () => props(host)["status-text"] === "已重新加载「vestra.video」", "restarted");
    host.setPackages(host.packages.map((p) => (p.id === "vestra.video" ? { ...p, state: "failed", error: "crash" } : p)));
    await until(host, () => controls(host, "vestra.video").badge === "失败", "failed");
    host.window("store").emit("rowAct", "\nvestra.video", "reload");
    await until(host, () => reloads(host).length === 2 && controls(host, "vestra.video").badge === "内置", "started");
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

test("offline on open: a short wait for the headers, then a calm note; Installed works meanwhile", async () => {
  const defaults = { ...timing };
  const net = new FakeNet();
  publish(net, OFFICIAL, [one("a.alpha", "Alpha")]);
  // A blackholed network: the server never answers.
  net.route(OFFICIAL, { delay: 60_000, body: "{}" });
  Object.assign(timing, { openHeaders: 300 });
  const { host, store, stop } = await start({ net });
  try {
    const opened = Date.now();
    await openStore(host);
    await showBuiltins(host);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video"], "the Installed tab needs no network");
    await tab(host, AVAILABLE);
    assert.equal(props(host)["available-known"], false);
    await until(host, () => !store.repos.official.refreshing, "gave up");
    assert.ok(Date.now() - opened < 10_000, "not the 15 s header deadline");
    assert.equal(
      props(host)["empty-text"],
      "无法连接仓库。联网之前这里不显示内容；点「刷新」重试。",
    );
    assert.match(String(props(host)["status-text"]), /^无法连接 官方仓库/);
    assert.equal(props(host)["status-warn"], false, "calm, not the warning colour");
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
    await tab(host, AVAILABLE);
    await until(host, () => rows(host).length === 3);
    assert.deepEqual(rows(host).map((r) => r.name).sort(), ["No Translation", "基础", "管道播放器"].sort());
    assert.equal((rows(host).find((r) => r.name === "管道播放器") as { punchline?: string }).punchline, "播放管道");
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "视频", "audio", "online"]);
    const window = host.window("store");
    for (const query of ["tube", "管道", "video"]) {
      window.emit("search", query);
      await host.settle();
      assert.deepEqual(rows(host).map((r) => r.name), ["管道播放器"], query);
    }
    window.emit("search", "");
    window.emit("setTag", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["管道播放器"]);
  } finally {
    await stop();
  }

  // English: the index's own fields.
  const en = await start({ net, locale: "en" });
  try {
    await openStore(en.host);
    await tab(en.host, AVAILABLE);
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
    assert.equal((rows(host).find((r) => r.name === "时钟") as { punchline?: string }).punchline, "显示时间");
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
