import assert from "node:assert/strict";
import { generateKeyPairSync } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { officialKeys, OFFICIAL_KEYS_DIRECTORY, verifyIndex } from "../signing.ts";
import { banners, choose, FakeNet, OFFICIAL, openStore, other, pair, props, publish, signWith, start, until } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-hardening-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const P = (version = "1.0.0") => [{ id: "vestra.p", name: "P", versions: [{ version }] }];
const REPLAY = "官方仓库返回了旧的索引，已忽略这次更新";
// A real 1×1 PNG.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64",
);
/** Another icon (the PNG with trailing bytes), so it has its own hash. */
const OTHER_PNG = Buffer.concat([PNG, Buffer.from("other")]);

test("generated: lower is a replay, equal and higher are fine, missing after one was seen is refused", async () => {
  const net = new FakeNet();
  const { store, logs, stop } = await start({ net });
  const official = store.repos.official;
  try {
    publish(net, OFFICIAL, P(), { extra: { generated: 5 } });
    await store.refreshAll(true);
    assert.equal(official.meta?.generated, 5);
    publish(net, OFFICIAL, P(), { extra: { generated: 4 } });
    await store.refreshAll(true);
    assert.equal(official.status.text, REPLAY);
    assert.ok(logs.some((l) => l.includes("generated 4 < 5")));
    publish(net, OFFICIAL, P(), { extra: { generated: 5 } });
    await store.refreshAll(true);
    assert.match(official.status.text, /已更新$/);
    publish(net, OFFICIAL, P("1.1.0").map((p) => ({ ...p, versions: [{ version: "1.1.0" }, { version: "1.0.0" }] })), { extra: { generated: 6 } });
    await store.refreshAll(true);
    assert.equal(official.meta?.generated, 6);
    for (const generated of [undefined, 0, -1, 6.5, "7"]) {
      publish(net, OFFICIAL, P("1.1.0").map((p) => ({ ...p, versions: [{ version: "1.1.0" }, { version: "1.0.0" }] })), {
        extra: generated === undefined ? {} : { generated },
      });
      await store.refreshAll(true);
      assert.equal(official.status.text, REPLAY, `generated ${String(generated)}`);
    }
    assert.equal(official.meta?.generated, 6);
  } finally {
    await stop();
  }
});

test("generated: an old index without it is accepted on a fresh store; a cached index older than the accepted one is ignored", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, P());
  const data = join(scratch, "cache");
  const first = await start({ net, dataDirectory: data });
  await first.store.refreshAll(true);
  assert.match(first.store.repos.official.status.text, /已更新$/);
  assert.equal(first.store.repos.official.meta?.generated, 0);
  await first.stop();
  // The cached index is older (0) than the highest accepted (says meta.json): not used.
  const dir = join(data, "repos", readdirSync(join(data, "repos"))[0]!);
  const meta = JSON.parse(readFileSync(join(dir, "meta.json"), "utf8"));
  writeFileSync(join(dir, "meta.json"), JSON.stringify({ ...meta, generated: 3 }));
  const second = await start({ net, dataDirectory: data });
  try {
    assert.equal(second.store.repos.official.index, null);
    assert.ok(second.logs.some((l) => l.includes("cached index is older")));
  } finally {
    await second.stop();
  }
});

test("asset hashes: matching README and icon are shown; a tampered or resized one is not shown or cached", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { id: "a.good", name: "Good", readme: "# Good", iconUrl: "https://plugins.test/assets/a.good/icon.png", icon: PNG, hashes: true, versions: [{ version: "1.0.0" }] },
    { id: "b.bad", name: "Bad", readme: "# Bad!", iconUrl: "https://plugins.test/assets/b.bad/icon.png", icon: OTHER_PNG, hashes: true, versions: [{ version: "1.0.0" }] },
    { id: "c.size", name: "Size", readme: "# Size", hashes: true, versions: [{ version: "1.0.0" }] },
  ]);
  net.route("https://plugins.test/assets/b.bad/README.md", { body: "# Evil" }); // same length, other bytes
  const icon = Buffer.from(OTHER_PNG);
  icon[icon.length - 1] = icon[icon.length - 1]! ^ 1;
  net.route("https://plugins.test/assets/b.bad/icon.png", { body: icon });
  net.route("https://plugins.test/assets/c.size/README.md", { body: "# Size, longer" });
  const { host, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => !!props(host)["rows"] && (props(host)["rows"] as unknown[]).length >= 3);
    await choose(host, "Good");
    await until(host, () => host.markdown(props(host)["doc"])?.text === "# Good", "good readme");
    await until(host, () => host.imagePath(props(host)["d-icon"])?.includes("sha-") === true, "good icon");

    await choose(host, "Bad");
    await until(host, () => banners(host).includes("无法加载说明：校验和与描述不符"), "bad readme");
    await host.settle(30);
    assert.match(String(host.imagePath(props(host)["d-icon"])), /assets\/icon\.png$/, "default icon");
    await choose(host, "Size");
    await until(host, () => banners(host).includes("无法加载说明：大小与描述不符"), "size mismatch");
    const icons = readdirSync(join(host.dataDirectory, "icons"));
    assert.equal(icons.length, 1, "only the good icon is cached");
  } finally {
    await stop();
  }
});

test("asset hashes: null (older index) means no check; malformed ones make only that asset unusable", async () => {
  const net = new FakeNet();
  const bytes = publish(net, OFFICIAL, [
    { id: "a.old", name: "Old", readme: "# Old", versions: [{ version: "1.0.0" }] },
    { id: "b.odd", name: "Odd", readme: "# Odd", versions: [{ version: "1.0.0" }] },
  ]);
  const index = JSON.parse(bytes.toString());
  index.plugins[1].readmeSha256 = "NOT-HEX";
  index.plugins[1].readmeSize = 5;
  const text = Buffer.from(JSON.stringify(index));
  net.route(OFFICIAL, { body: text });
  net.route(`${OFFICIAL}.sig`, { body: signWith(text) });
  const { host, store, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => store.repos.official.index?.plugins.length === 2, "both plugins kept");
    await choose(host, "Old");
    await until(host, () => host.markdown(props(host)["doc"])?.text === "# Old");
    await choose(host, "Odd");
    await until(host, () => props(host)["doc-note"] === "作者没有提供说明");
    assert.equal(net.count("https://plugins.test/assets/b.odd/README.md"), 0);
  } finally {
    await stop();
  }
});

test("a cached icon whose bytes no longer match its hash is fetched again", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.good", name: "Good", iconUrl: "https://plugins.test/i.png", icon: PNG, hashes: true, versions: [{ version: "1.0.0" }] }]);
  const data = join(scratch, "icon");
  const first = await start({ net, dataDirectory: data });
  await openStore(first.host);
  await until(first.host, () => (first.host.window("store").props["rows"] as unknown[]).length >= 1);
  await choose(first.host, "Good");
  await until(first.host, () => net.count("https://plugins.test/i.png") === 1);
  await until(first.host, () => readdirSync(join(data, "icons")).length === 1);
  await first.stop();
  const file = join(data, "icons", readdirSync(join(data, "icons"))[0]!);
  writeFileSync(file, Buffer.concat([PNG, Buffer.from("x")]));
  const second = await start({ net, dataDirectory: data });
  try {
    await openStore(second.host);
    await choose(second.host, "Good");
    await until(second.host, () => net.count("https://plugins.test/i.png") === 2, "downloaded again");
    await until(second.host, () => readFileSync(file).equals(PNG), "replaced by the verified copy");
  } finally {
    await second.stop();
  }
});

test("key rotation: a two-line .sig where only the second key is trusted; every keys/*.pub.pem is loaded", async () => {
  const unknown = generateKeyPairSync("ed25519");
  const net = new FakeNet();
  const bytes = publish(net, OFFICIAL, P());
  net.route(`${OFFICIAL}.sig`, { body: `${signWith(bytes, unknown.privateKey)}\n${signWith(bytes, other.privateKey)}` });
  const { store, stop } = await start({ net, keys: [pair.publicKey, other.publicKey] });
  try {
    await store.refreshAll(true);
    assert.match(store.repos.official.status.text, /已更新$/);
  } finally {
    await stop();
  }
  const dir = join(scratch, "keys");
  rmSync(dir, { recursive: true, force: true });
  const { mkdirSync } = await import("node:fs");
  mkdirSync(dir);
  writeFileSync(join(dir, "index-signing.pub.pem"), pair.publicKey.export({ type: "spki", format: "pem" }));
  writeFileSync(join(dir, "index-signing-next.pub.pem"), other.publicKey.export({ type: "spki", format: "pem" }));
  writeFileSync(join(dir, "README.txt"), "not a key");
  const keys = officialKeys(dir);
  assert.equal(keys.length, 2);
  assert.ok(verifyIndex(bytes, signWith(bytes, other.privateKey), keys));
  assert.equal(officialKeys(OFFICIAL_KEYS_DIRECTORY).length, 1, "the package ships one key today");
});
