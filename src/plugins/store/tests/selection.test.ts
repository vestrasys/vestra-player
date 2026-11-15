/**
 * The detail page (Store v3, design store-v3 §3.5): a click on a card opens
 * its page in the same window, on About; ← All Plugins (or Esc) goes back to
 * the grid, which kept its tab, search, category and scroll. The grid has no
 * selection. The page follows its plugin through install, update and
 * uninstall; a plugin that leaves the store's list closes it. `select(key)`
 * (after an install from a file or a link, from the revocation notice) opens
 * a page too, once the plugin is listed.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { ALL, BUILTIN, choose, controls, FakeNet, info, INSTALLED, OFFICIAL, openStore, props, publish, rows, showBuiltins, start, tab, until, UPDATES, yes } from "./helpers.ts";

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

/** Installed: the built-in, Alpha, Old (an update), Side (from a file); not installed: Beta, Gamma, Delta. */
const PLUGINS = [
  { id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] },
  { id: "o.old", name: "Old", versions: [{ version: "2.0.0" }, { version: "1.0.0" }] },
  { id: "b.beta", name: "Beta", tags: ["video"], versions: [{ version: "1.0.0" }] },
  { id: "g.gamma", name: "Gamma", tags: ["audio"], versions: [{ version: "1.0.0" }] },
  { id: "d.delta", name: "Delta", tags: ["audio"], versions: [{ version: "1.0.0" }] },
];

async function ready() {
  const net = new FakeNet();
  publish(net, OFFICIAL, PLUGINS);
  const started = await start({
    net,
    packages: [
      BUILTIN,
      info("a.alpha", { version: "1.0.0", source: fromRepo }),
      info("o.old", { version: "1.0.0", source: fromRepo }),
      info("x.side", { version: "1.0.0", source: { kind: "file" } }),
    ],
  });
  await openStore(started.host);
  await showBuiltins(started.host);
  await until(started.host, () => started.store.items.length === 7, "cards");
  return started;
}

type Host = Awaited<ReturnType<typeof start>>["host"];

/** The open page's plugin, or null on the grid. */
const page = (host: Host) => (props(host)["detail-open"] ? (props(host)["d-name"] as string) : null);
const keyOf = (id: string) => `${OFFICIAL}\n${id}`;

test("a click on a card opens its page on About; back returns to the grid as it was", async () => {
  const { host, store, stop } = await ready();
  try {
    const window = host.window("store");
    // The grid: every plugin, nothing selected.
    assert.equal(props(host)["category"], ALL);
    assert.deepEqual(rows(host).map((r) => r.name), ["Alpha", "Beta", "Delta", "Gamma", "Old", "vestra.video", "x.side"]);
    assert.equal(page(host), null);
    window.emit("search", "a");
    window.emit("setGoal", 2);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta", "Gamma"]);
    window.emit("setTab", 1);
    await choose(host, "Gamma");
    assert.equal(page(host), "Gamma");
    assert.equal(props(host)["tab"], 0, "a page opens on About");
    assert.equal(store.selected, keyOf("g.gamma"));
    // Back: the grid with its tab, search and category.
    window.emit("back");
    await host.settle();
    assert.equal(page(host), null);
    assert.equal(store.selected, null);
    assert.equal(store.search, "a");
    assert.equal(props(host)["goal-index"], 2);
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta", "Gamma"]);
    // A card the grid no longer has opens nothing.
    window.emit("openCard", `${OFFICIAL}\nz.none`);
    await host.settle();
    assert.equal(page(host), null);
  } finally {
    await stop();
  }
});

test("the page follows its plugin through install, update and uninstall; a plugin that leaves the list closes it", async () => {
  const { host, net, store, stop } = await ready();
  try {
    const window = host.window("store");
    await choose(host, "Delta");
    host.dialogAnswers.push(yes);
    window.emit("act", "install");
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "d.delta"), "installed");
    await host.settle();
    assert.equal(page(host), "Delta");
    assert.equal(controls(host, "Delta").state, "✓ 已安装");

    await tab(host, UPDATES);
    await choose(host, "Old");
    host.dialogAnswers.push(yes);
    window.emit("act", "update");
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "o.old" && p.version === "2.0.0"), "updated");
    await host.settle();
    // It left Updates; its page stays.
    assert.equal(props(host)["category"], UPDATES);
    assert.deepEqual(rows(host), []);
    assert.equal(page(host), "Old");

    // Uninstalled: its repository's entry stands for it, the page stays with Install.
    await tab(host, INSTALLED);
    await choose(host, "Alpha");
    host.dialogAnswers.push(yes);
    window.emit("act", "uninstall");
    await until(host, () => !store.busy && !host.packages.some((p) => p.id === "a.alpha"), "uninstalled");
    await host.settle();
    assert.equal(page(host), "Alpha");
    assert.deepEqual((props(host)["actions"] as { id: string }[]).map((a) => a.id), ["install"]);

    // A sideloaded plugin uninstalled: nothing stands for it, the grid shows.
    await choose(host, "x.side");
    host.dialogAnswers.push(yes);
    window.emit("act", "uninstall");
    await until(host, () => !store.busy && !host.packages.some((p) => p.id === "x.side"), "side uninstalled");
    await host.settle();
    assert.equal(page(host), null);
    assert.equal(props(host)["category"], INSTALLED, "the tab stays");

    // The repository drops Gamma while its page is open: the grid shows.
    await tab(host, ALL);
    await choose(host, "Gamma");
    publish(net, OFFICIAL, PLUGINS.filter((p) => p.id !== "g.gamma"));
    await store.refreshAll(true);
    await host.settle();
    assert.equal(page(host), null);
    assert.ok(!rows(host).some((r) => r.name === "Gamma"));
  } finally {
    await stop();
  }
});

test("a card's button acts in place: the grid stays", async () => {
  const { host, store, stop } = await ready();
  try {
    const beta = rows(host).find((r) => r.name === "Beta")!;
    assert.deepEqual(beta.button, { id: "install", text: "安装", enabled: true });
    host.dialogAnswers.push(yes);
    host.window("store").emit("cardAct", beta.key, "install");
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "b.beta"), "installed");
    await host.settle();
    assert.equal(page(host), null);
    assert.equal(controls(host, "Beta").state, "✓ 已安装");
    assert.equal(controls(host, "Beta").button, null);
  } finally {
    await stop();
  }
});

test("select(key) opens the page: a repository entry of an installed plugin opens the installed one; nothing on the grid changes", async () => {
  const { host, store, stop } = await ready();
  try {
    const window = host.window("store");
    await tab(host, UPDATES);
    window.emit("search", "zzz");
    await host.settle();
    store.select(keyOf("d.delta"));
    await host.settle();
    assert.equal(page(host), "Delta");
    assert.equal(props(host)["category"], UPDATES);
    assert.equal(store.search, "zzz");
    // A built-in's repository entry: the built-in's own page.
    store.select(keyOf("vestra.video"));
    await host.settle();
    assert.equal(store.selected, keyOf("vestra.video"), "the repository lists no vestra.video here: its key, waiting");
    assert.equal(page(host), null);
    store.select("\nvestra.video");
    await host.settle();
    assert.equal(page(host), "vestra.video");
  } finally {
    await stop();
  }
});

test("a plugin selected before it is in the store's list opens once it arrives", async () => {
  const { host, store, stop } = await ready();
  try {
    store.select("\nx.new");
    await host.settle();
    // Not listed yet: the grid.
    assert.equal(page(host), null);
    host.packages = [...host.packages, info("x.new", { version: "1.0.0" })];
    await store.reread();
    await host.settle();
    assert.equal(store.selected, "\nx.new");
    assert.equal(page(host), "x.new");
    assert.equal(props(host)["category"], ALL, "the tab stays");
  } finally {
    await stop();
  }
});
