/** Finite author-only public MP4 smoke. Independent acceptance is separate. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-media-mp4-author-"));
const ff = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg";
await mkdir(resolve(work, "consumer"));
const made = spawnSync(
  ff,
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "testsrc2=size=320x180:rate=30",
    "-f",
    "lavfi",
    "-i",
    "sine=frequency=440:sample_rate=48000",
    "-f",
    "lavfi",
    "-i",
    "aevalsrc=exprs='if(lt(t,1.5),0.04,0.4)*sin(2*PI*880*t)':s=48000:c=stereo",
    "-map",
    "0:v",
    "-map",
    "1:a",
    "-map",
    "2:a",
    "-t",
    "6",
    "-c:v",
    "ffv1",
    "-c:a",
    "pcm_s16le",
    "-metadata:s:a:0",
    "title=Tone 440",
    "-metadata:s:a:1",
    "title=Tone 880",
    "-disposition:a:0",
    "default",
    "-disposition:a:1",
    "0",
    resolve(work, "a.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(made.status, 0, made.stderr);
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_MEDIA_MP4_SMOKE,
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  let op = 0,
    source;
  async function job(snapshot, kind, name) {
    const started = await host.jobs.start({
      operationId: ++op,
      snapshotId: snapshot.snapshotId,
      kind,
      end: 4.3,
      destination: path.join(work, name + ".mp4"),
    });
    await host.snapshot.release(snapshot.snapshotId, ++op);
    for (let i = 0; i < 400; i++) {
      const state = await host.jobs.state(started.jobId);
      if (["completed", "failed", "cancelled"].includes(state.status)) {
        assert.equal(state.status, "completed", JSON.stringify(state));
        return state;
      }
      await wait(20);
    }
    throw Error("job timeout");
  }
  async function capture(mode, audio) {
    source = await host.source.state();
    const processing = await host.processing.state();
    return host.snapshot.capture({
      operationId: ++op,
      mode,
      expected: source.stamp,
      time: 2.3,
      audio,
      ...(mode === "processed"
        ? {
            expectedProcessingRevision: processing.configurationRevision,
            includeSubtitles: false,
          }
        : {}),
    });
  }
  try {
    const opened = await host.source.submit({
      kind: "open",
      source: path.join(work, "a.mkv"),
      startPaused: true,
    });
    for (let i = 0; i < 200; i++) {
      source = await host.source.state();
      if (
        source.phase === "ready" &&
        source.stamp.generation === opened.ticket.stamp.generation &&
        source.capabilities.snapshotSource
      )
        break;
      await wait(20);
    }
    assert.equal(source.phase, "ready");
    await wait(120);
    assert.equal(source.audioTracks.length, 2);
    const second = source.audioTracks[1];
    assert.equal(second.streamIndex, 2);
    await host.processing.replace({
      stage: "audio",
      resourceId: "audio",
      revision: 1,
      expectedGeneration: source.stamp.generation,
      offsetMs: 200,
      nodes: [
        { kind: "volume", gainDb: -3 },
        { kind: "equalizer", frequency: 1000, q: 1, gainDb: 2 },
        { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 },
        { kind: "dynaudnorm", enabled: true, maxGain: 2, peak: 0.9 },
      ],
    });
    const frozen = await capture("processed", {
      kind: "track",
      trackId: second.trackId,
    });
    assert.equal(frozen.audioOffsetMs, 200);
    assert.equal(frozen.audio.streamIndex, 2);
    await host.processing.replace({
      stage: "audio",
      resourceId: "audio",
      revision: 2,
      expectedGeneration: source.stamp.generation,
      offsetMs: -200,
      nodes: [],
    });
    const negative = await capture("processed", {
      kind: "track",
      trackId: second.trackId,
    });
    assert.equal(negative.processing.audio.length, 0);
    assert.equal(negative.audioOffsetMs, -200);
    // Device gain/mute are shared output controls, excluded from all recipes.
    await host.playback.set("volume", 0);
    await host.playback.set("muted", true);
    const raw = await capture("raw", {
      kind: "track",
      trackId: second.trackId,
    });
    const none = await capture("raw", { kind: "none" });
    assert.equal(none.audio, null);
    const results = [];
    results.push(await job(frozen, "processedMp4", "processed"));
    results.push(await job(negative, "processedMp4", "negative"));
    results.push(await job(raw, "rawMp4", "raw"));
    results.push(await job(none, "rawMp4", "silent"));
    const revision = (await host.processing.state()).configurationRevision;
    await host.processing.replace({
      stage: "audio",
      resourceId: "audio",
      revision: 3,
      expectedGeneration: source.stamp.generation,
      offsetMs: 0,
      nodes: [],
    });
    assert.equal((await host.processing.state()).audio.length, 0);
    assert.equal((await host.processing.state()).audioOffsetMs, 0);
    assert.ok((await host.processing.state()).configurationRevision > revision);
    const replay = await host.processing.replace({
      stage: "audio",
      resourceId: "audio",
      revision: 2,
      expectedGeneration: source.stamp.generation,
      offsetMs: -200,
      nodes: [],
    });
    assert.equal(replay.revision, 2);
    assert.equal(
      (await host.processing.state()).audioOffsetMs,
      0,
      "old receipt must not reapply reset offset",
    );
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        {
          pass: true,
          source,
          second,
          frozen,
          negative,
          raw,
          none,
          results,
          after: await host.source.state(),
        },
        null,
        2,
      ),
    );
    console.error("MEDIA_MP4_SMOKE_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify({ error: String(error), stack: error.stack }, null, 2),
    );
    throw error;
  }
}
await writeFile(
  resolve(work, "consumer/manifest.json"),
  JSON.stringify({
    id: "author.media-mp4",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "playback.read",
      "playback.control",
      "processing.audio",
      "subtitles.read",
      "media.read",
      "media.control",
    ],
  }),
);
await writeFile(
  resolve(work, "consumer/main.mjs"),
  `export const activate=${activate.toString()};\n`,
);
const args = [
  "--headless",
  "--silent",
  "--seconds",
  "10",
  "--plugin",
  resolve(work, "consumer"),
];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 20000,
    env: { ...process.env, VESTRA_MEDIA_MP4_SMOKE: work },
  },
);
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(
  resolve(work, "command.json"),
  JSON.stringify(
    { work, args, status: child.status, signal: child.signal },
    null,
    2,
  ),
);
console.log({ work, status: child.status });
assert.equal(child.status, 0, child.stderr);
assert.match(child.stderr, /MEDIA_MP4_SMOKE_DONE/);
const consumer = JSON.parse(
  await readFile(resolve(work, "consumer-result.json"), "utf8"),
);
const analyses = {};
for (const name of ["processed", "negative", "raw", "silent"]) {
  const probe = spawnSync(
    "/opt/homebrew/opt/ffmpeg-full/bin/ffprobe",
    [
      "-v",
      "error",
      "-show_streams",
      "-show_format",
      "-of",
      "json",
      resolve(work, name + ".mp4"),
    ],
    { encoding: "utf8" },
  );
  assert.equal(probe.status, 0, probe.stderr);
  const meta = JSON.parse(probe.stdout);
  await writeFile(resolve(work, name + "-probe.json"), probe.stdout);
  assert.equal(meta.streams[0].codec_name, "h264");
  assert.ok(Math.abs(+meta.format.duration - 2) < 0.05);
  assert.equal(meta.streams.length, name === "silent" ? 1 : 2);
  if (name === "silent") continue;
  assert.equal(meta.streams[1].codec_name, "aac");
  assert.equal(+meta.streams[1].sample_rate, 48000);
  const decoded = spawnSync(
    ff,
    [
      "-v",
      "error",
      "-i",
      resolve(work, name + ".mp4"),
      "-map",
      "0:a:0",
      "-t",
      "2",
      "-f",
      "f32le",
      "-",
    ],
    { maxBuffer: 2 * 48000 * 2 * 4 + 8192 },
  );
  assert.equal(decoded.status, 0, decoded.stderr?.toString());
  await writeFile(resolve(work, name + ".f32"), decoded.stdout);
  const graph =
    name === "processed"
      ? "volume=0.7079457843841379,equalizer=frequency=1000:width_type=q:width=1:gain=2,treble=frequency=3000:width_type=s:width=.5:gain=1,dynaudnorm=framelen=100:gausssize=5:maxgain=2:peak=.9:enable=1"
      : "anull";
  const offset = name === "processed" ? 0.2 : name === "negative" ? -0.2 : 0;
  const ref = spawnSync(
    ff,
    [
      "-v",
      "error",
      "-threads",
      "1",
      "-i",
      resolve(work, "a.mkv"),
      "-map",
      "0:2",
      "-af",
      `${graph},asetpts=PTS+(${offset})/TB,aresample=48000:async=1:first_pts=0,apad,atrim=start=2.3:end=4.3,asetpts=PTS-STARTPTS`,
      "-f",
      "f32le",
      "-",
    ],
    { maxBuffer: 2 * 48000 * 2 * 4 + 8192 },
  );
  assert.equal(ref.status, 0, ref.stderr?.toString());
  await writeFile(resolve(work, name + "-reference.f32"), ref.stdout);
  const count = Math.min(decoded.stdout.length, ref.stdout.length) / 4;
  let error = 0,
    energy = 0;
  for (let i = 0; i < count; i++) {
    const a = decoded.stdout.readFloatLE(i * 4),
      b = ref.stdout.readFloatLE(i * 4);
    error += (a - b) ** 2;
    energy += b * b;
  }
  const nrmse = Math.sqrt(error / energy);
  assert.ok(nrmse < 0.1, name + " AAC NRMSE " + nrmse);
  analyses[name] = {
    nrmse,
    decodedSamples: decoded.stdout.length / 4,
    referenceSamples: ref.stdout.length / 4,
  };
}
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify(
    {
      pass: true,
      work,
      analyses,
      limits: [
        "Author only, no live audio/device continuity or partial backend failure evidence.",
        "Native subtitle freezing and full M3 remain incomplete.",
      ],
    },
    null,
    2,
  ),
);
console.log({ pass: true, work, analyses });
