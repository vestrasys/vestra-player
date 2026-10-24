import assert from "node:assert/strict";
import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { BUILTIN, FakeNet, info, OFFICIAL, openStore, publish, rows, signWith, start, THIRD, until, other } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-repos-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const YT = { id: "zysilm.youtube", name: "YouTube", versions: [{ version: "0.2.0" }] };
const BB = { id: "zysilm.bilibili", name: "Bilibili 视频", versions: [{ version: "0.2.0" }] };

test("the official index is read, verified and listed when the window opens", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT, BB]);
  const { host, store, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 3, "rows");
    assert.deepEqual(
      rows(host).map((r) => r.name),
      ["Bilibili 视频", "vestra.video", "YouTube"],
    );
    assert.match(String(host.window("store").props["status-text"]), /^官方仓库 \d\d:\d\d 已更新$/);
    assert.equal(store.repos.official.index?.plugins.length, 2);
    const manifest = readFileSync(new URL("../manifest.json", import.meta.url), "utf8");
    const { version } = JSON.parse(manifest) as { version: string };
    assert.equal(net.requests[0]!.headers["user-agent"], `VestraPlayer/0.2.0 vestra.store/${version}`);
  } finally {
    await stop();
  }
});

test("a tampered index, a wrong key or a missing signature keep the last verified copy", async () => {
  const net = new FakeNet();
  const good = publish(net, OFFICIAL, [YT]);
  const data = join(scratch, "tamper");
  const first = await start({ net, dataDirectory: data });
  await first.store.refreshAll(true);
  assert.equal(first.store.repos.official.index?.plugins.length, 1);

  // Same signature, one byte changed (a plugin added by an attacker).
  const forged = Buffer.from(good.toString().replace('"zysilm.youtube"', '"zysilm.youtubf"'));
  net.route(OFFICIAL, { body: forged });
  await first.store.refreshAll(true);
  const official = first.store.repos.official;
  assert.equal(official.status.text, "官方仓库的签名校验失败，可能被篡改，已忽略这次更新");
  assert.deepEqual(official.index?.plugins.map((p) => p.id), ["zysilm.youtube"]);

  // Signed with a key the store does not trust.
  net.route(OFFICIAL, { body: forged });
  net.route(`${OFFICIAL}.sig`, { body: signWith(forged, other.privateKey) });
  await first.store.refreshAll(true);
  assert.match(official.status.text, /签名校验失败/);

  net.route(`${OFFICIAL}.sig`, { status: 404 });
  await first.store.refreshAll(true);
  assert.equal(official.status.text, "官方仓库缺少签名文件，已忽略这次更新");
  assert.deepEqual(official.index?.plugins.map((p) => p.id), ["zysilm.youtube"]);
  await first.stop();
});

test("a replayed older index is refused", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ ...YT, versions: [{ version: "0.2.0" }, { version: "0.1.0" }] }]);
  const { store, logs, stop } = await start({ net });
  try {
    await store.refreshAll(true);
    assert.equal(store.repos.official.index?.plugins[0]?.versions.length, 2);
    // An old, genuinely signed index that does not list 0.2.0 yet.
    publish(net, OFFICIAL, [{ ...YT, versions: [{ version: "0.1.0" }] }]);
    await store.refreshAll(true);
    assert.equal(store.repos.official.status.text, "官方仓库返回了旧的索引，已忽略这次更新");
    assert.equal(store.repos.official.index?.plugins[0]?.versions.length, 2);
    assert.ok(logs.some((line) => line.includes("replayed index")));
  } finally {
    await stop();
  }
});

test("offline: the verified cache is used after a restart; a tampered cache is not", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT, BB]);
  const data = join(scratch, "offline");
  const first = await start({ net, dataDirectory: data });
  await first.store.refreshAll(true);
  const saved = join(scratch, "offline-copy");
  cpSync(data, saved, { recursive: true });
  await first.stop();

  net.offline = true;
  const before = net.count(OFFICIAL);
  const second = await start({ net, dataDirectory: saved });
  try {
    assert.equal(second.store.repos.official.index?.plugins.length, 2, "cache loaded at start");
    await openStore(second.host);
    // Fresh enough not to be refreshed on opening; 「刷新」 tries the network.
    assert.equal(net.count(OFFICIAL), before);
    second.host.window("store").emit("refresh");
    await until(second.host, () => !second.store.repos.official.refreshing && /无法连接/.test(second.store.repos.official.status.text));
    assert.match(second.store.repos.official.status.text, /^无法连接 官方仓库（找不到服务器），显示 .+ 的缓存$/);
    assert.equal(rows(second.host).length, 3);
  } finally {
    await second.stop();
  }

  // Someone edits the cached index on disk: it no longer verifies and is ignored.
  const tampered = join(scratch, "offline-tampered");
  cpSync(saved, tampered, { recursive: true });
  const dir = join(tampered, "repos", readdirSync(join(tampered, "repos"))[0]!);
  writeFileSync(join(dir, "index.json"), readFileSync(join(dir, "index.json"), "utf8").replace("YouTube", "YouTubf"));
  const third = await start({ net, dataDirectory: tampered });
  try {
    assert.equal(third.store.repos.official.index, null);
    assert.ok(third.logs.some((line) => line.includes("cached index does not verify")));
    await openStore(third.host);
    await until(third.host, () => /暂时看不到/.test(third.store.repos.official.status.text));
    // Only the built-in plugin is left in the list.
    assert.deepEqual(rows(third.host).map((r) => r.name), ["vestra.video"]);
  } finally {
    await third.stop();
  }
});

test("conditional requests: 304 keeps the copy", async () => {
  const net = new FakeNet();
  const bytes = publish(net, OFFICIAL, [YT]);
  const { store, stop } = await start({ net });
  try {
    await store.refreshAll(true);
    const etag = store.repos.official.meta?.etag;
    assert.ok(etag);
    net.route(OFFICIAL, (headers) => (headers["if-none-match"] === etag ? { status: 304 } : { body: bytes }));
    await store.refreshAll(true);
    assert.match(store.repos.official.status.text, /已是最新$/);
    assert.equal(store.repos.official.index?.plugins.length, 1);
  } finally {
    await stop();
  }
});

test("third-party repositories: no signature read, `official: true` ignored, labelled with the host", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  publish(net, THIRD, [{ id: "someone.viz", name: "Viz", versions: [{ version: "1.0.0" }] }], {
    extra: { name: "Vestra 官方" },
  });
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    const third = store.repos.find(THIRD)!;
    assert.equal(third.official, false);
    assert.equal(third.label, "Vestra 官方（third.example）");
    assert.equal(net.count(`${THIRD}.sig`), 0);
    await openStore(host);
    const row = rows(host).find((r) => r.name === "Viz") as { meta: string } | undefined;
    assert.equal(row?.meta, "Someone · 非官方：Vestra 官方（third.example）");
  } finally {
    await stop();
  }
});

test("review B: replaying an older signed index cannot hide a revocation", async () => {
  const net = new FakeNet();
  const plugins = [{ id: "zysilm.p", name: "P", versions: [{ version: "0.2.0" }, { version: "0.1.0" }] }];
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, info("zysilm.p", { version: "0.2.0", source: { kind: "repository", url: OFFICIAL } })],
  });
  try {
    publish(net, OFFICIAL, plugins);
    await store.refreshAll(true);
    const old = [net.routes.get(OFFICIAL)!, net.routes.get(`${OFFICIAL}.sig`)!];
    host.dialogAnswers.push({ confirmed: false, checked: false });
    publish(net, OFFICIAL, plugins, { revoked: [{ id: "zysilm.p", versions: "0.2.0", reason: "bad" }] });
    await store.refreshAll(true);
    await until(host, () => store.items.find((i) => i.id === "zysilm.p")?.state === "revoked");
    net.route(OFFICIAL, old[0]!);
    net.route(`${OFFICIAL}.sig`, old[1]!);
    await store.refreshAll(true);
    assert.equal(store.repos.official.status.text, "官方仓库返回了旧的索引，已忽略这次更新");
    assert.equal(store.items.find((i) => i.id === "zysilm.p")?.state, "revoked");
    await openStore(host);
    assert.match(String(host.window("store").props["status-text"]), /官方仓库返回了旧的索引/);
    assert.equal(host.window("store").props["status-warn"], true);
  } finally {
    await stop();
  }
});

test("review C: a plugin absent for a while keeps its seen versions; an even older index is refused", async () => {
  const net = new FakeNet();
  const { store, stop } = await start({ net });
  try {
    publish(net, OFFICIAL, [{ id: "zysilm.p", versions: [{ version: "0.1.0" }] }]);
    await store.refreshAll(true);
    const v1 = [net.routes.get(OFFICIAL)!, net.routes.get(`${OFFICIAL}.sig`)!];
    publish(net, OFFICIAL, [{ id: "zysilm.q", versions: [{ version: "1.0.0" }] }]); // p delisted: accepted
    await store.refreshAll(true);
    assert.match(store.repos.official.status.text, /已更新$/);
    publish(net, OFFICIAL, [{ id: "zysilm.p", versions: [{ version: "0.2.0" }, { version: "0.1.0" }] }, { id: "zysilm.q", versions: [{ version: "1.0.0" }] }]);
    await store.refreshAll(true);
    assert.deepEqual(store.repos.official.meta?.seen?.versions["zysilm.p"]?.sort(), ["0.1.0", "0.2.0"]);
    net.route(OFFICIAL, v1[0]!);
    net.route(`${OFFICIAL}.sig`, v1[1]!);
    await store.refreshAll(true);
    assert.equal(store.repos.official.status.text, "官方仓库返回了旧的索引，已忽略这次更新");
    assert.deepEqual(store.repos.official.index?.plugins.map((p) => p.id), ["zysilm.p", "zysilm.q"]);
  } finally {
    await stop();
  }
});
