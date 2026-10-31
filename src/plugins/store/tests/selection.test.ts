/**
 * The details follow the list (GUI test 2026-10-27: Available showed the
 * Installed tab's built-in in its details): a tab, search or tag change keeps
 * the selection while the list shows it, else selects the list's first row,
 * and an empty list has no details. A plugin that left the list (installed,
 * updated, uninstalled, dropped) gives way to the row in its place; the tab stays.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { AVAILABLE, BUILTIN, choose, FakeNet, info, INSTALLED, OFFICIAL, openStore, props, publish, rows, start, tab, until, UPDATES, yes } from "./helpers.ts";

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

/** Installed: the built-in, Alpha, Old (an update); Available: Beta, Gamma (tagged), Delta. */
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
    ],
  });
  await openStore(started.host);
  await until(started.host, () => started.store.items.length === 6, "rows");
  return started;
}

type Host = Awaited<ReturnType<typeof start>>["host"];

/** What the details show: the name, or null for none; checked against the list. */
function details(host: Host): string | null {
  const p = props(host);
  const list = rows(host) as { name: string; selected?: boolean }[];
  const marked = list.filter((r) => r.selected).map((r) => r.name);
  if (!p["has-selection"]) {
    assert.deepEqual(marked, [], "no row marked without details");
    assert.equal(p["selected-index"], -1);
    return null;
  }
  // The details are those of the row marked selected, at the index the window scrolls to.
  assert.deepEqual(marked, [p["d-name"]]);
  assert.equal(list[p["selected-index"] as number]?.name, p["d-name"]);
  return p["d-name"] as string;
}

const search = async (host: Host, text: string) => {
  host.window("store").emit("search", text);
  await host.settle();
};

test("a tab switch selects the new list's first row, or keeps the selection that it lists", async () => {
  const { host, store, stop } = await ready();
  try {
    assert.equal(details(host), "vestra.video");
    await tab(host, AVAILABLE);
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta", "Delta", "Gamma"]);
    assert.equal(details(host), "Beta");
    await choose(host, "Gamma");
    await tab(host, UPDATES);
    assert.equal(details(host), "Old");
    // Old is listed under Installed too: it stays selected.
    await tab(host, INSTALLED);
    assert.equal(details(host), "Old");
    await tab(host, AVAILABLE);
    assert.equal(details(host), "Beta");
    // A refresh that keeps the selection leaves the list where the user scrolled it.
    const scroll = props(host)["scroll-request"];
    await store.refreshAll(true);
    await host.settle();
    assert.equal(details(host), "Beta");
    assert.equal(props(host)["scroll-request"], scroll);
  } finally {
    await stop();
  }
});

test("a search or tag that hides the selected plugin selects the first row it leaves; none left: no details", async () => {
  const { host, store, stop } = await ready();
  try {
    await tab(host, AVAILABLE);
    await choose(host, "Gamma");
    const before = props(host)["scroll-request"] as number;
    // Still listed: kept, and brought into view.
    await search(host, "a");
    assert.equal(details(host), "Gamma");
    assert.ok((props(host)["scroll-request"] as number) > before);
    await search(host, "delta");
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta"]);
    assert.equal(details(host), "Delta");
    await search(host, "nothing like it");
    assert.deepEqual(rows(host), []);
    assert.equal(details(host), null);
    assert.equal(store.selected, null);
    await search(host, "");
    assert.equal(details(host), "Beta");
    // Tags: All, audio, video.
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "audio", "video"]);
    host.window("store").emit("setTag", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta", "Gamma"]);
    assert.equal(details(host), "Delta");
    await choose(host, "Gamma");
    host.window("store").emit("setTag", 0);
    await host.settle();
    assert.equal(details(host), "Gamma");
  } finally {
    await stop();
  }
});

test("an empty tab has no details; the next tab selects its first row again", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "b.beta", name: "Beta", versions: [{ version: "1.0.0" }] }]);
  const { host, store, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => store.items.length === 2, "rows");
    assert.equal(details(host), "vestra.video");
    await tab(host, UPDATES);
    assert.deepEqual(rows(host), []);
    assert.equal(details(host), null);
    assert.equal(props(host)["empty-text"], "所有插件都是最新的");
    await tab(host, AVAILABLE);
    assert.equal(details(host), "Beta");
  } finally {
    await stop();
  }
});

test("install, update, uninstall and a repository dropping a plugin keep the tab; the row in its place is selected", async () => {
  const { host, net, store, stop } = await ready();
  try {
    await tab(host, AVAILABLE);
    await choose(host, "Delta");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "d.delta"), "installed");
    await host.settle();
    assert.equal(props(host)["category"], AVAILABLE);
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta", "Gamma"]);
    assert.equal(details(host), "Gamma");
    // The last update under Updates: the tab stays, now empty, without details.
    await tab(host, UPDATES);
    assert.equal(details(host), "Old");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "o.old" && p.version === "2.0.0"), "updated");
    await host.settle();
    assert.equal(props(host)["category"], UPDATES);
    assert.deepEqual(rows(host), []);
    assert.equal(details(host), null);
    // The repository drops Gamma while it is selected under Available.
    await tab(host, AVAILABLE);
    await choose(host, "Gamma");
    publish(net, OFFICIAL, PLUGINS.filter((p) => p.id !== "g.gamma"));
    await store.refreshAll(true);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);
    assert.equal(details(host), "Beta");
    // Uninstalled under Installed: the row below takes its place.
    await tab(host, INSTALLED);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video", "Alpha", "Delta", "Old"]);
    await choose(host, "Alpha");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "uninstall");
    await until(host, () => !store.busy && !host.packages.some((p) => p.id === "a.alpha"), "uninstalled");
    await host.settle();
    assert.equal(props(host)["category"], INSTALLED);
    assert.equal(details(host), "Delta");
  } finally {
    await stop();
  }
});

const keyOf = (id: string) => `${OFFICIAL}\n${id}`;
const scroll = (host: Host) => props(host)["scroll-request"] as number;

test("select(key) shows the plugin: its tab, then without the tag, then without the search", async () => {
  const { host, store, stop } = await ready();
  try {
    await tab(host, AVAILABLE);
    let before = scroll(host);
    store.select(keyOf("o.old"));
    await host.settle();
    assert.equal(props(host)["category"], INSTALLED);
    assert.equal(details(host), "Old");
    assert.ok(scroll(host) > before);
    // A tag that hides it is dropped.
    await tab(host, AVAILABLE);
    host.window("store").emit("setTag", 1);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Delta", "Gamma"]);
    store.select(keyOf("b.beta"));
    await host.settle();
    assert.equal(props(host)["category"], AVAILABLE);
    assert.equal(props(host)["tag-index"], 0);
    assert.equal(details(host), "Beta");
    // A search that hides it is cleared, in the window's box too.
    await search(host, "gamma");
    assert.equal(details(host), "Gamma");
    const cleared = props(host)["clear-search"] as number;
    store.select(keyOf("b.beta"));
    await host.settle();
    assert.equal(props(host)["clear-search"], cleared + 1);
    assert.equal(store.search, "");
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta", "Delta", "Gamma"]);
    assert.equal(details(host), "Beta");
    // Sorting keeps the selection and brings it into view.
    before = scroll(host);
    host.window("store").emit("setSort", 1);
    await host.settle();
    assert.equal(details(host), "Beta");
    assert.ok(scroll(host) > before);
  } finally {
    await stop();
  }
});

test("a plugin selected before it is in the plugin list is shown and scrolled to once it arrives", async () => {
  const { host, store, stop } = await ready();
  try {
    await tab(host, AVAILABLE);
    const before = scroll(host);
    store.select("\nx.side");
    await host.settle();
    // Not listed yet: no details, and the list does not move.
    assert.equal(details(host), null);
    assert.equal(scroll(host), before);
    host.packages = [...host.packages, info("x.side", { version: "1.0.0" })];
    await store.reread();
    await host.settle();
    assert.equal(props(host)["category"], INSTALLED);
    assert.equal(store.selected, "\nx.side");
    assert.equal(details(host), "x.side");
    assert.ok(scroll(host) > before);
  } finally {
    await stop();
  }
});

test("an empty list that fills selects its first row; a tab without the chosen tag drops it", async () => {
  const { host, net, store, stop } = await ready();
  try {
    await tab(host, AVAILABLE);
    await search(host, "zeta");
    assert.deepEqual(rows(host), []);
    assert.equal(details(host), null);
    publish(net, OFFICIAL, [...PLUGINS, { id: "z.zeta", name: "Zeta", versions: [{ version: "1.0.0" }] }]);
    await store.refreshAll(true);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Zeta"]);
    assert.equal(details(host), "Zeta");
    await search(host, "");
    // "video" (Beta) has no plugin under Installed: Installed shows all its plugins under "Tags: All".
    host.window("store").emit("setTag", 2);
    await host.settle();
    assert.deepEqual(rows(host).map((r) => r.name), ["Beta"]);
    await tab(host, INSTALLED);
    assert.equal(props(host)["tag-index"], 0);
    assert.equal(store.tag, null);
    assert.deepEqual(rows(host).map((r) => r.name), ["vestra.video", "Alpha", "Old"]);
    assert.equal(details(host), "vestra.video");
  } finally {
    await stop();
  }
});
