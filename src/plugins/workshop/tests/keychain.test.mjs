// The keychain can wait on a system dialog (macOS asks again after the player
// was re-signed) or refuse: activation never waits on it, the plugin keeps
// running, and the window says what is going on.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { KEY, dataDirectory, server, startAi, until } from "./fixture.mjs";

/** A keychain read that waits until the test answers, like a dialog nobody has clicked yet. */
function heldKeychain(host) {
  const held = [];
  host.handle("secrets.get", (params) => new Promise((resolve, reject) => held.push({ name: params.name, resolve, reject })));
  return held;
}

test("activation does not wait for the keychain; the window shows that it is reading", async () => {
  const host = new TestHost();
  const held = heldKeychain(host);
  const started = await Promise.race([
    startPlugin(activate, { directory: new URL("..", import.meta.url), host }),
    new Promise((_, reject) => setTimeout(() => reject(new Error("activation waited for the keychain")), 3000)),
  ]);
  assert.ok(host.commands.has("workshop.open"));
  // Not awaited: run() waits until the plugin is idle, and the read is held.
  void host.command("workshop.open").run();
  await until(host, () => host.windows.has("ai") && /正在读取钥匙串/.test(host.window("ai").props.banner), "reading banner");
  const props = host.window("ai").props;
  assert.equal(props.configured, false);
  assert.equal(host.settingsPage("byok").props["key-state"], "正在读取钥匙串…");
  for (const read of held) read.resolve(null);
  await started.stop();
});

test("a denied keychain keeps the plugin running, says so, and a new save fixes it", async () => {
  const ai = await startAi({ deny: true });
  await ai.until(() => /无法读取钥匙串中的 key，请在设置中重新保存/.test(ai.props.banner), "error banner");
  assert.equal(ai.props.configured, false);
  assert.match(ai.page.props["key-state"], /无法读取/);
  ai.host.reset("secrets.get");
  ai.page.emit("save-key", KEY);
  await ai.until(() => ai.props.configured === true && ai.props.banner === "", "saved");
  assert.equal(ai.host.secrets.get("key.anthropic"), KEY);
  await ai.stop();
});

test("a slow keychain: the key arrives when the user answers; it is read once", async () => {
  const host = new TestHost();
  const held = heldKeychain(host);
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
  void host.command("workshop.open").run();
  void host.command("workshop.open").run(); // opening again shares the read
  await until(host, () => held.length === 1, "one read");
  held[0].resolve(KEY);
  const props = host.window("ai").props;
  await until(host, () => props.configured === true && props.banner === "", "configured");
  await host.command("workshop.open").run();
  await host.settle();
  assert.equal(held.length, 1, "the key is kept in memory");
  await started.stop();
});

test("the first slice's key moves lazily, after a refusal too", async () => {
  const mock = await server(() => ({ text: ["好"] }));
  const data = await dataDirectory();
  await fs.writeFile(path.join(data, "settings.json"), JSON.stringify({ provider: "vestra-compat", models: { "vestra-compat": "m" }, baseUrl: mock.url, lastSession: null }));
  const host = new TestHost();
  host.secrets.set("key.vestra-compat", KEY);
  let refuse = true;
  const stored = host.secrets;
  host.handle("secrets.get", ({ name }) => {
    if (refuse) throw new HostError("failed", "User canceled the operation.");
    return stored.get(name) ?? null;
  });
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host, dataDirectory: data });
  await host.command("workshop.open").run();
  const props = host.window("ai").props;
  // A custom endpoint works without a key; the refusal is remembered and retried.
  await until(host, () => props.configured === true, "configured");
  assert.equal(stored.has("key.vestra-compat"), true, "nothing moved while refused");
  const saved = async () => JSON.parse(await fs.readFile(path.join(data, "settings.json"), "utf8"));
  // settings.json is written debounced; wait for the migrated file.
  for (let i = 0; i < 100 && (await saved()).provider !== "custom-openai"; i++) await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal((await saved()).provider, "custom-openai");
  assert.equal((await saved()).migrateKeys, true);
  refuse = false;
  await host.command("workshop.open").run();
  await until(host, () => stored.get("key.custom-openai") === KEY, "moved");
  assert.equal(stored.has("key.vestra-compat"), false);
  await started.stop();
  assert.equal((await saved()).migrateKeys, false);
});
