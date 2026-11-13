/**
 * The capability scan in the store (SDK 1.8.0, plan A1/A6; user ruling
 * 2026-11-13): a repository's index may carry, per version, the categories
 * its scan found in the code (`scan: { uses: [...] }`). They are all the
 * store tells about what a plugin does with the player: the install
 * confirmation and the window's Permissions list show them in plain words,
 * and without a scan (an older index, a package from a file or a link) show
 * nothing; the capabilities a package declares are never shown. Every
 * install confirmation says that plugins may read files or use the network.
 * The update confirmation and the window's notice compare scans only.
 */
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { parseIndex } from "../index-format.ts";
import { installFromRepository } from "../installer.ts";
import { addedBy, categories, CATEGORIES, CAPABILITIES } from "../labels.ts";
import { msg } from "../text.ts";
import { banners, BUILTIN, choose, props, FakeNet, info, OFFICIAL, openStore, publish, signWith, start, THIRD, until, vxp, yes, type PluginSpec } from "./helpers.ts";

const SENTENCE = "插件可能读取你的文件或联网。";
const NONE = "它不使用播放器的任何敏感功能。";
const PLAYER = "修改你的播放器设置（快捷键、主题、语言、渲染方式），或直接控制播放引擎";
const PLUGINS = "安装、卸载、停用或重新加载其它插件，或运行任意命令";

/** The bullet lines of a confirmation's detail. */
const bullets = (detail: unknown) =>
  String(detail)
    .split("\n")
    .filter((line) => line.startsWith("· "));

/** `publish`, then each version's `scan` as the plugins repository's index build writes it (`"<id> <version>"` → uses). */
function publishScanned(net: FakeNet, url: string, plugins: PluginSpec[], scans: Record<string, unknown>) {
  const index = JSON.parse(publish(net, url, plugins).toString("utf8")) as {
    plugins: { id: string; versions: { version: string; scan?: unknown }[] }[];
  };
  for (const plugin of index.plugins)
    for (const version of plugin.versions) {
      const key = `${plugin.id} ${version.version}`;
      if (Object.hasOwn(scans, key)) version.scan = { uses: scans[key] };
    }
  const bytes = Buffer.from(`${JSON.stringify(index, null, 2)}\n`);
  net.route(url, { body: bytes });
  net.route(`${url}.sig`, { body: `${signWith(bytes)}\n` });
}

async function ready(net: FakeNet, packages = [BUILTIN]) {
  const started = await start({ net, packages });
  await openStore(started.host);
  await until(started.host, () => started.store.items.some((item) => item.entry !== null), "rows");
  return started;
}

type Host = Awaited<ReturnType<typeof start>>["host"];
const installs = (host: Host) => host.packageCalls.filter((c) => c.method === "install");

/** The window's notice about an update's new permissions (of the chosen plugin), or none. */
const notice = (host: Host) => banners(host).filter((text) => text.startsWith("新版本需要新的权限"));

/** The window's Permissions list of the chosen plugin. */
const listed = (host: Host) => props(host)["permissions"] as { text: string; risky: boolean }[];

/** Choose `name`, press `action` and answer yes: the confirmation shown. */
async function confirmation(host: Host, name: string, action: "install" | "update") {
  await choose(host, name);
  const before = host.confirmTexts.length;
  host.dialogAnswers.push(yes);
  host.window("store").emit("act", action);
  await until(host, () => host.confirmTexts.length > before, "confirmation");
  return host.confirmTexts.at(-1)!;
}

const version = (extra: Record<string, unknown>) => ({
  version: "1.0.0",
  apiVersion: "vestra/1",
  minPlayerVersion: null,
  capabilities: ["commands"],
  downloadUrl: "https://plugins.test/a.vxp",
  sha256: "a".repeat(64),
  size: 10,
  published: "2026-11-13T00:00:00.000Z",
  ...extra,
});
const scanOf = (extra: Record<string, unknown>) =>
  parseIndex(JSON.stringify({ schema: 1, plugins: [{ id: "a.one", versions: [version(extra)] }] })).index.plugins[0]!.versions[0]!.scan;

test("index: a version's scan.uses is read; absent or malformed is no scan, and the version stays", () => {
  assert.deepEqual(scanOf({ scan: { uses: ["player", "clipboard"] } }), { uses: ["player", "clipboard"] });
  assert.deepEqual(scanOf({ scan: { uses: [] } }), { uses: [] }, "scanned, nothing found");
  assert.deepEqual(scanOf({ scan: { uses: ["player", "player"], sdk: "1.8.0", method: "types" } }), { uses: ["player"] }, "each once; other keys ignored");
  assert.deepEqual(scanOf({ scan: { uses: ["future"] } }), { uses: ["future"] }, "a word from a newer scan is kept");
  assert.equal(scanOf({}), null, "an index from before the scan");
  for (const bad of [null, "player", ["player"], {}, { uses: "player" }, { uses: [1] }, { uses: ["Not A Word"] }, { uses: Array.from({ length: 33 }, (_, i) => `w${i}`) }])
    assert.equal(scanOf({ scan: bad }), null, JSON.stringify(bad));
});

test("labels: mpv is risky; the eight categories in the scan's order, an unknown word named last", () => {
  assert.deepEqual(CAPABILITIES.mpv, {
    text: "Control the playback engine (mpv) directly: send its commands and change its options",
    risky: true,
  });
  assert.deepEqual(Object.keys(CATEGORIES), ["player", "plugins", "clipboard", "credentials", "camera", "microphone", "screen", "notifications"]);
  assert.deepEqual(categories(["future", "clipboard", "player", "clipboard"]), [
    msg(CATEGORIES.player),
    msg(CATEGORIES.clipboard),
    msg("Unknown permission: {}", "future"),
  ]);
  assert.deepEqual(categories([]), []);
});

test("the categories, the mpv capability and the two sentences are translated in all six languages", () => {
  const texts = [
    ...Object.values(CATEGORIES),
    CAPABILITIES.mpv.text,
    "Plugins may read your files or use the network.",
    "It uses none of the player's sensitive features.",
  ];
  const pot = readFileSync(new URL("../locales/messages.pot", import.meta.url), "utf8");
  for (const text of texts) assert.ok(pot.includes(`msgid ${JSON.stringify(text)}\n`), `messages.pot: ${text}`);
  for (const locale of ["de", "es", "fr", "ja", "ko", "zh-Hans"]) {
    const po = readFileSync(new URL(`../locales/${locale}.po`, import.meta.url), "utf8");
    for (const text of texts) {
      const at = po.indexOf(`msgid ${JSON.stringify(text)}\nmsgstr "`);
      assert.ok(at >= 0, `${locale}.po: ${text}`);
      assert.notEqual(po.slice(at).split("\n")[1], 'msgstr ""', `${locale}.po, not translated: ${text}`);
    }
  }
});

test("official install with a scan: the scanned categories instead of the capabilities, after the files-and-network sentence", async () => {
  const net = new FakeNet();
  publishScanned(
    net,
    OFFICIAL,
    [{ id: "a.scanned", name: "Scanned", author: "zysilm", versions: [{ version: "1.0.0", capabilities: ["commands", "clipboard", "plugins.manage", "mpv"] }] }],
    // Declares the clipboard but never uses it; out of the scan's order here.
    { "a.scanned 1.0.0": ["plugins", "player"] },
  );
  const { host, stop } = await ready(net);
  try {
    const confirm = await confirmation(host, "Scanned", "install");
    // The window lists the same categories, all drawn as ones to watch; not the four declared capabilities.
    assert.deepEqual(listed(host), [
      { text: PLAYER, risky: true },
      { text: PLUGINS, risky: true },
    ]);
    assert.equal(props(host)["scanned"], true);
    assert.equal(confirm.message, "安装「Scanned」1.0.0？");
    assert.equal(
      confirm.detail,
      `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}\n\n它将可以：\n· ${PLAYER}\n· ${PLUGINS}`,
    );
    await until(host, () => installs(host).length === 1, "install");
  } finally {
    await stop();
  }
});

test("official install with a scan that found nothing: no list, one sentence instead", async () => {
  const net = new FakeNet();
  publishScanned(
    net,
    OFFICIAL,
    [{ id: "a.quiet", name: "Quiet", author: "zysilm", versions: [{ version: "1.0.0", capabilities: ["commands", "clipboard"] }] }],
    { "a.quiet 1.0.0": [] },
  );
  const { host, stop } = await ready(net);
  try {
    const confirm = await confirmation(host, "Quiet", "install");
    assert.deepEqual(listed(host), [], "the window lists nothing either");
    assert.equal(props(host)["scanned"], true, "scanned: the Permissions heading shows, over an empty list");
    assert.equal(confirm.detail, `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}\n\n${NONE}`);
  } finally {
    await stop();
  }
});

test("official install without a scan: only the sentence; no list in the confirmation or the window, whatever is declared", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.old", name: "Old Index", author: "zysilm", versions: [{ version: "1.0.0", capabilities: ["commands", "ui.window", "clipboard", "mpv"] }] },
  ]);
  const { host, stop } = await ready(net);
  try {
    const confirm = await confirmation(host, "Old Index", "install");
    assert.equal(confirm.detail, `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}`);
    assert.deepEqual(listed(host), []);
    assert.equal(props(host)["scanned"], false, "no scan: no Permissions heading");
    await until(host, () => installs(host).length === 1, "install");
  } finally {
    await stop();
  }
});

test("unofficial install without a scan: the repository's warning alone", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  publish(net, THIRD, [{ id: "someone.viz", name: "Viz", versions: [{ version: "1.0.0", capabilities: ["commands", "secrets"] }] }]);
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => store.items.some((i) => i.name === "Viz"));
    const detail = String((await confirmation(host, "Viz", "install")).detail);
    assert.match(detail, /^仓库：/);
    assert.match(detail, /这个仓库未经 Vestra Player 审核/);
    assert.ok(!detail.includes("\n"), "one paragraph: nothing after the warning");
    assert.deepEqual(bullets(detail), []);
    assert.deepEqual(listed(host), []);
  } finally {
    await stop();
  }
});

test("replacing a file install from an unofficial repository without a scan: no list, no empty paragraph", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  publish(net, THIRD, [{ id: "someone.viz", name: "Viz", versions: [{ version: "1.0.0", capabilities: ["commands", "secrets"] }] }]);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, info("someone.viz", { version: "0.9.0", capabilities: ["commands"] })] });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => store.items.some((i) => i.entry !== null));
    const item = store.items.find((i) => i.entry !== null)!;
    host.dialogAnswers.push(yes);
    await installFromRepository(store, item, item.entry!.versions[0]!);
    const confirm = host.confirmTexts.at(-1)!;
    assert.equal(confirm.message, "替换已安装的「Viz」？");
    const detail = String(confirm.detail);
    assert.match(detail, /将安装非官方仓库的 1\.0\.0。\n\n仓库：/);
    assert.match(detail, /这个仓库未经 Vestra Player 审核/);
    assert.deepEqual(bullets(detail), []);
    assert.ok(!detail.endsWith("\n") && !detail.includes("\n\n\n"), detail);
  } finally {
    await stop();
  }
});

test("unofficial install with a scan: the repository's own warning says files and network once; the categories follow", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  publishScanned(
    net,
    THIRD,
    [{ id: "someone.viz", name: "Viz", versions: [{ version: "1.0.0", capabilities: ["commands", "secrets"] }] }],
    { "someone.viz 1.0.0": ["credentials", "future"] },
  );
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => store.items.some((i) => i.name === "Viz"));
    const confirm = await confirmation(host, "Viz", "install");
    assert.equal(confirm.message, "安装来自非官方仓库的「Viz」1.0.0？");
    const detail = String(confirm.detail);
    assert.match(detail, /这个仓库未经 Vestra Player 审核/);
    // The repository's warning says it (the same words in Chinese): not a second time.
    assert.equal(detail.split("读取你的文件").length, 2, "said once");
    assert.deepEqual(bullets(detail), ["· 在系统钥匙串保存登录信息", "· 未知权限：future"]);
  } finally {
    await stop();
  }
});

const UPDATE: PluginSpec = {
  id: "zysilm.youtube",
  name: "YouTube",
  author: "zysilm",
  versions: [
    { version: "0.3.0", capabilities: ["commands", "clipboard", "ui.window"] },
    { version: "0.2.0", capabilities: ["commands"] },
  ],
};
const INSTALLED = info("zysilm.youtube", { version: "0.2.0", capabilities: ["commands"], source: { kind: "repository", url: OFFICIAL } });

test("update, both versions scanned: a new category is named, by the window's notice and the confirmation alike", async () => {
  const net = new FakeNet();
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.3.0": ["player", "clipboard"], "zysilm.youtube 0.2.0": ["clipboard"] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    const shown = notice(host);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    assert.equal(confirm.detail, `新增：\n· ${PLAYER}\n\n当前 0.2.0。`);
    // The notice named the same, not the declared clipboard and ui.window.
    assert.deepEqual(shown, [`新版本需要新的权限：${PLAYER}`]);
  } finally {
    await stop();
  }
});

test("update, both versions scanned: the same categories ask plainly, whatever the capabilities declare", async () => {
  const net = new FakeNet();
  // 0.3.0 declares clipboard and ui.window anew, but its code crosses no new boundary.
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.3.0": ["clipboard"], "zysilm.youtube 0.2.0": ["clipboard", "player"] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    assert.deepEqual(notice(host), [], "no notice either");
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？");
    assert.equal(confirm.detail, "当前 0.2.0。");
  } finally {
    await stop();
  }
});

test("update, only the new version scanned: every category it uses is named; what it declares is not", async () => {
  const net = new FakeNet();
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.3.0": ["player"] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    assert.deepEqual(notice(host), [`新版本需要新的权限：${PLAYER}`]);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    // Not the clipboard and ui.window it declares anew.
    assert.equal(confirm.detail, `新增：\n· ${PLAYER}\n\n当前 0.2.0。`);
  } finally {
    await stop();
  }
});

test("update, only the new version scanned and nothing found: asks plainly, though it declares more", async () => {
  const net = new FakeNet();
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.3.0": [] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    assert.deepEqual(notice(host), []);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？");
    assert.equal(confirm.detail, "当前 0.2.0。");
  } finally {
    await stop();
  }
});

test("addedBy compares scans only: nothing without a scan of the new version, every category without one of the installed", () => {
  const words = (lines: readonly { text: string; args: readonly unknown[] }[]) => lines.map((line) => [line.text, ...line.args].join(" "));
  const v = (version: string, uses: string[] | null) => ({ version, scan: uses ? { uses } : null });
  // Both scanned: what the new one adds.
  assert.deepEqual(words(addedBy([v("1.0.0", ["clipboard"])], { version: "1.0.0" }, v("1.1.0", ["player", "clipboard", "future"]))), [
    CATEGORIES.player,
    "Unknown permission: {} future",
  ]);
  assert.deepEqual(addedBy([v("1.0.0", ["clipboard", "player"])], { version: "1.0.0" }, v("1.1.0", ["clipboard"])), []);
  // Only the new one scanned (or the installed version is not in the index): every category it uses.
  assert.deepEqual(words(addedBy([v("1.0.0", null)], { version: "1.0.0" }, v("1.1.0", ["player", "clipboard"]))), [CATEGORIES.player, CATEGORIES.clipboard]);
  assert.deepEqual(words(addedBy([], { version: "0.9.0" }, v("1.1.0", ["plugins"]))), [CATEGORIES.plugins]);
  assert.deepEqual(addedBy([v("1.0.0", null)], { version: "1.0.0" }, v("1.1.0", [])), []);
  // The new one not scanned: nothing, whatever the installed one's scan found.
  assert.deepEqual(addedBy([v("1.0.0", [])], { version: "1.0.0" }, v("1.1.0", null)), []);
  assert.deepEqual(addedBy([v("1.0.0", ["clipboard"])], { version: "1.0.0" }, v("1.1.0", null)), []);
  assert.deepEqual(addedBy([v("1.0.0", null)], { version: "1.0.0" }, v("1.1.0", null)), []);
});

test("update to a version without a scan: nothing about permissions, in the notice or the confirmation, though it declares more", async () => {
  const net = new FakeNet();
  // 0.2.0 was scanned; 0.3.0 has no scan and declares clipboard and ui.window anew.
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.2.0": [] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    assert.deepEqual(notice(host), []);
    assert.deepEqual(listed(host), [], "0.3.0 has no scan: the window lists nothing");
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？");
    assert.equal(confirm.detail, "当前 0.2.0。");
  } finally {
    await stop();
  }
});

test("update from a version without a scan to a scanned one: its categories are named, in the notice and the confirmation", async () => {
  const net = new FakeNet();
  const themed: PluginSpec = {
    ...UPDATE,
    versions: [
      { version: "0.3.0", capabilities: ["commands", "ui.theme"] },
      { version: "0.2.0", capabilities: ["commands", "ui.theme"] },
    ],
  };
  publishScanned(net, OFFICIAL, [themed], { "zysilm.youtube 0.3.0": ["player"] });
  const installed = info("zysilm.youtube", { version: "0.2.0", capabilities: ["commands", "ui.theme"], source: { kind: "repository", url: OFFICIAL } });
  const { host, stop } = await ready(net, [BUILTIN, installed]);
  try {
    await choose(host, "YouTube");
    assert.deepEqual(notice(host), [`新版本需要新的权限：${PLAYER}`]);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    assert.equal(confirm.detail, `新增：\n· ${PLAYER}\n\n当前 0.2.0。`);
  } finally {
    await stop();
  }
});

test("a package from a file: the sentence and no list, whatever it declares", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "vestra-store-scan-"));
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({ net });
  try {
    const risky = join(scratch, "a.vxp");
    writeFileSync(risky, vxp({ id: "someone.a", version: "1.0.0", name: "A", capabilities: ["commands", "bindings.manage", "playback.read"] }));
    host.dialogAnswers.push(yes);
    await host.drop([risky]);
    await until(host, () => host.confirms.length === 1 && !store.busy, "confirmation");
    const detail = String(host.confirmTexts[0]!.detail);
    assert.match(detail, /^来自文件 a\.vxp，未经任何仓库审核，也不会自动更新/);
    assert.ok(detail.endsWith(`\n\n${SENTENCE}`), detail);
    assert.deepEqual(bullets(detail), []);
    // Installed from a file: no index entry, so the window lists nothing.
    await until(host, () => store.selected === "\nsomeone.a", "selected");
    assert.deepEqual(listed(host), []);
  } finally {
    await stop();
    rmSync(scratch, { recursive: true, force: true });
  }
});
