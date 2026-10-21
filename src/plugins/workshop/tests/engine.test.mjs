// Real Pi Agent loop (local deterministic mock model, never a network model) and
// the real offline TypeScript compiler; the generated counter runs on the test host.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import test from "node:test";
import { startPlugin } from "@vestra-player/sdk/testing";
import { WorkshopJob, defaultRequirement, playerSdk, requirement } from "../engine.mjs";

const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-workshop-engine-"));
test.after(() => fs.rm(workspace, { recursive: true, force: true }));

for (const [version, step] of [["1.0.0", 1], ["1.1.0", 3]]) {
  test(`${version}: Pi fixes a real TS2322 and the generated counter counts by ${step}`, async () => {
    const job = new WorkshopJob(path.join(workspace, version));
    const result = await job.run(JSON.stringify({ kind: "native-counter", id: "workshop.counter", version, step }));
    assert.equal(result.actions, 8);
    assert.equal(result.turns, 3);
    assert.match(result.reports[0].stdout, /main\.ts\(2,7\): error TS2322/);
    assert.equal(result.reports[1].code, 0);
    const manifest = JSON.parse(await fs.readFile(path.join(result.directory, "manifest.json"), "utf8"));
    assert.deepEqual(manifest, {
      id: "workshop.counter",
      version,
      apiVersion: "vestra/1",
      entry: "main.js",
      capabilities: ["ui.window"],
      name: "独立计数器",
    });
    // Like any plugin: types from @vestra-player/sdk, here the player's own
    // declaration files (no SDK code) copied into the candidate.
    assert.match(await fs.readFile(path.join(result.directory, "main.ts"), "utf8"), /^import type \{ Plugin \} from '@vestra-player\/sdk';/);
    const types = path.join(result.directory, "node_modules/@vestra-player/sdk");
    const copied = (await fs.readdir(types, { recursive: true, withFileTypes: true })).filter((item) => item.isFile()).map((item) => item.name);
    assert.ok(copied.includes("index.d.ts") && copied.includes("package.json"));
    assert.deepEqual(copied.filter((name) => name !== "package.json" && !name.endsWith(".d.ts")), []);
    // The copy is the player's installed SDK version (1.1.x at the time of writing).
    const sdkVersion = JSON.parse(await fs.readFile(path.join(playerSdk(), "package.json"), "utf8")).version;
    assert.equal(JSON.parse(await fs.readFile(path.join(types, "package.json"), "utf8")).version, sdkVersion);
    // The candidate passes the same check a plugin author runs (1.1 manifest rules, type check).
    const cli = path.join(playerSdk(), "bin/vestra-plugin.mjs");
    const checked = await promisify(execFile)(process.execPath, [cli, "check", result.directory]);
    assert.match(checked.stdout, /type check passed/);
    assert.doesNotMatch(checked.stdout + checked.stderr, /prefix|前缀/i, "no id prefix hint");

    // Only the generated production JS, through the ordinary SDK v2 contract.
    const generated = await import(pathToFileURL(path.join(result.directory, "main.js")).href); // boundary-reviewed: generated candidate under test
    const { host, stop } = await startPlugin(generated.default, { directory: result.directory });
    const counter = host.window("counter");
    assert.equal(counter.file, "counter.slint");
    assert.equal(counter.props.count, "0");
    const seen = [];
    for (const callback of ["increase", "increase", "reset"]) {
      counter.emit(callback);
      await host.settle();
      seen.push(counter.props.count);
    }
    assert.deepEqual(seen, [String(step), String(step * 2), "0"]);
    await stop();
  });
}

test("requirements outside the finite counter schema are rejected", () => {
  assert.equal(requirement(defaultRequirement).id, "workshop.counter");
  assert.equal(requirement('{"kind":"native-counter","id":"community.counter","version":"1.0.0","step":1}').id, "community.counter", "requirement files from before 1.1");
  assert.throws(() => requirement('{"kind":"native-counter","id":"workshop.counter","version":"1.0.0","step":3}'));
  assert.throws(() => requirement('{"kind":"native-counter","id":"other","version":"1.0.0","step":1}'));
  assert.throws(() => requirement("x".repeat(65537)), /64KiB/);
  assert.equal(requirement('{"kind":"native-counter","id":"workshop.counter","version":"1.1.0","step":3}').step, 3);
});
