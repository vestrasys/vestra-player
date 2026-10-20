// Real Pi Agent loop (local deterministic mock model, never a network model) and
// the real offline TypeScript compiler; the generated counter runs on the test host.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import { startPlugin } from "../../../sdk/plugin/testing.ts";
import { WorkshopJob, requirement } from "../engine.mjs";

const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-workshop-engine-"));
test.after(() => fs.rm(workspace, { recursive: true, force: true }));

for (const [version, step] of [["1.0.0", 1], ["1.1.0", 3]]) {
  test(`${version}: Pi fixes a real TS2322 and the generated counter counts by ${step}`, async () => {
    const job = new WorkshopJob(path.join(workspace, version));
    const result = await job.run(JSON.stringify({ kind: "native-counter", id: "community.counter", version, step }));
    assert.equal(result.actions, 8);
    assert.equal(result.turns, 3);
    assert.match(result.reports[0].stdout, /main\.ts\(2,7\): error TS2322/);
    assert.equal(result.reports[1].code, 0);
    const manifest = JSON.parse(await fs.readFile(path.join(result.directory, "manifest.json"), "utf8"));
    assert.deepEqual(manifest, {
      id: "community.counter",
      version,
      apiVersion: "vestra.experimental/0.1",
      entry: "main.js",
      capabilities: ["ui.window"],
    });

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
  assert.throws(() => requirement('{"kind":"native-counter","id":"community.counter","version":"1.0.0","step":3}'));
  assert.throws(() => requirement('{"kind":"native-counter","id":"other","version":"1.0.0","step":1}'));
  assert.throws(() => requirement("x".repeat(65537)), /64KiB/);
  assert.equal(requirement('{"kind":"native-counter","id":"community.counter","version":"1.1.0","step":3}').step, 3);
});
