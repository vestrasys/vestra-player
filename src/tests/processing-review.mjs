/** Independent processing.resources candidate wire consumer. Run frozen host only.
 * Uses real JSONL/Node/Rust/libmpv; no Cargo, GUI, pixel or PCM claims.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const work = await mkdtemp(resolve(tmpdir(), "vestra-processing-review-"));
const media = resolve(work, "clock.mp4");
const made = spawnSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i",
  "testsrc2=size=320x180:rate=30", "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
  "-t", "18", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", media], { encoding: "utf8" });
assert.equal(made.status, 0, made.stderr);

// Exercise the public candidate JSONL wire independently of processing SDK calls,
// not a private mpv command API. IDs are disjoint from runner's SDK request IDs.
async function wire() {
  const { createInterface } = await import("node:readline");
  const input = createInterface({ input: process.stdin });
  let next = 1000000;
  const pending = new Map();
  input.on("line", line => {
    const message = JSON.parse(line), entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id); clearTimeout(entry.timer);
    if (message.error) entry.reject(Object.assign(new Error(message.error.message), message.error));
    else entry.resolve(message.result);
  });
  return (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`${method} timed out; outcome unknown`)); }, 8000);
    pending.set(id, { resolve, reject, timer });
    process.stdout.write(JSON.stringify({ id, method, params }) + "\n");
  });
}

async function consumer(host) {
  const assert = (await import("node:assert/strict")).default;
  const fs = await import("node:fs/promises"), path = await import("node:path");
  const work = process.env.VESTRA_PROCESSING_REVIEW, request = await wire();
  const checks = {}, events = [], wait = ms => new Promise(r => setTimeout(r, ms));
  const state = () => request("processing.state");
  async function until(predicate, label) {
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline) { const s = await state(); if (predicate(s)) return s; await wait(25); }
    throw Error(label + ": " + JSON.stringify(await state()));
  }
  const eq = brightness => ({ kind: "eq", brightness, contrast: 1, saturation: 1, gamma: 1 });
  let generation;
  const change = (stage, revision, nodes, extra = {}) => request("processing.replace", {
    stage, resourceId: "same-local-id", revision, expectedGeneration: generation, nodes, ...extra,
  });
  try {
    assert.ok((await host.capabilities()).experimentalCapabilities.includes("processing.resources/0.1"));
    for (const name of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(name, data => events.push({ name, data }));
    const opened = await host.source.submit({ kind: "open", source: path.join(work, "clock.mp4"), startPaused: false });
    generation = opened.ticket.stamp.generation;
    const deadline = Date.now() + 5000;
    while ((await host.source.state()).phase !== "ready") {
      if (Date.now() > deadline) throw Error("source not ready");
      await wait(20);
    }
    const empty = await state();
    assert.deepEqual(empty.video, []);
    assert.deepEqual(empty.audio, []);
    assert.deepEqual(empty.backendUncertain, {});
    assert.equal(empty.source.generation, generation);
    assert.equal(empty.source.phase, "ready");
    assert.equal(empty.source.load.generation, generation);
    assert.ok(Number.isInteger(empty.source.load.entry));
    assert.ok(empty.source.load.cycle > 0);
    for (const stage of ["video", "audio"]) {
      assert.equal(empty.source.stages[stage].status, "applied");
      assert.equal(empty.source.stages[stage].error, null);
    }
    const video = await change("video", 1, [eq(0.1), { kind: "gblur", sigma: 0.5, steps: 3 }]);
    assert.equal(video.status, "applied");
    assert.equal(video.generation, generation);
    const audioNodes = [
      { kind: "volume", gainDb: -6 },
      { kind: "equalizer", frequency: 1000, q: 1, gainDb: 2 },
      { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 },
      { kind: "dynaudnorm", enabled: true, maxGain: 2, peak: 0.9 },
    ];
    assert.equal((await change("audio", 1, audioNodes)).status, "applied");
    const initial = await state();
    assert.deepEqual(initial.audio[0].resource.nodes, audioNodes, "camelCase descriptors must roundtrip through Rust");
    const owner = initial.video[0].owner;
    assert.equal(initial.audio[0].owner, owner);
    assert.equal(initial.video[0].resource.manifestId, "review.processing-consumer");
    checks.serdeAndOwnedDescriptors = initial;

    const helper = path.join(work, "helper");
    await host.plugins.load(helper);
    const combined = await until(s => s.video.length === 2 && s.audio.length === 2, "two owners");
    assert.equal(new Set(combined.video.map(r => r.owner)).size, 2);
    assert.equal(combined.video.filter(r => r.resource.resourceId === "same-local-id").length, 2);
    const helperOwner = combined.video.find(r => r.owner !== owner).owner;
    const ownRows = s => ({ video: s.video.filter(r => r.owner === owner), audio: s.audio.filter(r => r.owner === owner) });
    const ownBefore = ownRows(combined);
    await host.plugins.disable("review.processing-helper");
    const remaining = await until(s => s.video.length === 1 && s.audio.length === 1, "helper cleanup");
    assert.deepEqual(ownRows(remaining), ownBefore);
    assert.ok(!remaining.video.some(r => r.owner === helperOwner));
    assert.ok(!remaining.audio.some(r => r.owner === helperOwner));
    checks.ownerCleanup = { combined, remaining };

    async function rejectedUnchanged(promise, code, before) {
      await assert.rejects(promise, e => e.code === code);
      assert.deepEqual(await state(), before, "rejection must preserve confirmed public resources");
    }
    await rejectedUnchanged(change("video", 1, [eq(-0.2)]), "conflict", remaining);
    await rejectedUnchanged(change("video", 2, [eq(-0.2)], { expectedGeneration: generation + 100 }), "stale", remaining);
    await rejectedUnchanged(change("audio", 2, [{ kind: "volume", gain_db: -6 }]), "invalid", remaining);
    await rejectedUnchanged(change("video", 2, [{ kind: "gblur", sigma: 2, steps: 2 }]), "invalid", remaining);
    checks.rejectedCandidatesPreservePublicResources = await state();

    const before = await host.source.state(), frameBefore = await host.playback.state();
    const marker = events.length, started = Date.now();
    for (let revision = 3; revision <= 32; revision++) {
      const nodes = [eq((revision % 5) * 0.04), { kind: "gblur", sigma: 0.5, steps: 3 }];
      const result = await change("video", revision, nodes);
      assert.equal(result.status, "applied");
      assert.equal(result.revision, revision);
      await wait(20);
    }
    await wait(180);
    const after = await host.source.state(), frameAfter = await host.playback.state();
    assert.deepEqual(after.stamp, before.stamp, "processing cannot create source control intents");
    assert.equal(after.cycle, before.cycle);
    assert.equal(after.phase, "ready");
    assert.ok(after.position > before.position + 0.25);
    assert.ok(frameAfter.videoFrames > frameBefore.videoFrames + 5);
    assert.ok(!events.slice(marker).some(e => e.name === "playback.backend" && /StartFile|FileLoaded/.test(e.data.event)), "parameter updates must not reload source");
    const final = await state();
    assert.equal(final.video[0].revision, 32);
    checks.parameterCommits = { before, after, frameBefore, frameAfter, ms: Date.now() - started, final };
    assert.ok(!events.some(e => e.name === "host.error"), "no hidden backend cleanup fault");
    await fs.writeFile(path.join(work, "result.json"), JSON.stringify({ pass: true, checks, events,
      limits: ["Public typed descriptors, backend acceptance and source/render progress only; no pixel/PCM or device audio proof.",
        "Invalid/stale requests fail before backend changes. This does not exercise partial backend failure rollback or preservation of actual old effects/DSP history.",
        "processing.state collection order is not used as proof of actual backend filter order."] }, null, 2));
    console.error("PROCESSING_REVIEW_PASS");
  } catch (error) {
    await fs.writeFile(path.join(work, "failure.json"), JSON.stringify({ pass: false, error: String(error), stack: error.stack, checks, events }, null, 2));
    throw error;
  }
}

async function helper(host) {
  const request = await wire(), s = await host.source.state();
  for (const [stage, nodes] of [
    ["video", [{ kind: "eq", brightness: -0.05, contrast: 1, saturation: 1, gamma: 1 }]],
    ["audio", [{ kind: "volume", gainDb: -3 }]],
  ]) await request("processing.replace", { stage, resourceId: "same-local-id", revision: 1, expectedGeneration: s.stamp.generation, nodes });
}

for (const [name, activate] of [["consumer", consumer], ["helper", helper]]) {
  const directory = resolve(work, name); await mkdir(directory);
  await writeFile(resolve(directory, "manifest.json"), JSON.stringify({ id: `review.processing-${name}`, apiVersion: "vestra.experimental/0.1", entry: "main.ts",
    capabilities: ["playback.read", "processing.video", "processing.audio", ...(name === "consumer" ? ["playback.control", "events", "plugins.manage"] : [])] }));
  await writeFile(resolve(directory, "main.ts"), `const wire=${wire.toString()};\nexport const activate=${activate.toString()};\n`);
}
const args = ["--headless", "--silent", "--seconds", "14", "--plugin", resolve(work, "consumer")];
const result = spawnSync(resolve(import.meta.dirname, "../host/target/debug/vestra-host"), args, { encoding: "utf8", timeout: 22000, env: { ...process.env, VESTRA_PROCESSING_REVIEW: work } });
await writeFile(resolve(work, "host.stderr.log"), result.stderr ?? "");
await writeFile(resolve(work, "host.stdout.log"), result.stdout ?? "");
await writeFile(resolve(work, "command.json"), JSON.stringify({ work, args, exitCode: result.status, signal: result.signal }, null, 2));
console.log(JSON.stringify({ work, exitCode: result.status }));
assert.equal(result.status, 0, result.stderr);
assert.match(result.stderr, /PROCESSING_REVIEW_PASS/);
assert.equal(JSON.parse(await readFile(resolve(work, "result.json"), "utf8")).pass, true);
