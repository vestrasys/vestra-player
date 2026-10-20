import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";
import { HostError, startPlugin } from "../../sdk/plugin/testing.ts";
import type { Plugin } from "../../sdk/plugin/index.ts";

const runner = resolve(import.meta.dirname, "../../runtime/runner.ts");

/** Run a plugin in the real runner and answer its requests like the host. */
async function run(
  source: string,
  reply: (method: string) => object = () => ({ result: { revision: 1 } }),
) {
  const directory = mkdtempSync(join(tmpdir(), "runner-"));
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({
      id: "runner.test",
      entry: "main.mjs",
      capabilities: ["commands"],
    }),
  );
  writeFileSync(join(directory, "main.mjs"), source);
  const child = spawn(process.execPath, [runner, directory], {
    stdio: ["pipe", "pipe", "inherit"],
  });
  const methods: string[] = [];
  const lines = createInterface({ input: child.stdout });
  const event = await new Promise<string>((done) => {
    lines.on("line", (line) => {
      const message = JSON.parse(line);
      if (message.event) return done(message.event);
      methods.push(message.method);
      child.stdin.write(
        `${JSON.stringify({ id: message.id, ...reply(message.method) })}\n`,
      );
    });
  });
  child.stdin.end();
  await new Promise((done) => child.once("exit", done));
  return { event, methods };
}

test("the runner activates a default-exported plugin and refuses the removed flat API", async () => {
  const v2 = await run(
    "export default async function activate(plugin) { plugin.commands.add({ id: 'a', path: ['文件', 'A'], run() {} }); }",
  );
  assert.deepEqual(v2, {
    // The window state is read before activation, like the player state.
    event: "runtime.ready",
    methods: ["window.state", "commands.register"],
  });
  const legacy = await run(
    "export async function activate(host) { await host.commands.register('a', 1, {}, {}); }",
  );
  assert.deepEqual(legacy, { event: "runtime.failed", methods: [] });
});

test("a v2 plugin is not ready until its registrations succeeded", async () => {
  const failed = await run(
    "export default async function activate(plugin) { plugin.commands.add({ id: 'a', path: ['文件', 'A'], run() {} }); }",
    () => ({ error: { code: "command", message: "menu path collides" } }),
  );
  assert.deepEqual(failed, {
    event: "runtime.failed",
    methods: ["window.state", "commands.register"],
  });
});

test("packages: trial waits for start, install follows the operation to its end", async () => {
  const { host, plugin, stop } = await startPlugin(() => {}, {
    capabilities: ["plugins.manage"],
  });
  let trialPolls = 0;
  let operationPolls = 0;
  host.handle("packages.validate", () => ({
    token: "t",
    digest: "d",
    apiVersion: "vestra.experimental/0.1",
    manifest: {
      id: "community.counter",
      version: "1.0.0",
      apiVersion: "vestra.experimental/0.1",
      entry: "main.js",
      capabilities: [],
    },
    checks: ["slint"],
    trustedExecution: true,
  }));
  host.handle("packages.trial", () => ({ trialId: 7, status: "activating" }));
  host.handle("packages.trialState", () => ({
    trialId: 7,
    status: ++trialPolls > 1 ? "running" : "activating",
    cleanupConfirmed: false,
  }));
  host.handle("packages.stopTrial", () => ({
    trialId: 7,
    status: "stopped",
    cleanupConfirmed: true,
  }));
  host.handle("packages.installed", () => ({
    revision: 3,
    current: null,
    previous: null,
  }));
  host.handle("packages.install", (p) => {
    assert.equal(p.expectedRevision, 3);
    throw new Error("reply lost");
  });
  host.handle("packages.operation", (p) => ({
    operationId: p.operationId,
    status: ++operationPolls > 1 ? "committed" : "activating",
  }));
  const candidate = await plugin.packages.validate("/tmp/candidate");
  const trial = await candidate.trial();
  assert.equal(trial.status, "running");
  await trial.stop();
  const change = await candidate.install();
  assert.equal(await change.done, "committed");
  const install = host.sent("packages.install")[0]!;
  assert.equal(
    host.sent("packages.operation")[0]!.operationId,
    install.requestId,
  );
  await stop();
});

test("plugin id and update are available to plugin code", async () => {
  let id = "";
  const { stop } = await startPlugin(
    (plugin: Plugin) => {
      id = plugin.id;
    },
    { id: "demo.id" },
  );
  assert.equal(id, "demo.id");
  await stop();
});

test("packages: an unconfirmed trial cleanup fails with code cleanup and a retry re-sends stop", async () => {
  const { host, plugin, stop } = await startPlugin(() => {}, {
    capabilities: ["plugins.manage"],
  });
  host.handle("packages.validate", () => ({
    token: "t",
    digest: "d",
    apiVersion: "vestra.experimental/0.1",
    manifest: {
      id: "community.counter",
      version: "1.0.0",
      apiVersion: "vestra.experimental/0.1",
      entry: "main.js",
      capabilities: [],
    },
    checks: [],
    trustedExecution: true,
  }));
  host.handle("packages.trial", () => ({ trialId: 9, status: "running" }));
  let confirmed = false;
  host.handle("packages.stopTrial", () => ({
    trialId: 9,
    status: "failed",
    cleanupConfirmed: confirmed,
  }));
  const trial = await (await plugin.packages.validate("/tmp/c")).trial();
  await assert.rejects(
    trial.stop(),
    (error: HostError) => error.code === "cleanup",
  );
  confirmed = true;
  await trial.stop();
  assert.equal(
    host.sent("packages.stopTrial").length,
    2,
    "the second stop asked the host again",
  );
  await trial.stop();
  assert.equal(
    host.sent("packages.stopTrial").length,
    2,
    "a confirmed stop is final",
  );
  await stop();
});
