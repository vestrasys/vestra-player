/**
 * "Hide built-in plugins" (store 0.2.8; Store v3: "Show built-in plugins" in
 * the ⋯ menu, its opposite): on by default, also for settings saved before
 * it; built-ins are left out of every tab (their own Installed cards and a
 * repository's entries for them) and of the tab titles' counts. Search and
 * category combine with it, an open detail page stays, and the choice is
 * kept in settings.json.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ALL, BUILTIN, FakeNet, info, type Info, INSTALLED, NOT_INSTALLED, OFFICIAL, openStore, props, publish, rows, showBuiltins, start, tab, until } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-builtins-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

/**
 * Installed: the built-in, Alpha, Old (an update). Not installed: Beta, the
 * repository's entry for the built-in ("Video"; All shows the built-in's own
 * row for it, tagged "core" by the player), and vestra.other, a reserved id
 * the player does not ship (not a built-in).
 */
const PLUGINS = [
  { id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] },
  { id: "o.old", name: "Old", versions: [{ version: "2.0.0" }, { version: "1.0.0" }] },
  { id: "b.beta", name: "Beta", tags: ["audio"], versions: [{ version: "1.0.0" }] },
  { id: "vestra.video", name: "Video", tags: ["core"], versions: [{ version: "9.0.0" }] },
  { id: "vestra.other", name: "Other", versions: [{ version: "1.0.0" }] },
];

const BUILTIN_CORE = { ...BUILTIN, display: { name: "vestra.video", punchline: null, description: null, tags: ["core"] } } as Info;

async function ready(dataDirectory?: string) {
  const net = new FakeNet();
  publish(net, OFFICIAL, PLUGINS);
  const started = await start({
    net,
    packages: [BUILTIN_CORE, info("a.alpha", { version: "1.0.0", source: fromRepo }), info("o.old", { version: "1.0.0", source: fromRepo })],
    ...(dataDirectory ? { dataDirectory } : {}),
  });
  await openStore(started.host);
  await until(started.host, () => started.store.items.length === 6, "rows");
  return started;
}

type Host = Awaited<ReturnType<typeof start>>["host"];

const names = (host: Host) => rows(host).map((r) => r.name);
const details = (host: Host) => (props(host)["detail-open"] ? (props(host)["d-name"] as string) : null);
const hide = async (host: Host, on: boolean) => {
  host.window("store").emit("setHideBuiltins", on);
  await host.settle();
};

test("on by default: built-ins are left out of the lists, the counts and the tags; unticking shows them", async () => {
  const { host, store, stop } = await ready();
  try {
    assert.equal(props(host)["hide-builtins"], true);
    assert.equal(props(host)["category"], ALL);
    // All: the built-in's own row and the repository's entry for it go; a reserved id the player does not ship stays.
    assert.deepEqual(names(host), ["Alpha", "Beta", "Old", "Other"]);
    assert.equal(details(host), null, "the grid: no plugin is selected");
    // The tab titles count what the tabs list.
    assert.equal(props(host)["all-count"], 4);
    assert.equal(props(host)["installed-count"], 2);
    await tab(host, INSTALLED);
    assert.deepEqual(names(host), ["Alpha", "Old"]);
    await tab(host, ALL);
    await tab(host, INSTALLED);
    assert.deepEqual(names(host), ["Alpha", "Old"]);
    // Not installed: never a built-in's id, whichever way the switch is (checked again below).
    await tab(host, NOT_INSTALLED);
    assert.deepEqual(names(host), ["Beta", "Other"]);
    assert.equal(props(host)["not-installed-count"], 2);
    await tab(host, ALL);

    await showBuiltins(host);
    assert.equal(props(host)["hide-builtins"], false);
    assert.equal(store.settings.value.hideBuiltins, false);
    // One row per built-in id: its own row stands for the repository's entry ("Video").
    assert.deepEqual(names(host), ["Alpha", "Beta", "Old", "Other", "vestra.video"]);
    assert.equal(props(host)["all-count"], 5);
    await tab(host, INSTALLED);
    // As before the option: built-ins first under Installed.
    assert.deepEqual(names(host), ["vestra.video", "Alpha", "Old"]);
    assert.equal(props(host)["installed-count"], 3);
    // The update count (the menu, the Updates tab) never counted built-ins.
    assert.equal(props(host)["updates"], 1);
    await tab(host, NOT_INSTALLED);
    assert.deepEqual(names(host), ["Beta", "Other"]);
    assert.equal(props(host)["not-installed-count"], 2);
  } finally {
    await stop();
  }
});

test("every plugin the player lists as built in is one; a built-in's id loaded with --plugin is a development load, not a built-in", async () => {
  // The player's packages.list: a bundled built-in is `origin: "installed", builtin: true`; a
  // development load (`--plugin`) of the same id, while it runs, is `origin: "development",
  // builtin: false` (host package_list: a development load is never the installation's built-in).
  const bundled = (id: string) => info(id, { version: "1.0.0", builtin: true, source: { kind: "bundled" } });
  const dev = (id: string) => info(id, { version: "1.0.0", origin: "development", source: null, directory: `/src/plugins/${id}` });
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "b.beta", name: "Beta", versions: [{ version: "1.0.0" }] }]);
  const started = await start({
    net,
    locale: "en",
    packages: [bundled("vestra.audio"), bundled("vestra.marks"), bundled("vestra.video"), dev("vestra.window"), dev("vestra.history")],
  });
  const { host, store, stop } = started;
  try {
    await openStore(host);
    await until(host, () => store.items.length === 6, "rows");
    // Hidden: every bundled built-in goes; the development loads stay, as the development loads they are.
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.state]),
      [
        ["Beta", ""],
        ["vestra.history", "Development load"],
        ["vestra.window", "Development load"],
      ],
    );
    assert.equal(props(host)["builtin-count"], 3);
    await showBuiltins(host);
    await tab(host, INSTALLED);
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.state]),
      [
        ["vestra.audio", "Comes with the player"],
        ["vestra.marks", "Comes with the player"],
        ["vestra.video", "Comes with the player"],
        ["vestra.history", "Development load"],
        ["vestra.window", "Development load"],
      ],
    );
  } finally {
    await stop();
  }
});

test("hiding built-ins leaves an open built-in's page open; a search only built-ins match empties the grid", async () => {
  const { host, stop } = await ready();
  try {
    await tab(host, INSTALLED);
    await showBuiltins(host);
    host.window("store").emit("openCard", "\nvestra.video");
    await host.settle();
    assert.equal(details(host), "vestra.video");
    await hide(host, true);
    assert.deepEqual(names(host), ["Alpha", "Old"]);
    assert.equal(details(host), "vestra.video", "the page stays");
    host.window("store").emit("back");
    await host.settle();
    assert.equal(details(host), null);

    // Only built-ins matched the search: nothing left, Clear Search and Show Built-in Plugins offered (store-v4 §2);
    // showing them fills the grid again.
    host.window("store").emit("search", "vestra");
    await host.settle();
    assert.deepEqual(names(host), []);
    assert.equal(props(host)["empty-text"], "没有符合条件的插件\n\n有 1 个内置插件符合条件，但内置插件已隐藏");
    assert.equal(props(host)["can-clear"], true);
    assert.equal(props(host)["can-show-builtins"], true);
    // "Show built-in plugins (n)" counts the built-in plugins whatever the search.
    assert.equal(props(host)["builtin-count"], 1);
    host.window("store").emit("search", "nothing like it");
    await host.settle();
    assert.equal(props(host)["empty-text"], "没有符合条件的插件");
    assert.equal(props(host)["can-show-builtins"], false);
    host.window("store").emit("search", "vestra");
    await showBuiltins(host);
    assert.equal(props(host)["can-show-builtins"], false);
    assert.deepEqual(names(host), ["vestra.video"]);
    assert.equal(props(host)["can-clear"], false);
  } finally {
    await stop();
  }
});

test("search and category combine with it", async () => {
  const { host, stop } = await ready();
  try {
    const window = host.window("store");
    await tab(host, ALL);
    window.emit("search", "vestra.");
    await host.settle();
    // Both reserved ids match; the built-in's entry is left out.
    assert.deepEqual(names(host), ["Other"]);
    await showBuiltins(host);
    assert.deepEqual(names(host), ["Other", "vestra.video"], "All: the built-in's own row, for its entry too");
    window.emit("search", "");
    // Other: tags no category knows ("core"), or none.
    window.emit("setGoal", 8);
    await host.settle();
    assert.deepEqual(names(host), ["Alpha", "Old", "Other", "vestra.video"]);
    await hide(host, true);
    assert.equal(props(host)["goal-index"], 8, "the category stays");
    assert.deepEqual(names(host), ["Alpha", "Old", "Other"]);
    // Sound: "audio".
    window.emit("setGoal", 2);
    await host.settle();
    assert.deepEqual(names(host), ["Beta"]);
  } finally {
    await stop();
  }
});

test("the choice is kept in settings.json; settings saved before the option hide built-ins", async () => {
  const data = join(scratch, "kept");
  const first = await ready(data);
  await showBuiltins(first.host);
  // The tab is kept too: the next start opens on Installed.
  await tab(first.host, INSTALLED);
  await first.stop();
  assert.equal(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).hideBuiltins, false);

  const second = await ready(data);
  try {
    assert.equal(props(second.host)["hide-builtins"], false);
    assert.deepEqual(names(second.host), ["vestra.video", "Alpha", "Old"]);
    // The settings page's check box keeps it.
    second.host.settingsPage("store.repos").emit("toggleAuto", false);
    await second.host.settle();
    assert.equal(second.store.settings.value.hideBuiltins, false);
    await hide(second.host, true);
  } finally {
    await second.stop();
  }
  assert.equal(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).hideBuiltins, true);

  // A store before 0.2.8 wrote no hideBuiltins: the file is still read (auto check stays off), built-ins hidden.
  const old = join(scratch, "old");
  mkdirSync(old, { recursive: true });
  writeFileSync(join(old, "settings.json"), JSON.stringify({ repos: [], autoCheck: false, lastCheck: 0, revocations: {} }));
  const third = await ready(old);
  try {
    assert.equal(third.store.settings.value.autoCheck, false);
    assert.equal(props(third.host)["hide-builtins"], true);
    assert.equal(props(third.host)["category"], ALL, "no tab kept: All");
    assert.deepEqual(names(third.host), ["Alpha", "Beta", "Old", "Other"]);
  } finally {
    await third.stop();
  }
});

test("select(key) of a hidden built-in opens its page and changes nothing on the grid", async () => {
  const { host, store, stop } = await ready();
  try {
    await tab(host, INSTALLED);
    host.window("store").emit("search", "vestra");
    await host.settle();
    assert.deepEqual(names(host), []);
    const reset = props(host)["search-reset"];
    store.select("\nvestra.video");
    await host.settle();
    assert.equal(details(host), "vestra.video");
    assert.equal(store.hideBuiltins, true);
    assert.equal(store.search, "vestra");
    assert.equal(props(host)["search-reset"], reset);
    assert.equal(props(host)["category"], INSTALLED);
    assert.deepEqual(names(host), []);
  } finally {
    await stop();
  }
});

test("only built-ins installed and hidden: the Installed tab says they are hidden", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, stop } = await start({ net, packages: [BUILTIN] });
  try {
    await openStore(host);
    // All: the repository is empty and the built-in hidden.
    assert.deepEqual(names(host), []);
    assert.equal(props(host)["empty-text"], "仓库里还没有插件");
    assert.equal(props(host)["all-count"], 0);
    await tab(host, INSTALLED);
    assert.deepEqual(names(host), []);
    assert.equal(props(host)["empty-text"], "除内置插件外还没有安装插件（内置插件已隐藏）");
    assert.equal(props(host)["installed-count"], 0);
    await showBuiltins(host);
    assert.deepEqual(names(host), ["vestra.video"]);
  } finally {
    await stop();
  }
  // Nothing installed at all: as before.
  const empty = await start({ net, packages: [] });
  try {
    await openStore(empty.host);
    await tab(empty.host, INSTALLED);
    assert.equal(props(empty.host)["empty-text"], "还没有安装插件");
  } finally {
    await empty.stop();
  }
});
