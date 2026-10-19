/** Real-host pixel replay check. Requires frozen opt-in headless frame evidence.
 * Diagnostic source snapshots are NOT frame/source fences; distinct source
 * corner markers identify A/B independently of observedSource metadata.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-processing-replay-review-"));
const frames = resolve(work, "frames"); await mkdir(frames);
for (const [name, marker] of [["a", "red"], ["b", "blue"]]) {
  const result = spawnSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i",
    `color=c=gray:size=320x180:rate=30,drawbox=x=0:y=0:w=64:h=64:color=${marker}:t=fill`,
    "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
    "-t", "16", "-c:v", "ffv1", "-c:a", "pcm_s16le", resolve(work, `${name}.mkv`)], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
}

async function activate(host) {
  const fs = await import("node:fs/promises"), path = await import("node:path");
  const work = process.env.VESTRA_REPLAY_REVIEW;
  const wait = ms => new Promise(r => setTimeout(r, ms));
  const phases = [], events = [];
  async function frameRecords() {
    try {
      const text = await fs.readFile(path.join(work, "frames", "frames.jsonl"), "utf8");
      return text.split("\n").slice(0, -1).filter(Boolean).map(JSON.parse);
    } catch (e) { if (e.code === "ENOENT") return []; throw e; }
  }
  async function ready(generation) {
    const deadline = Date.now() + 4000;
    while (Date.now() < deadline) {
      const s = await host.source.state();
      if (s.stamp.generation === generation && s.phase === "ready") return s;
      await wait(20);
    }
    throw Error("source ready timeout");
  }
  async function stableWindow(name, expectedSource) {
    await wait(700); // avoid treating transition/cached frames as settled output
    const first = (await frameRecords()).length;
    await wait(650);
    const last = (await frameRecords()).length;
    if (last - first < 2) throw Error("insufficient diagnostic samples for " + name);
    const source = await host.source.state();
    if (source.source !== expectedSource || source.phase !== "ready") throw Error("unexpected source while sampling " + name);
    if (name.startsWith("b-") && !source.paused) throw Error("B resumed while sampling " + name);
    phases.push({ name, first, last, source, processing: await host.processing.state() });
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, data => events.push({ event, data }));
    const a = path.join(work, "a.mkv"), b = path.join(work, "b.mkv");
    const opened = await host.source.submit({ kind: "open", source: a, startPaused: false });
    await ready(opened.ticket.stamp.generation);
    const generation = opened.ticket.stamp.generation;
    const node = brightness => ({ kind: "eq", brightness, contrast: 1, saturation: 1, gamma: 1 });
    await host.processing.replace({ stage: "video", resourceId: "tone", revision: 1, expectedGeneration: generation, nodes: [node(0)] });
    await host.processing.replace({ stage: "audio", resourceId: "norm", revision: 1, expectedGeneration: generation,
      nodes: [{ kind: "dynaudnorm", enabled: true, maxGain: 2, peak: 0.9 }] });
    await stableWindow("a-baseline", a);
    await host.processing.replace({ stage: "video", resourceId: "tone", revision: 2, expectedGeneration: generation, nodes: [node(0.2)] });
    await stableWindow("a-runtime-bright", a);
    const replaced = await host.source.submit({ kind: "open", source: b, startPaused: true });
    await ready(replaced.ticket.stamp.generation);
    // No new processing.replace here: B must replay the already confirmed value.
    await stableWindow("b-paused-replay", b);
    await host.processing.replace({ stage: "video", resourceId: "tone", revision: 3,
      expectedGeneration: replaced.ticket.stamp.generation, nodes: [node(0.3)] });
    await stableWindow("b-paused-with-norm-adjust", b);
    const beforeRestart = await host.source.state();
    await host.source.submit({ kind: "restart", expected: beforeRestart.stamp });
    const restarted = await ready(beforeRestart.stamp.generation);
    if (restarted.cycle <= beforeRestart.cycle) throw Error("restart must create new cycle at same generation");
    await host.source.submit({ kind: "control", control: { kind: "pause", paused: true }, expected: restarted.stamp });
    // No new processing.replace: same-generation reload must replay revision 3.
    await stableWindow("restart-b-replay", b);
    const final = await host.source.state();
    if (!final.paused) throw Error("B must remain paused");
    await fs.writeFile(path.join(work, "consumer-result.json"), JSON.stringify({ phases, events, final }, null, 2));
    console.error("PROCESSING_REPLAY_CONSUMER_DONE");
  } catch (error) {
    await fs.writeFile(path.join(work, "consumer-failure.json"), JSON.stringify({ error: String(error), stack: error.stack, phases, events }, null, 2)); throw error;
  }
}
const fixture = resolve(work, "consumer"); await mkdir(fixture);
await writeFile(resolve(fixture, "manifest.json"), JSON.stringify({ id: "review.processing-replay", apiVersion: "vestra.experimental/0.1", entry: "main.ts", capabilities: ["playback.read", "playback.control", "processing.video", "processing.audio", "events"] }));
await writeFile(resolve(fixture, "main.ts"), `export const activate=${activate.toString()};\n`);
const args = ["--headless", "--silent", "--seconds", "10", "--frame-evidence-dir", frames, "--plugin", fixture];
const child = spawnSync(resolve(import.meta.dirname, "../host/target/debug/vestra-host"), args, { encoding: "utf8", timeout: 17000, env: { ...process.env, VESTRA_REPLAY_REVIEW: work } });
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? ""); await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(resolve(work, "command.json"), JSON.stringify({ work, args, exitCode: child.status, signal: child.signal }, null, 2));
console.log(JSON.stringify({ work, exitCode: child.status }));
assert.equal(child.status, 0, child.stderr); assert.match(child.stderr, /PROCESSING_REPLAY_CONSUMER_DONE/);
const consumer = JSON.parse(await readFile(resolve(work, "consumer-result.json"), "utf8"));
const records = (await readFile(resolve(frames, "frames.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
async function inspect(record) {
  assert.equal(record.frameSourceAssociation, "none");
  const bytes = await readFile(resolve(frames, record.file));
  const header = /^(P6\s+)(\d+)\s+(\d+)\s+255\s/.exec(bytes.toString("ascii", 0, 80));
  assert.ok(header, "expected binary RGB PPM");
  const width = Number(header[2]), height = Number(header[3]), pixels = bytes.subarray(header[0].length);
  assert.equal(pixels.length, width * height * 3);
  function mean(x0, y0, x1, y1) {
    const sum = [0, 0, 0]; let n = 0;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
      const i = (y * width + x) * 3; for (let c = 0; c < 3; c++) sum[c] += pixels[i + c]; n++;
    }
    return sum.map(v => v / n);
  }
  const center = mean(Math.floor(width * .4), Math.floor(height * .4), Math.floor(width * .6), Math.floor(height * .6));
  const marker = mean(Math.floor(width * .02), Math.floor(height * .02), Math.floor(width * .12), Math.floor(height * .12));
  return { file: record.file, softwareSequence: record.softwareSequence, elapsedMs: record.elapsedMs, observedSource: record.observedSource, center, marker };
}
const analysis = {};
try {
  for (const phase of consumer.phases) {
    const samples = await Promise.all(records.slice(phase.first, phase.last).map(inspect));
    assert.ok(samples.length >= 2);
    for (const sample of samples) {
      if (phase.name.startsWith("a-")) assert.ok(sample.marker[0] > sample.marker[2] + 50, "actual A red marker required");
      else assert.ok(sample.marker[2] > sample.marker[0] + 50, "actual B blue marker required; observedSource alone is insufficient");
    }
    analysis[phase.name] = samples;
  }
  const baseline = analysis["a-baseline"].map(s => s.center.reduce((a, b) => a + b) / 3);
  const bright = analysis["a-runtime-bright"].map(s => s.center.reduce((a, b) => a + b) / 3);
  const replay = analysis["b-paused-replay"].map(s => s.center.reduce((a, b) => a + b) / 3);
  const pausedAdjust = analysis["b-paused-with-norm-adjust"].map(s => s.center.reduce((a, b) => a + b) / 3);
  const restartReplay = analysis["restart-b-replay"].map(s => s.center.reduce((a, b) => a + b) / 3);
  const average = a => a.reduce((x, y) => x + y) / a.length;
  assert.ok(Math.min(...bright) > Math.max(...baseline) + 20, "runtime brightness must actually change RGB");
  for (const value of replay) assert.ok(Math.abs(value - average(bright)) <= 3, "actual B gray must retain latest brightness");
  assert.ok(Math.min(...pausedAdjust) > Math.max(...replay) + 10, "paused video must actually refresh with norm installed");
  const bPhases = consumer.phases.filter(p => p.name.startsWith("b-"));
  assert.deepEqual(bPhases[0].source.stamp, bPhases[1].source.stamp);
  assert.ok(Math.abs(bPhases[0].source.position - bPhases[1].source.position) < 0.001, "paused adjustment must preserve reported media position");
  const restartPhase = consumer.phases.find(p => p.name === "restart-b-replay");
  assert.equal(restartPhase.source.stamp.generation, bPhases[1].source.stamp.generation);
  assert.ok(restartPhase.source.cycle > bPhases[1].source.cycle);
  for (const value of restartReplay) assert.ok(Math.abs(value - average(pausedAdjust)) <= 3, "same-generation restart must retain latest actual brightness");
  assert.equal(consumer.final.paused, true);
  const result = { pass: true, analysis, mean: { baseline: average(baseline), bright: average(bright), replay: average(replay), pausedAdjust: average(pausedAdjust), restartReplay: average(restartReplay) }, limits: ["Headless software RGB with actual red/blue source markers; observedSource snapshots are not frame PTS/source fences.", "Stable paused samples may repeat one softwareSequence; this proves settled content, not fresh decode per sample.", "Norm is installed on an actual audio track; no PCM/device measurement proves its audio effect or dynamic history.", "No physical display or visible first-frame fence proof."] };
  await writeFile(resolve(work, "result.json"), JSON.stringify(result, null, 2)); console.log(JSON.stringify({ pass: true, mean: result.mean }));
} catch (error) {
  await writeFile(resolve(work, "pixel-failure.json"), JSON.stringify({ pass: false, error: String(error), stack: error.stack, analysis, consumer }, null, 2)); throw error;
}
