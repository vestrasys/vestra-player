/** Experimental public raw-PNG smoke. Not full M3 or independent acceptance. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const work = await mkdtemp(resolve(tmpdir(), "vestra-media-raw-author-"));
const anamorphic = process.argv.includes("--sar");
const rotated = process.argv.includes("--rotation");
assert.ok(!(anamorphic && rotated), "Test SAR and rotation separately");
const fixture = resolve(work, "source.mp4");
const plugin = resolve(work, "consumer");
const artifacts = resolve(work, "artifacts");
await mkdir(plugin);
await mkdir(artifacts);
const made = spawnSync(
  "ffmpeg",
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "color=c=red:size=320x180:rate=30" + (anamorphic ? ",setsar=2" : ""),
    "-t",
    "8",
    "-c:v",
    "libx264",
    rotated ? resolve(work, "unrotated.mp4") : fixture,
  ],
  { encoding: "utf8" },
);
assert.equal(made.status, 0, made.stderr);
if (rotated) {
  const rotation = spawnSync(
    "ffmpeg",
    [
      "-v",
      "error",
      "-display_rotation:v:0",
      "90",
      "-i",
      resolve(work, "unrotated.mp4"),
      "-c",
      "copy",
      fixture,
    ],
    { encoding: "utf8" },
  );
  assert.equal(rotation.status, 0, rotation.stderr);
}
const fixtureProbe = spawnSync(
  "ffprobe",
  ["-v", "error", "-show_streams", "-of", "json", fixture],
  { encoding: "utf8" },
);
assert.equal(fixtureProbe.status, 0, fixtureProbe.stderr);
await writeFile(resolve(work, "fixture-metadata.json"), fixtureProbe.stdout);
const input = JSON.parse(fixtureProbe.stdout).streams[0];
if (anamorphic) assert.equal(input.sample_aspect_ratio, "2:1");
if (rotated)
  assert.ok(
    input.side_data_list?.some((data) => Math.abs(data.rotation) === 90),
    "fixture must really contain rotation metadata",
  );

async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_MEDIA_RAW_SMOKE;
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const observed = [];
  async function terminal(id) {
    const deadline = Date.now() + 4000;
    while (Date.now() < deadline) {
      const job = await host.jobs.state(id);
      observed.push(job);
      if (["completed", "cancelled", "failed"].includes(job.status)) return job;
      await wait(20);
    }
    throw Error("raw job did not reach a terminal state");
  }
  try {
    const opened = await host.source.submit({
      kind: "open",
      source: path.join(work, "source.mp4"),
      startPaused: true,
    });
    const deadline = Date.now() + 4000;
    let source;
    do {
      source = await host.source.state();
      if (source.phase === "ready") break;
      await wait(20);
    } while (Date.now() < deadline);
    assert.equal(source.phase, "ready");
    assert.equal(source.stamp.generation, opened.ticket.stamp.generation);
    assert.equal(source.capabilities.snapshotSource, true);
    const destination = path.join(work, "artifacts", "raw.png");
    const capture = { operationId: 1, expected: source.stamp, time: 1 };
    const snapshot = await host.snapshot.capture(capture);
    assert.deepEqual(await host.snapshot.capture(capture), snapshot);
    const start = {
      operationId: 2,
      snapshotId: snapshot.snapshotId,
      kind: "rawPng",
      destination,
    };
    const started = await host.jobs.start(start);
    assert.deepEqual(await host.jobs.start(start), started);
    await host.snapshot.release(snapshot.snapshotId, 3);
    const completed = await terminal(started.jobId);
    assert.equal(completed.status, "completed", JSON.stringify(completed));
    assert.equal(completed.publicationStarted, true);
    const bytes = await fs.readFile(destination);
    const second = await host.snapshot.capture({
      operationId: 4,
      expected: source.stamp,
      time: 2,
    });
    const blocked = await host.jobs.start({
      operationId: 5,
      snapshotId: second.snapshotId,
      kind: "rawPng",
      destination,
    });
    const failed = await terminal(blocked.jobId);
    assert.equal(failed.status, "failed");
    assert.equal(
      failed.publicationStarted,
      true,
      "publication attempt stays observable after no-clobber failure",
    );
    assert.deepEqual(await fs.readFile(destination), bytes);
    await host.snapshot.release(second.snapshotId, 6);
    const after = await host.source.state();
    assert.equal(after.paused, true);
    assert.equal(after.position, source.position);
    assert.deepEqual(after.stamp, source.stamp);
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        { pass: true, snapshot, completed, failed, observed, source, after },
        null,
        2,
      ),
    );
    console.error("MEDIA_RAW_SMOKE_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        { error: String(error), stack: error.stack, observed },
        null,
        2,
      ),
    );
    throw error;
  }
}

await writeFile(
  resolve(plugin, "manifest.json"),
  JSON.stringify({
    id: "author.media-raw",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "playback.read",
      "playback.control",
      "media.read",
      "media.control",
    ],
  }),
);
await writeFile(
  resolve(plugin, "main.mjs"),
  `export const activate = ${activate.toString()};\n`,
);
const args = ["--headless", "--silent", "--seconds", "7", "--plugin", plugin];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 15000,
    env: { ...process.env, VESTRA_MEDIA_RAW_SMOKE: work },
  },
);
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(
  resolve(work, "command.json"),
  JSON.stringify(
    { work, args, exitCode: child.status, signal: child.signal },
    null,
    2,
  ),
);
console.log(JSON.stringify({ work, exitCode: child.status }));
assert.equal(child.status, 0, child.stderr);
assert.match(child.stderr, /MEDIA_RAW_SMOKE_DONE/);
const png = resolve(artifacts, "raw.png");
const decoded = spawnSync(
  "ffmpeg",
  [
    "-v",
    "error",
    "-i",
    png,
    "-frames:v",
    "1",
    "-f",
    "rawvideo",
    "-pix_fmt",
    "rgb24",
    "-",
  ],
  { maxBuffer: 1024 * 1024 },
);
assert.equal(decoded.status, 0, decoded.stderr?.toString());
const width = rotated ? 180 : anamorphic ? 640 : 320;
const height = rotated ? 320 : 180;
const probe = spawnSync(
  "ffprobe",
  ["-v", "error", "-show_streams", "-of", "json", png],
  { encoding: "utf8" },
);
assert.equal(probe.status, 0, probe.stderr);
const stream = JSON.parse(probe.stdout).streams[0];
assert.equal(stream.width, width);
assert.equal(stream.height, height);
assert.equal(decoded.stdout.length, width * height * 3);
assert.ok(
  decoded.stdout[0] > 240 && decoded.stdout[1] < 10 && decoded.stdout[2] < 10,
);
assert.deepEqual(
  await readdir(artifacts),
  ["raw.png"],
  "owned temporary output removed on no-clobber failure",
);
const result = {
  pass: true,
  work,
  width,
  height,
  anamorphic,
  rotated,
  firstRGB: [...decoded.stdout.subarray(0, 3)],
  sha256: createHash("sha256")
    .update(await readFile(png))
    .digest("hex"),
  limits: [
    "Author smoke, not independent acceptance or processed export coverage.",
    "No cancellation, owner death, worker-load continuity or exact frame PTS proof.",
  ],
};
await writeFile(resolve(work, "result.json"), JSON.stringify(result, null, 2));
console.log(JSON.stringify(result));
