/**
 * "Hide built-in plugins" (store 0.2.8), under the tag and sort drop-downs:
 * on by default, also for settings saved before it; built-ins are left out of
 * every tab (their own Installed rows and a repository's entries for them),
 * of the tab titles' counts and of the tag list. Search and tag combine with
 * it, the details follow the list as for a tag, and the choice is kept in
 * settings.json.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ALL, BUILTIN, FakeNet, info, INSTALLED, OFFICIAL, openStore, props, publish, rows, showBuiltins, start, tab, until } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-builtins-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

/**
 * Installed: the built-in, Alpha, Old (an update). Not installed: Beta, the
 * repository's entry for the built-in ("Video", tagged "core"), and
 * vestra.other, a reserved id the player does not ship (not a built-in).
 */
const PLUGINS = [
  { id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] },
  { id: "o.old", name: "Old", versions: [{ version: "2.0.0" }, { version: "1.0.0" }] },
  { id: "b.beta", name: "Beta", tags: ["audio"], versions: [{ version: "1.0.0" }] },
  { id: "vestra.video", name: "Video", tags: ["core"], versions: [{ version: "9.0.0" }] },
  { id: "vestra.other", name: "Other", versions: [{ version: "1.0.0" }] },
];

async function ready(dataDirectory?: string) {
  const net = new FakeNet();
  publish(net, OFFICIAL, PLUGINS);
  const started = await start({
    net,
    packages: [BUILTIN, info("a.alpha", { version: "1.0.0", source: fromRepo }), info("o.old", { version: "1.0.0", source: fromRepo })],
    ...(dataDirectory ? { dataDirectory } : {}),
  });
  await openStore(started.host);
  await until(started.host, () => started.store.items.length === 6, "rows");
  return started;
}

type Host = Awaited<ReturnType<typeof start>>["host"];

const names = (host: Host) => rows(host).map((r) => r.name);
const details = (host: Host) => (props(host)["has-selection"] ? (props(host)["d-name"] as string) : null);
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
    assert.equal(details(host), "Alpha");
    // The tab titles count what the tabs list.
    assert.equal(props(host)["all-count"], 4);
    assert.equal(props(host)["installed-count"], 2);
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "audio"]);
    await tab(host, INSTALLED);
    assert.deepEqual(names(host), ["Alpha", "Old"]);
    await tab(host, ALL);

    await showBuiltins(host);
    assert.equal(props(host)["hide-builtins"], false);
    assert.equal(store.settings.value.hideBuiltins, false);
    assert.deepEqual(names(host), ["Alpha", "Beta", "Old", "Other", "vestra.video", "Video"]);
    assert.deepEqual(props(host)["tag-model"], ["标签：全部", "audio", "core"]);
    assert.equal(props(host)["all-count"], 6);
    assert.equal(details(host), "Alpha", "the selection stays while listed");
    await tab(host, INSTALLED);
    // As before the option: built-ins first under Installed.
    assert.deepEqual(names(host), ["vestra.video", "Alpha", "Old"]);
    assert.equal(props(host)["installed-count"], 3);
    // The update count (the menu, the Updates tab) never counted built-ins.
    assert.equal(props(host)["updates"], 1);
  } finally {
    await stop();
  }
});

test("hiding the selected built-in selects the list's first row; an emptied list has no details", async () => {
  const { host, stop } = await ready();
  try {
    await tab(host, INSTALLED);
    await showBuiltins(host);
    host.window("store").emit("select","\nvestra.video");
    await host.settle();
    assert.equal(details(host), "vestra.video");
    const scroll = props(host)["scroll-request"] as number;
    await hide(host, true);
    assert.deepEqual(names(host), ["Alpha", "Old"]);
    assert.equal(details(host), "Alpha");
    assert.ok((props(host)["scroll-request"] as number) > scroll, "the list scrolls to the selection");

    // A selection the list keeps stays.
    host.window("store").emit("select", rows(host)[1]!.key);
    await showBuiltins(host);
    assert.equal(details(host), "Old");
    await hide(host, true);
    assert.equal(details(host), "Old");

    // Only built-ins matched the search: nothing left, no details; showing them fills the list again.
    host.window("store").emit("search", "vestra");
    await host.settle();
    assert.deepEqual(names(host), []);
    assert.equal(details(host), null);
    assert.equal(props(host)["empty-text"], "没有符合条件的插件");
    await showBuiltins(host);
    assert.deepEqual(names(host), ["vestra.video"]);
    assert.equal(details(host), "vestra.video");
  } finally {
    await stop();
  }
});

test("search and tag combine with it; a tag only built-ins have goes back to Tags: All", async () => {
  const { host, stop } = await ready();
  try {
    const window = host.window("store");
    await tab(host, ALL);
    window.emit("search", "vestra.");
    await host.settle();
    // Both reserved ids match; the built-in's entry is left out.
    assert.deepEqual(names(host), ["Other"]);
    await showBuiltins(host);
    assert.deepEqual(names(host), ["Other", "vestra.video", "Video"], "All: the built-in's own row too");
    window.emit("search", "");
    window.emit("setTag", 2);
    await host.settle();
    assert.deepEqual(names(host), ["Video"]);
    assert.equal(details(host), "Video");
    await hide(host, true);
    assert.equal(props(host)["tag-index"], 0, "Tags: All");
    assert.deepEqual(names(host), ["Alpha", "Beta", "Old", "Other"]);
    assert.equal(details(host), "Alpha");
    // A tag other plugins have stays and filters as before.
    window.emit("setTag", 1);
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

test("select(key) of a hidden built-in shows the built-ins first and keeps the search that lists it", async () => {
  const { host, store, stop } = await ready();
  try {
    await tab(host, INSTALLED);
    host.window("store").emit("search", "vestra");
    await host.settle();
    assert.deepEqual(names(host), []);
    const clear = props(host)["clear-search"];
    store.select("\nvestra.video");
    await host.settle();
    assert.equal(store.hideBuiltins, false);
    assert.equal(props(host)["hide-builtins"], false);
    assert.equal(store.search, "vestra", "the search still lists it");
    assert.equal(props(host)["clear-search"], clear);
    assert.equal(props(host)["category"], INSTALLED);
    assert.deepEqual(names(host), ["vestra.video"]);
    assert.equal(details(host), "vestra.video");
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
