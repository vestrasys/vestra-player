import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";
import { startPlugin } from "@vestra-player/sdk/testing";
import type { Plugin } from "@vestra-player/sdk";

const runner = resolve(import.meta.dirname, "../../runtime/runner.ts");

/** Run a plugin in the real runner and answer its requests like the host. */
async function run(
  source: string,
  reply: (method: string) => object = () => ({ result: { revision: 1 } }),
  runnerPath = runner,
  capabilities = ["commands"],
) {
  const directory = mkdtempSync(join(tmpdir(), "runner-"));
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({
      id: "runner.test",
      entry: "main.mjs",
      capabilities,
    }),
  );
  writeFileSync(join(directory, "main.mjs"), source);
  const child = spawn(process.execPath, [runnerPath, directory], {
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

test("the packaged runtime layout (runner + SDK package) runs a plugin", async () => {
  // What a packaged player ships as Resources/runtime (scripts/stage-runtime.mjs),
  // outside the workspace: the runner resolves the SDK from its own node_modules.
  const staged = join(mkdtempSync(join(tmpdir(), "runtime-")), "runtime");
  const stage = spawnSync(
    process.execPath,
    [resolve(import.meta.dirname, "../../scripts/stage-runtime.mjs"), staged],
    { encoding: "utf8" },
  );
  assert.equal(stage.status, 0, stage.stderr);
  // The SDK's Apache-2.0 license ships with its code (S7 decision 12).
  assert.match(
    readFileSync(
      join(staged, "node_modules/@vestra-player/sdk/LICENSE"),
      "utf8",
    ),
    /Apache License/,
  );
  const result = await run(
    "export default async function activate(plugin) { plugin.commands.add({ id: 'a', path: ['文件', 'A'], run() {} }); }",
    undefined,
    join(staged, "runner.js"),
  );
  assert.deepEqual(result, {
    event: "runtime.ready",
    methods: ["window.state", "commands.register"],
  });
  rmSync(staged, { recursive: true, force: true });
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

test("packages: install follows the operation to its end", async () => {
  const { host, plugin, stop } = await startPlugin(() => {}, {
    capabilities: ["plugins.manage"],
  });
  let operationPolls = 0;
  host.handle("packages.validate", () => ({
    token: "t",
    digest: "d",
    apiVersion: "vestra/1",
    manifest: {
      id: "community.counter",
      version: "1.0.0",
      apiVersion: "vestra/1",
      entry: "main.js",
      capabilities: [],
    },
    checks: ["slint"],
    trustedExecution: true,
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

test("a panel refused because another plugin holds its slot does not fail activation", async () => {
  // The panel's ready rejects with `conflict`; the plugin keeps its commands.
  const source = `export default async function activate(plugin) {
    const panel = plugin.panels.mount("sidebar", "ui/panel.slint", { component: "Panel", global: "PanelBridge" });
    const code = await panel.ready.then(() => "mounted", (error) => error.code);
    plugin.commands.add({ id: code, path: ["文件", code], run() {} });
  }`;
  const refuse = (detail: object) => (method: string) =>
    method === "ui.mount"
      ? {
          error: {
            code: "conflict",
            message: "侧栏已被 vestra.playlist 使用",
            ...detail,
          },
        }
      : { result: { revision: 1 } };
  const capabilities = ["commands", "ui.mount"];
  const occupied = await run(
    source,
    refuse({
      reason: "slot-occupied",
      slot: "sidebar",
      owner: "vestra.playlist",
    }),
    runner,
    capabilities,
  );
  assert.equal(occupied.event, "runtime.ready");
  assert.ok(occupied.methods.includes("ui.mount"));
  assert.ok(occupied.methods.includes("commands.register"));
  // Any other refusal of a start-up mount still fails activation.
  const other = await run(source, refuse({}), runner, capabilities);
  assert.equal(other.event, "runtime.failed");
});

test("a sequence another plugin provides does not fail activation", async () => {
  const source = `export default async function activate(plugin) {
    plugin.player.provideSequence({ canNext: true, canPrevious: false, next() {}, previous() {} });
    plugin.commands.add({ id: "a", path: ["文件", "A"], run() {} });
  }`;
  const refuse = (detail: object) => (method: string) =>
    method === "sequence.set"
      ? {
          error: {
            code: "conflict",
            message: "上一个/下一个已由 vestra.playlist 提供",
            ...detail,
          },
        }
      : { result: { revision: 1 } };
  const capabilities = ["commands", "playback.control"];
  const taken = await run(
    source,
    refuse({ reason: "sequence-taken", owner: "vestra.playlist" }),
    runner,
    capabilities,
  );
  assert.equal(taken.event, "runtime.ready");
  assert.ok(taken.methods.includes("sequence.set"));
  assert.ok(taken.methods.includes("commands.register"));
  const other = await run(source, refuse({}), runner, capabilities);
  assert.equal(other.event, "runtime.failed");
});

test("a file type or site another plugin resolves does not fail activation", async () => {
  const source = `export default async function activate(plugin) {
    const resolver = plugin.resolvers.add({ id: "nes", extensions: ["nes"], async resolve() { throw new Error("unused"); } });
    const code = await resolver.ready.then(() => "registered", (error) => error.code);
    plugin.commands.add({ id: code, path: ["文件", code], run() {} });
  }`;
  const refuse = (detail: object) => (method: string) =>
    method === "sourceResolvers.register"
      ? {
          error: {
            code: "conflict",
            message: "文件类型 .nes 已由 vestra.nes 解析",
            ...detail,
          },
        }
      : { result: { revision: 1 } };
  const capabilities = ["commands", "source.resolve"];
  const taken = await run(
    source,
    refuse({
      reason: "resolver-taken",
      owner: "vestra.nes",
      target: "文件类型 .nes",
    }),
    runner,
    capabilities,
  );
  assert.equal(taken.event, "runtime.ready");
  assert.ok(taken.methods.includes("sourceResolvers.register"));
  assert.ok(taken.methods.includes("commands.register"));
  const other = await run(source, refuse({}), runner, capabilities);
  assert.equal(other.event, "runtime.failed");
});

test("a plugin that logs to a closed stderr keeps running", async () => {
  // The player's stderr, which the runner inherits, can be a pipe whose
  // reader is gone (a launcher that captured it and exited).
  const directory = mkdtempSync(join(tmpdir(), "runner-"));
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({ id: "runner.test", entry: "main.mjs", capabilities: [] }),
  );
  writeFileSync(
    join(directory, "main.mjs"),
    `export default async function activate() {
      for (let i = 0; i < 3; i++) {
        console.error("log", i);
        process.stderr.write("raw " + i + "\\n");
        await new Promise((done) => setTimeout(done, 50));
      }
    }`,
  );
  const child = spawn(process.execPath, [runner, directory], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  child.stderr.destroy();
  const exited = new Promise((done) => child.once("exit", done));
  const lines = createInterface({ input: child.stdout });
  const event = await new Promise<string>((done) => {
    // A runner stopped by a failed log write never reports.
    void exited.then(() => done("exited"));
    lines.on("line", (line) => {
      const message = JSON.parse(line);
      if (message.event) return done(message.event);
      child.stdin.write(
        `${JSON.stringify({ id: message.id, result: { revision: 1 } })}\n`,
      );
    });
  });
  child.stdin.end();
  const code = await exited;
  assert.deepEqual({ event, code }, { event: "runtime.ready", code: 0 });
  rmSync(directory, { recursive: true, force: true });
});

test("a request longer than the player's 1 MiB line is refused with limit, not sent", async () => {
  // The player stops a plugin that writes a longer line (host/src/plugins.rs).
  const result = await run(
    `export default async function activate(plugin) {
      const code = await plugin.osd.show("x".repeat(1024 * 1024)).then(() => "sent", (e) => e.code);
      await plugin.osd.show(code);
    }`,
    () => ({ result: null }),
    runner,
    ["ui.osd"],
  );
  assert.deepEqual(result, {
    event: "runtime.ready",
    methods: ["window.state", "osd.show"],
  });
});
