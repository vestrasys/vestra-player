import assert from "node:assert/strict";
import test from "node:test";
import { timing } from "../store.ts";
import { actions, banners, BUILTIN, choose, FakeNet, info, OFFICIAL, openStore, props, publish, rows, start, THIRD, until, yes } from "./helpers.ts";

const path = (host: Awaited<ReturnType<typeof start>>["host"]) => host.command("open").declaration.path;

test("menu 「插件商城…」 counts updates from the same source only, announces them once", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "vestra.youtube", name: "YouTube", versions: [{ version: "0.3.0" }, { version: "0.2.0" }] },
    { id: "vestra.video", name: "Video", versions: [{ version: "9.0.0" }] },
  ]);
  publish(net, THIRD, [{ id: "someone.x", name: "X", versions: [{ version: "5.0.0" }] }]);
  const { host, store, stop } = await start({
    net,
    packages: [
      BUILTIN,
      info("vestra.youtube", { version: "0.2.0", source: { kind: "repository", url: OFFICIAL } }),
      // Installed from the official repository, but only the third-party one has a newer version.
      info("someone.x", { version: "1.0.0", source: { kind: "repository", url: OFFICIAL } }),
    ],
  });
  try {
    const command = host.command("open");
    assert.deepEqual(command.declaration.path, ["插件商城…"]);
    assert.equal(command.declaration.order, 822);
    assert.equal(command.declaration.group, "windows");
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    await until(host, () => path(host)?.[0] !== "插件商城…", "menu title");
    assert.deepEqual(path(host), ["插件商城（1 个可更新）…"]);
    assert.deepEqual(host.osd, ["插件商城：1 个插件可以更新"]);
    await store.refreshAll(true);
    await host.settle(20);
    assert.equal(host.osd.length, 1, "announced once per run");

    await openStore(host);
    host.window("store").emit("setFilter", 2);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["YouTube"]);
    await choose(host, "YouTube");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => path(host)?.[0] === "插件商城…", "count gone after the update");
    await until(host, () => !store.busy);
    assert.equal(props(host)["empty-text"], "所有插件都是最新的");
  } finally {
    await stop();
  }
});

test("automatic check: once after activation, then daily; off means no network", async () => {
  Object.assign(timing, { firstCheck: 30, tick: 10, interval: 150 });
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const first = await start({ net });
  try {
    assert.equal(net.count(OFFICIAL), 0, "not during activation");
    await until(first.host, () => net.count(OFFICIAL) === 1, "first check");
    await until(first.host, () => net.count(OFFICIAL) === 2, "next check after the interval");
    assert.ok(first.store.settings.value.lastCheck > 0);
  } finally {
    await first.stop();
  }
  const quiet = new FakeNet();
  publish(quiet, OFFICIAL, []);
  const second = await start({ net: quiet });
  try {
    second.store.settings.save({ ...second.store.settings.value, autoCheck: false });
    await second.host.settle(300);
    assert.equal(quiet.count(OFFICIAL), 0);
  } finally {
    await second.stop();
    Object.assign(timing, { firstCheck: 1_000_000_000, tick: 1_000_000_000, interval: 86_400_000 });
  }
});

test("list: filters, search, tags, sort; incompatible and fully revoked plugins", async () => {
  const net = new FakeNet();
  publish(
    net,
    OFFICIAL,
    [
      { id: "a.alpha", name: "Alpha", author: "Ann", tags: ["网络视频"], versions: [{ version: "1.0.0", published: "2026-01-01T00:00:00Z" }] },
      { id: "b.beta", name: "Beta", tags: ["字幕"], versions: [{ version: "1.0.0", published: "2026-09-01T00:00:00Z" }] },
      { id: "c.gamma", name: "Gamma", versions: [{ version: "2.0.0", minPlayerVersion: "9.0.0" }] },
      { id: "d.delta", name: "Delta", versions: [{ version: "1.0.0" }] },
    ],
    { revoked: [{ id: "d.delta", versions: "*", reason: "停止维护" }] },
  );
  const { host, stop } = await start({
    net,
    packages: [BUILTIN, info("b.beta", { version: "1.0.0", source: { kind: "repository", url: OFFICIAL } })],
  });
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 5);
    const window = host.window("store");
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha", "Beta", "Delta", "Gamma", "vestra.video"]);
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "网络视频", "字幕"]);
    window.emit("setTag", 2);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);
    window.emit("setTag", 0);
    window.emit("search", "ANN");
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha"]);
    window.emit("search", "zzz");
    await host.settle();
    assert.equal(props(host)["empty-text"], "没有符合条件的插件");
    window.emit("search", "");
    window.emit("setFilter", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => [r.name, r.badge]), [
      ["Beta", "已安装"],
      ["vestra.video", "内置"],
    ]);
    window.emit("setFilter", 0);
    window.emit("setSort", 1);
    await host.settle();
    assert.deepEqual(rows(host).slice(0, 4).map((r) => r.name), ["Delta", "Gamma", "Beta", "Alpha"]);

    const gamma = await choose(host, "Gamma");
    assert.equal(gamma.badge, "不兼容");
    assert.deepEqual(actions(host), []);
    assert.ok(banners(host).includes("需要 Vestra Player ≥ 9.0.0（当前 0.2.0）"));
    const delta = await choose(host, "Delta");
    assert.equal(delta.badge, "已撤回");
    assert.ok(banners(host).includes("此插件已被撤回：停止维护"));
    await choose(host, "vestra.video");
    assert.ok(banners(host).some((b) => b.startsWith("内置插件随播放器更新")));
    assert.deepEqual(actions(host), []);
  } finally {
    await stop();
  }
});

test("README and changelog tabs; missing ones say so; 显示图片 for a third-party README", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.alpha", name: "Alpha", readme: "# Alpha\n\n![x](https://img.test/a.png)", changelog: "## 1.0.0\n\n- first" },
    { id: "b.beta", name: "Beta", readme: null, versions: [{ version: "1.0.0" }] },
  ].map((p) => ({ versions: [{ version: "1.0.0" }], ...p })));
  publish(net, THIRD, [{ id: "c.gamma", name: "Gamma", readme: "![a](https://img.test/a.png) ![b](https://img.test/b.png)", versions: [{ version: "1.0.0" }] }]);
  const { host, store, stop } = await start({ net });
  try {
    host.remoteImages.set("https://img.test/a.png", null);
    host.remoteImages.set("https://img.test/b.png", null);
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => rows(host).length === 4);
    await choose(host, "Alpha");
    await until(host, () => host.markdown(props(host)["doc"]) !== null, "readme");
    assert.match(host.markdown(props(host)["doc"])!.text, /^# Alpha/);
    assert.ok(!banners(host).some((b) => b.startsWith("已阻止")), "official READMEs load remote images");
    host.window("store").emit("setTab", 1);
    await until(host, () => host.markdown(props(host)["doc"])?.text.startsWith("## 1.0.0") === true, "changelog");
    await choose(host, "Beta");
    await until(host, () => props(host)["doc-note"] === "作者没有提供更新日志");
    host.window("store").emit("setTab", 0);
    await until(host, () => props(host)["doc-note"] === "作者没有提供说明");

    await choose(host, "Gamma");
    await until(host, () => banners(host).includes("已阻止 2 张远程图片（非官方来源）"));
    host.window("store").emit("bannerAct", "showImages");
    await until(host, () => !banners(host).some((b) => b.startsWith("已阻止")), "images allowed");
    assert.equal(host.markdown(props(host)["doc"])?.text, "![a](https://img.test/a.png) ![b](https://img.test/b.png)");

    // A README that fails to load offers 重试.
    const readme = "https://plugins.test/assets/b.beta/README.md";
    net.route(readme, { status: 500 });
    store.repos.official.index!.plugins[1]!.readmeUrl = readme;
    await choose(host, "Beta");
    await until(host, () => banners(host).includes("无法加载说明：HTTP 500"), "failure banner");
    net.route(readme, { body: "# Beta" });
    host.window("store").emit("bannerAct", "reloadDoc");
    await until(host, () => host.markdown(props(host)["doc"])?.text === "# Beta", "reloaded");
  } finally {
    await stop();
  }
});

test("仓库… opens the settings page; the window keeps the selection when closed and reopened", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]);
  const { host, store, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 2);
    await choose(host, "Alpha");
    host.window("store").emit("repos");
    await host.settle();
    assert.deepEqual(host.settings.shows, ["store.repos"]);
    host.window("store").userClose();
    await host.settle();
    await host.command("open").run();
    assert.equal(store.selected, `${OFFICIAL}\na.alpha`);
    assert.equal(props(host)["d-name"], "Alpha");
  } finally {
    await stop();
  }
});
