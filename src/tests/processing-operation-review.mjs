/** Independent real-host operation retention consumer. Run frozen host only.
 * No build/GUI, no mock backend, no claim about pixels/PCM/rollback DSP history.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-processing-operation-review-"));
const media = resolve(work, "source.mp4");
const made = spawnSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=20", "-t", "18", "-c:v", "libx264", "-preset", "ultrafast", media], { encoding: "utf8" });
assert.equal(made.status, 0, made.stderr);

async function consumer(host) {
  const assert = (await import("node:assert/strict")).default;
  const fs = await import("node:fs/promises"), path = await import("node:path");
  const work = process.env.VESTRA_PROCESSING_OPERATION_REVIEW;
  const p = host.processing, checks = {}, events = [];
  const wait = ms => new Promise(r => setTimeout(r, ms));
  const eq = brightness => ({ kind: "eq", brightness, contrast: 1, saturation: 1, gamma: 1 });
  const key = (resourceId, revision) => ({ stage: "video", resourceId, revision });
  const change = (resourceId, revision, expectedGeneration, brightness = .05) => ({ ...key(resourceId, revision), expectedGeneration, nodes: [eq(brightness)] });
  async function error(promise, code) {
    try { await promise; throw Error("expected rejection " + code); }
    catch (e) { assert.equal(e.code, code, String(e)); return { code: e.code, message: e.message }; }
  }
  async function ready(generation) {
    const deadline = Date.now() + 4000;
    while (Date.now() < deadline) {
      const s = await host.source.state();
      if (s.phase === "ready" && s.stamp.generation === generation) return s;
      await wait(20);
    }
    throw Error("actual source did not become ready");
  }
  async function helperRows() {
    const deadline = Date.now() + 4000;
    while (Date.now() < deadline) {
      const s = await p.state();
      if (s.video.some(r => r.resource.manifestId === "review.processing-operation-helper" && r.resource.resourceId === "ready")) return s;
      await wait(20);
    }
    throw Error("helper did not finish recording its independent result");
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, data => events.push({ event, data }));
    assert.equal((await host.source.state()).phase, "empty");
    assert.deepEqual(await p.operation(key("without-source", 1)), { status: "unknown" });
    const noSource = change("without-source", 1, 0);
    const noSourceError = await error(p.replace(noSource), "unavailable");
    assert.deepEqual(await p.operation(key("without-source", 1)), { status: "failed", error: noSourceError });
    assert.deepEqual(await error(p.replace(noSource), "unavailable"), noSourceError);
    checks.noSourceFailureRetained = { payload: noSource, error: noSourceError };

    const opened = await host.source.submit({ kind: "open", source: path.join(work, "source.mp4"), startPaused: false });
    const a = await ready(opened.ticket.stamp.generation), g = a.stamp.generation;
    const valid = change("live", 1, g);
    const applied = await p.replace(valid);
    assert.deepEqual(await p.operation(key("live", 1)), { status: "completed", result: applied });
    const beforeReplay = await p.state(), sourceBefore = await host.source.state();
    // Different object field insertion order still denotes the same typed payload.
    const reordered = { nodes: [{ gamma: 1, saturation: 1, contrast: 1, brightness: .05, kind: "eq" }], expectedGeneration: g, revision: 1, resourceId: "live", stage: "video" };
    assert.deepEqual(await p.replace(reordered), applied);
    await error(p.replace(change("live", 1, g, .2)), "conflict");
    assert.deepEqual(await p.state(), beforeReplay);
    assert.deepEqual((await host.source.state()).stamp, sourceBefore.stamp);
    checks.completedReplayAndConflict = { applied, confirmed: beforeReplay };

    const stale = change("live", 2, g + 1000, .1);
    const staleError = await error(p.replace(stale), "stale");
    assert.deepEqual(await p.operation(key("live", 2)), { status: "failed", error: staleError });
    assert.deepEqual(await error(p.replace(stale), "stale"), staleError);
    await error(p.replace(change("live", 2, g, .1)), "conflict");
    assert.deepEqual(await p.state(), beforeReplay);
    // Invalid pure data is rejected before admission and must not consume token 3.
    await error(p.replace({ ...change("live", 3, g), nodes: [{ kind: "gblur", sigma: 1, steps: 2 }] }), "invalid");
    assert.deepEqual(await p.operation(key("live", 3)), { status: "unknown" });
    const third = await p.replace(change("live", 3, g, .1));
    assert.equal(third.revision, 3);
    checks.failedReplayAndPureValidation = { staleError, third };

    const openedB = await host.source.submit({ kind: "open", source: path.join(work, "source.mp4"), startPaused: false });
    const b = await ready(openedB.ticket.stamp.generation);
    assert.ok(b.stamp.generation > g);
    const current = await p.state(), marker = events.length;
    assert.deepEqual(await p.replace(valid), applied, "old token returns old receipt even across a source change");
    await error(p.replace(change("live", 1, b.stamp.generation)), "conflict");
    assert.deepEqual(await p.state(), current, "old receipt must not overwrite current revision/source metadata");
    assert.deepEqual((await host.source.state()).stamp, b.stamp);
    await wait(120);
    assert.ok(!events.slice(marker).some(e => e.event === "playback.backend" && /StartFile|FileLoaded/.test(e.data.event)), "replay cannot load source again");
    checks.oldSourceReplayNoEffect = { original: applied, b, current };

    // Cheap actual dynamic rejections exercise terminal retention without rebuilding graphs.
    const retainedState = await p.state();
    const expiryPayload = revision => change("expiry", revision, b.stamp.generation + 1000);
    for (let revision = 1; revision <= 514; revision++) await error(p.replace(expiryPayload(revision)), "stale");
    assert.deepEqual(await p.operation(key("expiry", 1)), { status: "unknownExpired" });
    await error(p.replace(expiryPayload(1)), "unknown-expired");
    assert.equal((await p.operation(key("expiry", 514))).status, "failed");
    assert.deepEqual(await p.operation(key("expiry", 515)), { status: "unknown" });
    assert.deepEqual(await p.state(), retainedState);
    checks.retentionEvictsWithoutReexecution = { terminals: 514, oldest: await p.operation(key("expiry", 1)), latest: await p.operation(key("expiry", 514)) };

    // Three previously admitted keys + 61 more = 64 lifetime keys for this owner.
    for (let i = 0; i < 61; i++) await error(p.replace(change(`limit-${i}`, 1, b.stamp.generation + 1000)), "stale");
    await error(p.replace(change("over-limit", 1, b.stamp.generation + 1000)), "busy");
    assert.deepEqual(await p.operation(key("over-limit", 1)), { status: "unknown" });
    assert.deepEqual(await p.state(), retainedState);
    checks.keyLimitDoesNotAdmit = { query: await p.operation(key("over-limit", 1)) };

    const helper = path.join(work, "helper");
    await host.plugins.load(helper);
    const first = await helperRows();
    const owner1 = first.video.find(r => r.resource.manifestId === "review.processing-operation-helper").owner;
    await host.plugins.disable("review.processing-operation-helper");
    await host.plugins.load(helper);
    const second = await helperRows();
    const owner2 = second.video.find(r => r.resource.manifestId === "review.processing-operation-helper").owner;
    assert.notEqual(owner1, owner2);
    assert.ok(!second.video.some(r => r.owner === owner1));
    await host.plugins.disable("review.processing-operation-helper");
    const final = await p.state();
    assert.ok(!final.video.some(r => r.owner === owner2));
    assert.deepEqual(final.video, retainedState.video);
    assert.deepEqual(final.backendUncertain, {});
    checks.ownerNamespace = { owner1, owner2, final };
    await fs.writeFile(path.join(work, "result.json"), JSON.stringify({ pass: true, checks, events,
      limits: ["Real host/backend admission and public retained results only; completed remains backend acceptance.", "Expired paths use genuine generation-stale rejections; no arbitrary mpv failure injection or actual rollback/DSP claim.", "Pending saturation is not forced by this synchronous backend; no native pixel, device audio or frame fence evidence."] }, null, 2));
    console.error("PROCESSING_OPERATION_REVIEW_PASS");
  } catch (error) {
    await fs.writeFile(path.join(work, "failure.json"), JSON.stringify({ pass: false, error: String(error), stack: error.stack, checks, events }, null, 2)); throw error;
  }
}
async function helper(host) {
  const fs = await import("node:fs/promises"), path = await import("node:path");
  const p = host.processing, token = { stage: "video", resourceId: "live", revision: 1 };
  const prior = await p.operation(token), s = await host.source.state();
  const result = await p.replace({ ...token, expectedGeneration: s.stamp.generation, nodes: [{ kind: "eq", brightness: -.03, contrast: 1, saturation: 1, gamma: 1 }] });
  const state = await p.state(), owner = state.video.find(r => r.resource.manifestId === "review.processing-operation-helper").owner;
  const query = await p.operation(token);
  await fs.appendFile(path.join(process.env.VESTRA_PROCESSING_OPERATION_REVIEW, "helper-results.jsonl"), JSON.stringify({ owner, prior, result, query }) + "\n");
  // Only expose the public ready marker after writing the independent evidence.
  await p.replace({ ...token, resourceId: "ready", expectedGeneration: s.stamp.generation, nodes: [{ kind: "eq", brightness: 0, contrast: 1, saturation: 1, gamma: 1 }] });
}
for (const [name, activate] of [["consumer", consumer], ["helper", helper]]) {
  const directory = resolve(work, name); await mkdir(directory);
  await writeFile(resolve(directory, "manifest.json"), JSON.stringify({ id: `review.processing-operation-${name}`, apiVersion: "vestra.experimental/0.1", entry: "main.mjs", capabilities: ["playback.read", "processing.video", ...(name === "consumer" ? ["playback.control", "events", "plugins.manage"] : [])] }));
  await writeFile(resolve(directory, "main.mjs"), `export const activate=${activate.toString()};\n`);
}
const args = ["--headless", "--silent", "--seconds", "16", "--plugin", resolve(work, "consumer")];
const child = spawnSync(resolve(import.meta.dirname, "../host/target/debug/vestra-host"), args, { encoding: "utf8", timeout: 24000, env: { ...process.env, VESTRA_PROCESSING_OPERATION_REVIEW: work } });
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? ""); await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(resolve(work, "command.json"), JSON.stringify({ work, args, exitCode: child.status, signal: child.signal }, null, 2));
console.log(JSON.stringify({ work, exitCode: child.status })); assert.equal(child.status, 0, child.stderr); assert.match(child.stderr, /PROCESSING_OPERATION_REVIEW_PASS/);
const result = JSON.parse(await readFile(resolve(work, "result.json"), "utf8"));
const helpers = (await readFile(resolve(work, "helper-results.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
assert.equal(helpers.length, 2);
assert.notEqual(helpers[0].owner, helpers[1].owner);
for (const h of helpers) { assert.deepEqual(h.prior, { status: "unknown" }); assert.deepEqual(h.query, { status: "completed", result: h.result }); }
assert.equal(result.pass, true);
await writeFile(resolve(work, "external-review.json"), JSON.stringify({ pass: true, helpers, scope: "External harness checks fresh helper token namespace after actual host exit" }, null, 2));
