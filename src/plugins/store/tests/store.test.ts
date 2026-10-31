import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { timing } from "../store.ts";
import { actions, AVAILABLE, banners, BUILTIN, choose, FakeNet, info, INSTALLED, OFFICIAL, openStore, props, publish, rows, start, tab, THIRD, until, UPDATES, yes } from "./helpers.ts";

const path = (host: Awaited<ReturnType<typeof start>>["host"]) => host.command("open").declaration.path;

test("menu Plugin Store… counts updates from the same source only, announces them once", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "zysilm.youtube", name: "YouTube", versions: [{ version: "0.3.0" }, { version: "0.2.0" }] },
    { id: "vestra.video", name: "Video", versions: [{ version: "9.0.0" }] },
  ]);
  publish(net, THIRD, [{ id: "someone.x", name: "X", versions: [{ version: "5.0.0" }] }]);
  const { host, store, stop } = await start({
    net,
    packages: [
      BUILTIN,
      info("zysilm.youtube", { version: "0.2.0", source: { kind: "repository", url: OFFICIAL } }),
      // Installed from the official repository, but only the third-party one has a newer version.
      info("someone.x", { version: "1.0.0", source: { kind: "repository", url: OFFICIAL } }),
    ],
  });
  try {
    const command = host.command("open");
    assert.deepEqual(command.declaration.path, ["Plugin Store…"]);
    assert.deepEqual(command.display.path, ["插件商城…"]);
    assert.equal(command.declaration.order, 822);
    assert.equal(command.declaration.group, "windows");
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    await until(host, () => path(host)?.[0] !== "Plugin Store…", "menu title");
    assert.deepEqual(host.command("open").display.path, ["插件商城（1 个可更新）…"]);
    assert.deepEqual(host.osd, ["插件商城：1 个插件可以更新"]);
    await store.refreshAll(true);
    await host.settle(20);
    assert.equal(host.osd.length, 1, "announced once per run");

    await openStore(host);
    await tab(host, UPDATES);
    assert.deepEqual(rows(host).map((r) => r.name), ["YouTube"]);
    await choose(host, "YouTube");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => path(host)?.[0] === "Plugin Store…", "count gone after the update");
    await until(host, () => !store.busy);
    // The updated plugin left Updates; the tab stays, empty, without details.
    assert.equal(props(host)["category"], UPDATES);
    assert.equal(props(host)["empty-text"], "所有插件都是最新的");
    assert.equal(props(host)["has-selection"], false);
  } finally {
    await stop();
  }
});

test("bottom-bar button: store icon at 200, the menu title as tooltip, a click opens the store", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "zysilm.youtube", name: "YouTube", versions: [{ version: "0.3.0" }] }]);
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, info("zysilm.youtube", { version: "0.2.0", source: { kind: "repository", url: OFFICIAL } })],
  });
  try {
    assert.deepEqual(host.command("open").declaration.button, { icon: "store", order: 200 });
    assert.equal(host.barButtons.length, 1);
    const [button] = host.barButtons;
    assert.equal(button?.command, "open");
    assert.equal(button?.icon, "store");
    assert.equal(button?.image, false);
    assert.equal(button?.order, 200);
    assert.equal(button?.enabled, true, "the command has no disabled or busy state");
    assert.equal(button?.checked, false);
    assert.equal(button?.tooltip, "插件商城", "the menu title without its …");

    await button?.click();
    assert.equal(host.window("store").visible, true, "opens like the menu item");

    // The update count re-registers the command; the button follows its title.
    await store.refreshAll(true);
    await until(host, () => path(host)?.[0] !== "Plugin Store…", "menu title");
    assert.equal(host.barButtons.length, 1);
    assert.equal(host.barButtons[0]?.tooltip, "插件商城（1 个可更新）");
    assert.equal(host.barButtons[0]?.order, 200);
  } finally {
    await stop();
  }
});

test("automatic check: once after activation, then daily; off means no network", async () => {
  // Real timers stay short, but nothing depends on how fast this machine is:
  // the first check is bounded from below only, "a day later" is an injected
  // clock, and every positive wait polls the observable request count.
  const defaults = { ...timing };
  const DAY = 86_400_000;
  let clock = 1_000; // far less than a day since lastCheck = 0: only the first-check timer may fire
  const now = () => clock;
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  Object.assign(timing, { firstCheck: 300, tick: 10, interval: DAY });
  const data = mkdtempSync(join(tmpdir(), "vestra-store-autocheck-"));
  try {
    const activated = Date.now();
    const first = await start({ net, now });
    try {
      if (Date.now() - activated < timing.firstCheck) assert.equal(net.count(OFFICIAL), 0, "not during activation");
      await until(first.host, () => net.count(OFFICIAL) === 1, "first check");
      assert.ok(Date.now() - activated >= timing.firstCheck, "not before the first-check delay");
      await until(first.host, () => first.store.settings.value.lastCheck === 1_000, "lastCheck saved");
      await first.host.settle(50);
      assert.equal(net.count(OFFICIAL), 1, "once, not again within the day");
      clock += DAY;
      await until(first.host, () => net.count(OFFICIAL) === 2, "next check a day later");
    } finally {
      await first.stop();
    }

    // Turned off before the timers exist, so no first check can win a race with the save.
    Object.assign(timing, defaults);
    const quiet = new FakeNet();
    publish(quiet, OFFICIAL, []);
    const off = await start({ net: quiet, dataDirectory: data });
    off.store.settings.save({ ...off.store.settings.value, autoCheck: false });
    await off.stop();
    Object.assign(timing, { firstCheck: 30, tick: 10, interval: DAY });
    clock = 10 * DAY; // the daily check is due too
    const second = await start({ net: quiet, now, dataDirectory: data });
    try {
      assert.equal(second.store.settings.value.autoCheck, false);
      await second.host.settle(300);
      assert.equal(quiet.count(OFFICIAL), 0);
    } finally {
      await second.stop();
    }
  } finally {
    Object.assign(timing, defaults);
    rmSync(data, { recursive: true, force: true });
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
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, info("b.beta", { version: "1.0.0", source: { kind: "repository", url: OFFICIAL } })],
  });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 5);
    const window = host.window("store");
    // Installed (the first tab): built-ins first, then third-party; tags of this tab only.
    assert.equal(props(host)["category"], INSTALLED);
    assert.deepEqual(rows(host).map((r) => [r.name, r.badge]), [
      ["vestra.video", "内置"],
      ["Beta", "已安装"],
    ]);
    // The first entry is "Tags: All", translated by the plugin.
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "字幕"]);
    assert.equal(props(host)["installed-count"], 2);
    assert.equal(props(host)["available-count"], 3);
    assert.equal(props(host)["available-known"], true);
    await tab(host, AVAILABLE);
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha", "Delta", "Gamma"]);
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "网络视频"]);
    window.emit("setTag", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha"]);
    window.emit("setTag", 0);
    window.emit("search", "ANN");
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha"]);
    window.emit("search", "zzz");
    await host.settle();
    assert.equal(props(host)["empty-text"], "没有符合条件的插件");
    window.emit("search", "");
    window.emit("setSort", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta", "Gamma", "Alpha"]);

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
    await until(host, () => store.items.length === 4);
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
    // The banner goes as soon as the reload starts, before the README is back: wait for both.
    const images = "![a](https://img.test/a.png) ![b](https://img.test/b.png)";
    await until(
      host,
      () => !banners(host).some((b) => b.startsWith("已阻止")) && host.markdown(props(host)["doc"])?.text === images,
      "images allowed",
    );

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
    await until(host, () => store.items.length === 2);
    await choose(host, "Alpha");
    // ⚙ ▸ Repositories… and ⚙ ▸ Store Settings… both open the store's page in Options.
    host.window("store").emit("repos");
    host.window("store").emit("settings");
    await host.settle();
    assert.deepEqual(host.settings.shows, ["store.repos", "store.repos"]);
    host.window("store").userClose();
    await host.settle();
    await host.command("open").run();
    assert.equal(store.selected, `${OFFICIAL}\na.alpha`);
    assert.equal(props(host)["d-name"], "Alpha");
  } finally {
    await stop();
  }
});

test("review 4: third-party README and icon on another host are not fetched until asked", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  publish(net, THIRD, [
    { id: "c.same", name: "Same", iconUrl: "https://third.example/icons/same.png", versions: [{ version: "1.0.0" }] },
    { id: "c.far", name: "Far", iconUrl: "https://tracker.test/i.png", versions: [{ version: "1.0.0" }] },
  ]);
  // Point Far's README at another host.
  const index = JSON.parse(String((net.routes.get(THIRD) as { body: Buffer }).body));
  index.plugins[1].readmeUrl = "https://tracker.test/readme.md";
  net.route(THIRD, { body: JSON.stringify(index) });
  net.route("https://tracker.test/readme.md", { body: "# Far" });
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => store.items.length === 3);
    await choose(host, "Same");
    await until(host, () => net.count("https://third.example/icons/same.png") === 1, "same-host icon fetched");
    await choose(host, "Far");
    await until(host, () => banners(host).includes("说明位于其它网站（tracker.test），没有自动加载"));
    assert.equal(net.count("https://tracker.test/readme.md"), 0);
    assert.equal(net.count("https://tracker.test/i.png"), 0);
    host.window("store").emit("bannerAct", "showImages");
    await until(host, () => host.markdown(props(host)["doc"])?.text === "# Far", "loaded after asking");
    assert.equal(net.count("https://tracker.test/i.png"), 0, "the icon never");
  } finally {
    await stop();
  }
});
