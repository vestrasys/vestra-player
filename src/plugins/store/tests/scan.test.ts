/**
 * The capability scan in the confirmations (SDK 1.8.0, plan A1/A6): a
 * repository's index may carry, per version, the categories its scan found
 * in the code (`scan: { uses: [...] }`). The install confirmation then lists
 * those in plain words; without them (an older index, a package from a file
 * or a link) it lists the risky capabilities the package declares. Every
 * install confirmation says that plugins may read files or use the network.
 * The update confirmation compares the scanned categories when the index has
 * them for both versions, else the declared capabilities.
 */
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { parseIndex } from "../index-format.ts";
import { addedBy, categories, CATEGORIES, CAPABILITIES, riskyOnly } from "../labels.ts";
import { msg } from "../text.ts";
import { banners, BUILTIN, choose, FakeNet, info, OFFICIAL, openStore, publish, signWith, start, THIRD, until, vxp, yes, type PluginSpec } from "./helpers.ts";

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
  // The declared capabilities a confirmation names: risky ones, and those the store does not know.
  assert.deepEqual(riskyOnly(["commands", "clipboard", "ui.window", "mpv", "future.thing", "playback.read"]), ["clipboard", "mpv", "future.thing"]);
  assert.deepEqual(riskyOnly(["commands", "playback.control"]), []);
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
    assert.equal(confirm.detail, `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}\n\n${NONE}`);
  } finally {
    await stop();
  }
});

test("official install without a scan: only the declared risky capabilities, after the sentence", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.old", name: "Old Index", author: "zysilm", versions: [{ version: "1.0.0", capabilities: ["commands", "ui.window", "clipboard", "mpv"] }] },
    { id: "a.plain", name: "Plain", author: "zysilm", versions: [{ version: "1.0.0", capabilities: ["commands", "playback.control"] }] },
  ]);
  const { host, store, stop } = await ready(net);
  try {
    const confirm = await confirmation(host, "Old Index", "install");
    assert.equal(
      confirm.detail,
      `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}\n\n它将可以：\n· 读写剪贴板\n· 直接控制播放引擎（mpv）：发送它的命令、修改它的选项`,
    );
    await until(host, () => installs(host).length === 1 && !store.busy, "installed");
    // Nothing risky declared: no list.
    const plain = await confirmation(host, "Plain", "install");
    assert.equal(plain.detail, `来自 Vestra Player 官方仓库，作者 zysilm。\n\n${SENTENCE}\n\n${NONE}`);
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

test("update, only the new version scanned: the declared capabilities are compared, all of them", async () => {
  const net = new FakeNet();
  publishScanned(net, OFFICIAL, [UPDATE], { "zysilm.youtube 0.3.0": [] });
  const { host, stop } = await ready(net, [BUILTIN, INSTALLED]);
  try {
    await choose(host, "YouTube");
    const shown = notice(host);
    assert.equal(shown.length, 1);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    // The notice and the confirmation name the same two declared capabilities.
    for (const line of bullets(confirm.detail)) assert.ok(shown[0]!.includes(line.slice(2)), line);
    assert.equal(bullets(confirm.detail).length, 2, "clipboard and ui.window");
    assert.equal(bullets(confirm.detail)[0], "· 读写剪贴板");
    assert.match(String(confirm.detail), /^新增：\n/);
    assert.match(String(confirm.detail), /\n\n当前 0\.2\.0。$/);
  } finally {
    await stop();
  }
});

test("addedBy, one version scanned and the other not: nothing the user may not have been told is left out", () => {
  const words = (lines: readonly { text: string; args: readonly unknown[] }[]) => lines.map((line) => [line.text, ...line.args].join(" "));
  // Installed scanned (nothing found, so its install named nothing), the new one not scanned and declaring the same.
  assert.deepEqual(
    words(
      addedBy(
        [{ version: "1.0.0", scan: { uses: [] } }],
        { version: "1.0.0", capabilities: ["commands", "clipboard", "secrets"] },
        { capabilities: ["commands", "clipboard", "secrets"], scan: null },
      ),
    ),
    [CAPABILITIES.clipboard.text, CAPABILITIES.secrets.text],
    "every risky capability the new version declares",
  );
  // The same, with capabilities declared anew: named too, risky or not, each once.
  assert.deepEqual(
    words(
      addedBy(
        [{ version: "1.0.0", scan: { uses: ["clipboard"] } }],
        { version: "1.0.0", capabilities: ["commands", "clipboard"] },
        { capabilities: ["commands", "clipboard", "ui.window", "mpv"], scan: null },
      ),
    ),
    [CAPABILITIES.clipboard.text, CAPABILITIES.mpv.text, CAPABILITIES["ui.window"].text],
  );
  // Installed not scanned, declaring a capability that was never drawn as risky; the new one's scan finds the category.
  assert.deepEqual(
    words(
      addedBy(
        [{ version: "1.0.0", scan: null }],
        { version: "1.0.0", capabilities: ["commands", "ui.theme"] },
        { capabilities: ["commands", "ui.theme"], scan: { uses: ["player"] } },
      ),
    ),
    [CATEGORIES.player],
    "every category the new version uses",
  );
  // The same, with a capability declared anew: after the categories; the clipboard's words once.
  assert.deepEqual(
    words(
      addedBy(
        [{ version: "1.0.0", scan: null }],
        { version: "1.0.0", capabilities: ["commands"] },
        { capabilities: ["commands", "clipboard", "ui.window"], scan: { uses: ["player", "clipboard"] } },
      ),
    ),
    [CATEGORIES.player, CATEGORIES.clipboard, CAPABILITIES["ui.window"].text],
  );
  // The installed version is not in the index (delisted version): as not scanned.
  assert.deepEqual(
    words(addedBy([], { version: "0.9.0", capabilities: ["commands"] }, { capabilities: ["commands"], scan: { uses: ["plugins"] } })),
    [CATEGORIES.plugins],
  );
  // Neither scanned, nothing declared anew; a scan that found nothing: nothing to say.
  assert.deepEqual(addedBy([{ version: "1.0.0", scan: null }], { version: "1.0.0", capabilities: ["clipboard"] }, { capabilities: ["clipboard"], scan: null }), []);
  assert.deepEqual(addedBy([{ version: "1.0.0", scan: null }], { version: "1.0.0", capabilities: ["clipboard"] }, { capabilities: ["clipboard"], scan: { uses: [] } }), []);
});

test("update from a scanned version to one without a scan: its declared risky capabilities are named, in the notice and the confirmation", async () => {
  const net = new FakeNet();
  const same: PluginSpec = {
    ...UPDATE,
    versions: [
      { version: "0.3.0", capabilities: ["commands", "clipboard", "secrets"] },
      { version: "0.2.0", capabilities: ["commands", "clipboard", "secrets"] },
    ],
  };
  // 0.2.0 was installed with a scan that found nothing in use; 0.3.0 has no scan.
  publishScanned(net, OFFICIAL, [same], { "zysilm.youtube 0.2.0": [] });
  const installed = info("zysilm.youtube", { version: "0.2.0", capabilities: ["commands", "clipboard", "secrets"], source: { kind: "repository", url: OFFICIAL } });
  const { host, stop } = await ready(net, [BUILTIN, installed]);
  try {
    await choose(host, "YouTube");
    const shown = notice(host);
    const confirm = await confirmation(host, "YouTube", "update");
    assert.equal(confirm.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    assert.equal(confirm.detail, "新增：\n· 读写剪贴板\n· 在系统钥匙串保存登录信息\n\n当前 0.2.0。");
    assert.equal(shown.length, 1);
    for (const line of bullets(confirm.detail)) assert.ok(shown[0]!.includes(line.slice(2)), line);
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

test("a package from a file: the sentence, then only the declared risky capabilities", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "vestra-store-scan-"));
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({ net });
  try {
    const risky = join(scratch, "a.vxp");
    writeFileSync(risky, vxp({ id: "someone.a", version: "1.0.0", name: "A", capabilities: ["commands", "bindings.manage", "playback.read"] }));
    const plain = join(scratch, "b.vxp");
    writeFileSync(plain, vxp({ id: "someone.b", version: "2.0.0", name: "B", capabilities: ["commands"] }));
    host.dialogAnswers.push(yes, yes);
    await host.drop([risky, plain]);
    await until(host, () => host.confirms.length === 2 && !store.busy, "two confirmations");
    const first = String(host.confirmTexts[0]!.detail);
    assert.match(first, /^来自文件 a\.vxp，未经任何仓库审核，也不会自动更新/);
    assert.ok(first.endsWith(`\n\n${SENTENCE}\n\n它将可以：\n· 修改所有快捷键`), first);
    const second = String(host.confirmTexts[1]!.detail);
    assert.ok(second.endsWith(`\n\n${SENTENCE}\n\n${NONE}`), second);
  } finally {
    await stop();
    rmSync(scratch, { recursive: true, force: true });
  }
});
