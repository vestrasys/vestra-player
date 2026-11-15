/**
 * The tabs All · Installed · Updates (store 0.2.11, user ruling 2026-11-03,
 * as GNOME Software, KDE Discover, the JetBrains Marketplace and VS Code):
 * All lists every plugin of every repository, installed ones included, and
 * the installed plugins no repository lists, each card with its state and
 * its button (Store v3), the rest on its page. The store remembers the last tab; the first open is All, and a
 * remembered "available" (the tab All replaced) is All.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ALL, BUILTIN, choose, controls, FakeNet, info, type Info, INSTALLED, manage, more, OFFICIAL, openStore, props, publish, rows, start, tab, THIRD, until, UPDATES, yes } from "./helpers.ts";

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
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.state, r.by, r.byNote]),
      [
        ["Alpha", "✓ 已安装", "作者：Someone", ""],
        ["Beta", "有更新", "作者：Someone", ""],
        ["Gamma", "", "作者：Someone", ""],
        ["s.side", "✓ 已安装", "作者：未知作者", "· 本地文件"],
        ["w.work", "已停用", "作者：未知作者", "· 插件工坊"],
      ],
      "the built-in is hidden (\"Show built-in plugins\" is off by default)",
    );
    assert.equal(props(host)["all-count"], 5);
    assert.equal(props(host)["installed-count"], 4);
    assert.equal(props(host)["updates"], 1);

    // Each card's one button by its state (Install, Update, Enable, else none); its page has the rest in the fixed order.
    const of = (name: string) => controls(host, name).button?.id ?? null;
    assert.deepEqual(["Gamma", "Alpha", "Beta", "s.side", "w.work"].map(of), ["install", null, "update", null, "enable"]);
    const page = async (name: string) => {
      const { buttons, more } = await manage(host, name);
      return [...buttons, ...more].map((a) => a.id);
    };
    assert.deepEqual(await page("Alpha"), ["disable", "uninstall", "reload", "openData"]);
    assert.deepEqual(await page("s.side"), ["disable", "uninstall", "reload", "openData"]);
    assert.deepEqual(await page("w.work"), ["enable", "uninstall", "openData"]);
    assert.equal(controls(host, "w.work").dim, true);

    // Installed and Updates as before.
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha", "Beta", "s.side", "w.work"]);
    await tab(host, UPDATES);
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);

    // Search applies to All as it did to Available.
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

test("the same plugin in two repositories: one card under All when installed, one per repository when not", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] },
    { id: "b.beta", name: "Beta", versions: [{ version: "1.0.0" }] },
  ]);
  publish(net, THIRD, [
    { id: "a.alpha", name: "Alpha", versions: [{ version: "5.0.0" }] },
    { id: "b.beta", name: "Beta", versions: [{ version: "2.0.0" }] },
  ]);
  const { host, store, stop } = await start({ net, packages: [info("a.alpha", { version: "1.0.0", source: fromRepo })] });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    await openStore(host);
    await until(host, () => store.items.length === 4, "rows");
    // Alpha (installed from the official repository): its installed card only. Beta (not installed): one card per repository.
    assert.deepEqual(
      rows(host).map((r) => [r.key, r.state, r.byNote]).sort(),
      [
        [`${OFFICIAL}\na.alpha`, "✓ 已安装", ""],
        [`${OFFICIAL}\nb.beta`, "", ""],
        [`${THIRD}\nb.beta`, "", "· 非官方"],
      ].sort(),
    );
    assert.equal(props(host)["all-count"], 3);
    // The other repository's version is replaced from the page's ⋯ menu (a newer version there is no update: updates only from the original source).
    assert.equal(props(host)["updates"], 0);
    await choose(host, "Alpha");
    const replace = more(host).at(-1)!;
    assert.equal(replace.id, `replace:${THIRD}\na.alpha`);
    assert.match(String(replace.text), /^用.+的 5\.0\.0 替换…$/);
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.key), [`${OFFICIAL}\na.alpha`]);
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
    assert.deepEqual((await manage(host, "s.side")).buttons[0]?.id, "disable");
  } finally {
    await stop();
  }
});

test("All in English: one button per card (Install, Update) and the state in words; a .vxp install a repository lists too is one card", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "t.tube", name: "Tube", versions: [{ version: "2.0.0" }] },
    { id: "g.gamma", name: "Gamma", versions: [{ version: "1.0.0" }] },
    { id: "n.newer", name: "Newer", versions: [{ version: "1.4.0" }, { version: "1.3.0" }] },
    { id: "q.quiet", name: "Quiet", versions: [{ version: "0.9.0" }] },
  ]);
  const tube = {
    ...info("t.tube", { version: "1.0.0", source: { kind: "file" } }),
    display: { name: "Tube", punchline: "Plays tubes", description: null, tags: [] },
  } as Info;
  const { host, store, stop } = await start({
    net,
    locale: "en",
    packages: [
      BUILTIN,
      tube,
      info("n.newer", { version: "1.3.0", source: fromRepo }),
      info("q.quiet", { version: "0.9.0", source: fromRepo, state: "disabled" }),
    ],
  });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 6, "rows");
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.state, r.by, r.byNote, r.intro, r.dim]),
      [
        ["Gamma", "", "by Someone", "", "Gamma 的简介", false],
        ["Newer", "Update available", "by Someone", "", "Newer 的简介", false],
        ["Quiet", "Disabled", "by Someone", "", "Quiet 的简介", true],
        ["Tube", "✓ Installed", "by Unknown author", "· Local file", "Plays tubes", false],
      ],
    );
    const one = (name: string) => controls(host, name).button?.text ?? "";
    assert.deepEqual(["Gamma", "Newer", "Quiet", "Tube"].map(one), ["Install", "Update", "Enable", ""]);
    assert.deepEqual(rows(host).find((r) => r.name === "Tube")!.menu.map((m) => m.text), ["Disable", "Uninstall…"]);
    assert.equal(props(host)["all-count"], 4);
    host.window("store").emit("openCard", "\nt.tube");
    await host.settle();
    assert.ok((props(host)["banners"] as { text: string }[]).some((b) => b.text === "Also in Official repository"));
    assert.equal(more(host).at(-1)?.text, "Replace with 2.0.0 from Official repository…");
  } finally {
    await stop();
  }
});

test("a development load (--plugin) folds nothing: the repository's entry of its id stays under All and installs", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "t.dev", name: "Dev Tool", versions: [{ version: "1.0.0" }] }]);
  const dev = info("t.dev", { origin: "development", source: null });
  const { host, store, stop } = await start({ net, packages: [BUILTIN, dev] });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 3, "rows");
    const all = rows(host);
    assert.deepEqual(all.map((r) => r.key).sort(), ["\nt.dev", `${OFFICIAL}\nt.dev`].sort());
    assert.equal(props(host)["all-count"], 2);
    const entry = all.find((r) => r.key === `${OFFICIAL}\nt.dev`)!;
    assert.equal(entry.button.id, "install");
    // Asked for by its key, the entry itself is shown (not the development row).
    store.select(entry.key);
    await host.settle();
    assert.equal(store.selected, entry.key);
    // No "Also in" and no replace entry on the development row's page.
    await choose(host, "t.dev");
    assert.ok(!more(host).some((m) => m.id.startsWith("replace:")));
    // The card's Install: the install question, then (as before 0.2.11) the store's question about replacing the loaded copy.
    host.dialogAnswers.push(yes, yes);
    host.window("store").emit("cardAct", entry.key, "install");
    await until(host, () => host.packageCalls.some((c) => c.method === "install"), "installed from the repository");
  } finally {
    await stop();
  }
});

test("an update card offers only Update; another repository's version is replaced from its page's ⋯ menu", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "2.0.0" }, { version: "1.0.0" }] }]);
  publish(net, THIRD, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "5.0.0" }] }]);
  const { host, store, stop } = await start({ net, packages: [info("a.alpha", { version: "1.0.0", source: fromRepo })] });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    // Opened first: an update count re-registers the menu command.
    await openStore(host);
    await store.refreshAll(true);
    await until(host, () => rows(host).length === 1, "rows");
    assert.deepEqual(controls(host, "Alpha").button?.id, "update");
    host.window("store").emit("openCard", rows(host)[0]!.key);
    await host.settle();
    const replace = `replace:${THIRD}\na.alpha`;
    assert.deepEqual(
      (props(host)["actions"] as { id: string }[]).map((a) => a.id),
      ["update", "disable", "uninstall", "reload"],
    );
    assert.deepEqual(more(host).map((m) => m.id), ["openData", replace]);
    // Its entry asks as Replace… did.
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", replace);
    await until(host, () => host.packageCalls.some((c) => c.method === "install"), "replaced");
    assert.deepEqual(host.packageCalls.find((c) => c.method === "install")?.source, { kind: "repository", url: THIRD });
  } finally {
    await stop();
  }
});
