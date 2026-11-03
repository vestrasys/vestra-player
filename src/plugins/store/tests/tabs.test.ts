/**
 * The tabs All · Installed · Updates (store 0.2.11, user ruling 2026-11-03,
 * as GNOME Software, KDE Discover, the JetBrains Marketplace and VS Code):
 * All lists every plugin of every repository, installed ones included, and
 * the installed plugins no repository lists, each row with its state and its
 * own buttons. The store remembers the last tab; the first open is All, and a
 * remembered "available" (the tab All replaced) is All.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ALL, BUILTIN, controls, FakeNet, info, INSTALLED, OFFICIAL, openStore, props, publish, rows, start, tab, THIRD, until, UPDATES } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-tabs-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

test("All lists the repositories' plugins, installed or not, and installed plugins from no repository, with their buttons", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] },
    { id: "b.beta", name: "Beta", versions: [{ version: "2.0.0" }, { version: "1.0.0" }] },
    { id: "g.gamma", name: "Gamma", versions: [{ version: "1.0.0" }] },
  ]);
  const { host, store, stop } = await start({
    net,
    packages: [
      BUILTIN,
      info("a.alpha", { version: "1.0.0", source: fromRepo }),
      info("b.beta", { version: "1.0.0", source: fromRepo }),
      // Sideloaded from a .vxp file, and made in the Plugin Workshop (disabled).
      info("s.side", { version: "1.0.0", source: { kind: "file" } }),
      info("w.work", { version: "1.0.0", source: { kind: "workshop" }, state: "disabled" }),
    ],
  });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 6, "rows");
    assert.equal(props(host)["category"], ALL);
    type Row = { name: string; badge: string; meta?: string };
    assert.deepEqual(
      (rows(host) as Row[]).map((r) => [r.name, r.badge, r.meta]),
      [
        ["Alpha", "已安装", "Someone · 官方仓库"],
        ["Beta", "可更新 → 2.0.0", "Someone · 官方仓库"],
        ["Gamma", "", "Someone · 官方仓库"],
        ["s.side", "已安装", "未知作者 · 本地文件"],
        ["w.work", "已停用", "未知作者 · 插件工坊"],
      ],
      "the built-in is hidden (the check box is on by default)",
    );
    assert.equal(props(host)["all-count"], 5);
    assert.equal(props(host)["installed-count"], 4);
    assert.equal(props(host)["updates"], 1);

    // Each row's buttons by its state, in the fixed order Update, Disable or Enable, Uninstall….
    const of = (name: string) => {
      const { primary, menu } = controls(host, name);
      return [...(primary ? [primary] : []), ...menu].map((a) => a.id);
    };
    assert.deepEqual(of("Gamma"), ["install"]);
    assert.deepEqual(of("Alpha"), ["disable", "uninstall", "reload", "openData"]);
    assert.deepEqual(of("Beta"), ["update", "disable", "uninstall", "reload", "openData"]);
    assert.deepEqual(of("s.side"), ["disable", "uninstall", "reload", "openData"]);
    assert.deepEqual(of("w.work"), ["enable", "uninstall", "openData"]);
    assert.equal(controls(host, "w.work").dim, true);

    // Installed and Updates as before.
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha", "Beta", "s.side", "w.work"]);
    await tab(host, UPDATES);
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);

    // Search and the tag filter apply to All as they did to Available.
    await tab(host, ALL);
    host.window("store").emit("search", "side");
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["s.side"]);
    host.window("store").emit("search", "beta");
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);
  } finally {
    await stop();
  }
});

test("the same plugin in two repositories: its installed row and the other repository's entry, as before", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]);
  publish(net, THIRD, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "5.0.0" }] }]);
  const { host, store, stop } = await start({ net, packages: [info("a.alpha", { version: "1.0.0", source: fromRepo })] });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    await openStore(host);
    await until(host, () => rows(host).length === 2, "rows");
    // One row per repository (Installed lists only the installed one, its own repository's, as before).
    type Row = { key: string; badge: string };
    assert.deepEqual(
      (rows(host) as Row[]).map((r) => [r.key, r.badge]).sort(),
      [
        [`${OFFICIAL}\na.alpha`, "已安装"],
        [`${THIRD}\na.alpha`, "已从仓库 plugins.test安装"],
      ].sort(),
    );
    // A newer version in another repository is no update (updates only from the original source).
    assert.equal(props(host)["updates"], 0);
    await tab(host, INSTALLED);
    assert.deepEqual((rows(host) as Row[]).map((r) => r.key), [`${OFFICIAL}\na.alpha`]);
  } finally {
    await stop();
  }
});

test("the tab is remembered; the first open is All; a remembered Available (before 0.2.11) is All", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]);
  const settings = (tab: unknown) => ({ repos: [], autoCheck: false, lastCheck: 0, revocations: {}, tab });
  const run = async (data: string, check: (started: Awaited<ReturnType<typeof start>>) => Promise<void> | void) => {
    const started = await start({ net, dataDirectory: data });
    try {
      await openStore(started.host);
      await check(started);
    } finally {
      await started.stop();
    }
  };

  // No settings yet: All.
  const fresh = join(scratch, "fresh");
  await run(fresh, ({ host }) => assert.equal(props(host)["category"], ALL));

  // An older store's "available": All, and the rest of the file is still read.
  const old = join(scratch, "old");
  mkdirSync(old, { recursive: true });
  writeFileSync(join(old, "settings.json"), JSON.stringify(settings("available")));
  await run(old, async ({ host, store }) => {
    assert.equal(props(host)["category"], ALL);
    assert.equal(store.settings.value.autoCheck, false);
    // Picking a tab keeps it, by name.
    await tab(host, UPDATES);
  });
  assert.equal(JSON.parse(readFileSync(join(old, "settings.json"), "utf8")).tab, "updates");
  await run(old, ({ host }) => assert.equal(props(host)["category"], UPDATES));

  // Installed is kept the same way; a value of the wrong type is All and does not reset the settings.
  writeFileSync(join(old, "settings.json"), JSON.stringify(settings("installed")));
  await run(old, ({ host }) => assert.equal(props(host)["category"], INSTALLED));
  writeFileSync(join(old, "settings.json"), JSON.stringify(settings(7)));
  await run(old, ({ host, store }) => {
    assert.equal(props(host)["category"], ALL);
    assert.equal(store.settings.value.autoCheck, false);
  });

  // A tab index out of range from the window is clamped.
  await run(fresh, async ({ host }) => {
    await tab(host, 9);
    assert.equal(props(host)["category"], UPDATES);
    await tab(host, -1);
    assert.equal(props(host)["category"], ALL);
  });
});

test("offline, no index: All in English shows the installed plugins, their count and the calm note under the list", async () => {
  const net = new FakeNet();
  net.offline = true;
  const { host, store, stop } = await start({
    net,
    locale: "en",
    packages: [BUILTIN, info("s.side", { version: "1.0.0" }), info("w.work", { version: "1.0.0", source: { kind: "workshop" }, state: "disabled" })],
  });
  try {
    await openStore(host);
    await until(host, () => !store.repos.official.refreshing, "gave up");
    assert.equal(props(host)["category"], ALL);
    assert.deepEqual(rows(host).map((r) => r.name), ["s.side", "w.work"]);
    assert.equal(props(host)["all-count"], 2);
    assert.equal(props(host)["list-note"], "Can't reach the repositories; only installed plugins are shown. Click Refresh to retry.");
    assert.deepEqual(controls(host, "s.side").primary?.id, "disable");
  } finally {
    await stop();
  }
});
