/** Author native/plain SRT, shared sampler, and actual active encoder cancel. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-media-native-author-"));
await mkdir(resolve(work, "consumer"));
await mkdir(resolve(work, "burst"));
const ff = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg";
await writeFile(
  resolve(work, "native.srt"),
  "1\n00:00:00,500 --> 00:00:08,000\n原生字幕 NATIVE\n\n",
);
let p = spawnSync(
  ff,
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "testsrc2=s=640x360:r=30",
    "-i",
    resolve(work, "native.srt"),
    "-map",
    "0:v",
    "-map",
    "1:s",
    "-t",
    "9",
    "-c:v",
    "ffv1",
    "-c:s",
    "srt",
    "-disposition:s:0",
    "default",
    resolve(work, "a.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(p.status, 0, p.stderr);
p = spawnSync(
  ff,
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "color=c=blue:s=1920x1080:r=30",
    "-t",
    "20",
    "-c:v",
    "ffv1",
    resolve(work, "b.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(p.status, 0, p.stderr);
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    cp = await import("node:child_process"),
    assert = (await import("node:assert/strict")).default,
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const work = process.env.VESTRA_MEDIA_NATIVE;
  let op = 0;
  const jobs = [];
  let source;
  async function open(name) {
    const reply = await host.source.submit({
      kind: "open",
      source: path.join(work, name + ".mkv"),
      startPaused: true,
    });
    for (let i = 0; i < 200; i++) {
      source = await host.source.state();
      if (
        source.phase === "ready" &&
        source.stamp.generation === reply.ticket.stamp.generation
      ) {
        await wait(150);
        return source;
      }
      await wait(20);
    }
    throw Error("ready timeout");
  }
  async function terminal(id) {
    for (let i = 0; i < 400; i++) {
      const state = await host.jobs.state(id);
      if (["completed", "failed", "cancelled"].includes(state.status))
        return state;
      await wait(20);
    }
    throw Error("terminal timeout");
  }
  async function conditions() {
    const p = await host.processing.state(),
      s = await host.subtitles.state();
    return {
      expectedProcessingRevision: p.configurationRevision,
      expectedSubtitle: {
        selection: s.selection.stamp,
        presentationRevision: s.presentation.revision,
      },
    };
  }
  try {
    await open("a");
    let sub = await host.subtitles.state();
    assert.equal(sub.selection.target.kind, "embedded");
    await host.subtitles.style({
      operationId: 1,
      revision: 1,
      expected: sub.selection.stamp,
      style: {
        fontId: sub.presentation.fonts[0].id,
        fontSize: 72,
        color: "yellow",
        outline: 2,
        marginPercent: 5,
        offsetMs: 250,
      },
    });
    source = await host.source.state();
    const capture = {
      operationId: ++op,
      mode: "processed",
      expected: source.stamp,
      time: 1.5,
      ...(await conditions()),
    };
    const snapshot = await host.snapshot.capture(capture);
    assert.equal(snapshot.subtitle.kind, "native");
    assert.equal(snapshot.subtitle.coverageValidated, false);
    await host.playback.set("brightness", 10);
    await assert.rejects(
      host.snapshot.capture({ ...capture, operationId: ++op }),
      (e) => e.code === "unsupported",
    );
    await host.playback.set("brightness", 0);
    await host.playback.set("sub-delay", 0.123);
    await assert.rejects(
      host.snapshot.capture({ ...capture, operationId: ++op }),
      (e) => e.code === "unsupported",
    );
    await host.playback.set("sub-delay", 0.25);
    await open("b");
    sub = await host.subtitles.state();
    await host.subtitles.style({
      operationId: 2,
      revision: 2,
      expected: sub.selection.stamp,
      style: null,
    });
    for (const [index, time] of [1, 1.5, 2, 2.5, 3].entries()) {
      const started = await host.jobs.start({
        operationId: ++op,
        snapshotId: snapshot.snapshotId,
        kind: "processedPng",
        sampleTime: time,
        destination: path.join(work, "burst", index + ".png"),
      });
      const result = await terminal(started.jobId);
      assert.equal(result.status, "completed", JSON.stringify(result));
      assert.equal(result.facts.nativeSubtitle.coverageValidated, true);
      assert.equal(result.facts.range.start, time);
      jobs.push(result);
    }
    assert.equal(
      (await host.snapshot.state(snapshot.snapshotId)).subtitle
        .coverageValidated,
      false,
      "resolved job facts must not rewrite native descriptor",
    );
    await assert.rejects(
      host.jobs.start({
        operationId: ++op,
        snapshotId: snapshot.snapshotId,
        kind: "processedPng",
        sampleTime: snapshot.duration,
        destination: path.join(work, "invalid.png"),
      }),
      (e) => e.code === "invalid",
    );
    await host.snapshot.release(snapshot.snapshotId, ++op);
    source = await host.source.state();
    const cancelSnap = await host.snapshot.capture({
      operationId: ++op,
      mode: "processed",
      expected: source.stamp,
      time: 0,
      includeSubtitles: false,
      ...(await conditions()),
    });
    const active = await host.jobs.start({
      operationId: ++op,
      snapshotId: cancelSnap.snapshotId,
      kind: "processedMp4",
      end: 19,
      destination: path.join(work, "cancelled.mp4"),
    });
    let pid, ps;
    for (let i = 0; i < 300; i++) {
      let ids;
      try {
        ids = cp
          .execFileSync("/usr/bin/pgrep", ["-P", String(process.ppid)], {
            encoding: "utf8",
          })
          .trim()
          .split(/\s+/)
          .join(",");
      } catch (error) {
        if (error.status !== 1) throw error;
        await wait(10);
        continue;
      }
      ps = cp.execFileSync("/bin/ps", ["-p", ids, "-o", "pid=,ppid=,args="], {
        encoding: "utf8",
      });
      for (const line of ps.split("\n")) {
        const m = /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(line);
        if (
          m &&
          +m[2] === process.ppid &&
          m[3].includes("libx264") &&
          m[3].includes("-filter_complex")
        )
          pid = +m[1];
      }
      if (pid) break;
      await wait(10);
    }
    assert.ok(pid, "actual MP4 encoder child must exist before cancel");
    await fs.writeFile(
      path.join(work, "active-child.txt"),
      ps
        .split("\n")
        .filter((line) => line.trim().split(/\s+/)[0] === String(pid))
        .join("\n"),
    );
    const cancelled = await host.jobs.cancel(active.jobId, ++op);
    assert.equal(cancelled.cancel, "requested");
    const ended = await terminal(active.jobId);
    assert.equal(ended.status, "cancelled");
    assert.throws(
      () => process.kill(pid, 0),
      (e) => e.code === "ESRCH",
    );
    await assert.rejects(fs.readFile(path.join(work, "cancelled.mp4")), {
      code: "ENOENT",
    });
    assert.equal(
      (await fs.readdir(work)).some((name) => name.startsWith(".tmp")),
      false,
    );
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        {
          pass: true,
          snapshot,
          jobs,
          activePid: pid,
          cancelled,
          ended,
          after: await host.source.state(),
        },
        null,
        2,
      ),
    );
    console.error("MEDIA_NATIVE_SMOKE_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        { error: String(error), stack: error.stack, jobs },
        null,
        2,
      ),
    );
    throw error;
  }
}
await writeFile(
  resolve(work, "consumer/manifest.json"),
  JSON.stringify({
    id: "author.media-native",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "playback.read",
      "playback.control",
      "subtitles.read",
      "subtitles.control",
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
  "16",
  "--plugin",
  resolve(work, "consumer"),
];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 25000,
    env: { ...process.env, VESTRA_MEDIA_NATIVE: work },
  },
);
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
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
assert.match(child.stderr, /MEDIA_NATIVE_SMOKE_DONE/);
const analysis = [];
for (let i = 0; i < 5; i++) {
  const decoded = spawnSync(
    ff,
    [
      "-v",
      "error",
      "-i",
      resolve(work, "burst", i + ".png"),
      "-f",
      "rawvideo",
      "-pix_fmt",
      "rgb24",
      "-",
    ],
    { maxBuffer: 640 * 360 * 3 + 1024 },
  );
  assert.equal(decoded.status, 0, decoded.stderr?.toString());
  assert.equal(decoded.stdout.length, 640 * 360 * 3);
  let yellow = 0;
  for (let y = 300; y < 360; y++)
    for (let x = 0; x < 640; x++) {
      const k = (y * 640 + x) * 3;
      if (
        decoded.stdout[k] > 180 &&
        decoded.stdout[k + 1] > 180 &&
        decoded.stdout[k + 2] < 100
      )
        yellow++;
    }
  assert.ok(yellow > 100);
  analysis.push({ index: i, yellow });
}
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify(
    {
      pass: true,
      work,
      analysis,
      limits: [
        "Author smoke; native only plain subrip/srt, zero-origin lavf.",
        "No active validation-stage cancel, arbitrary native formatting or full M3 claim.",
      ],
    },
    null,
    2,
  ),
);
console.log({ pass: true, work, analysis });
