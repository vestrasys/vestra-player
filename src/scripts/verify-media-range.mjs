/** Author regression: absolute half-open video endpoints under a fine TB. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-media-range-author-"));
await mkdir(resolve(work, "consumer"));
const ff = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg";
const made = spawnSync(
  ff,
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "testsrc2=s=320x180:r=30",
    "-t",
    "6",
    "-c:v",
    "ffv1",
    resolve(work, "source.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(made.status, 0, made.stderr);
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    assert = (await import("node:assert/strict")).default,
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const work = process.env.VESTRA_MEDIA_RANGE;
  let op = 0;
  const results = [];
  try {
    const opened = await host.source.submit({
      kind: "open",
      source: path.join(work, "source.mkv"),
      startPaused: true,
    });
    let source;
    for (let i = 0; i < 150; i++) {
      source = await host.source.state();
      if (
        source.phase === "ready" &&
        source.stamp.generation === opened.ticket.stamp.generation
      )
        break;
      await wait(20);
    }
    await wait(150);
    for (const mode of ["raw", "processed"])
      for (const [name, end] of [
        ["empty", 4.32],
        ["two", 4.38],
      ]) {
        source = await host.source.state();
        const p = await host.processing.state();
        const snapshot = await host.snapshot.capture({
          operationId: ++op,
          mode,
          expected: source.stamp,
          time: 4.31,
          audio: { kind: "none" },
          ...(mode === "processed"
            ? {
                includeSubtitles: false,
                expectedProcessingRevision: p.configurationRevision,
              }
            : {}),
        });
        const job = await host.jobs.start({
          operationId: ++op,
          snapshotId: snapshot.snapshotId,
          kind: mode === "raw" ? "rawMp4" : "processedMp4",
          end,
          destination: path.join(work, mode + "-" + name + ".mp4"),
        });
        await host.snapshot.release(snapshot.snapshotId, ++op);
        let terminal;
        for (let i = 0; i < 200; i++) {
          terminal = await host.jobs.state(job.jobId);
          if (["failed", "completed", "cancelled"].includes(terminal.status))
            break;
          await wait(20);
        }
        assert.equal(
          terminal.status,
          name === "empty" ? "failed" : "completed",
          JSON.stringify(terminal),
        );
        if (name === "empty") assert.equal(terminal.publicationStarted, false);
        results.push({ mode, name, end, snapshot, terminal });
      }
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify({ pass: true, results }, null, 2),
    );
    console.error("MEDIA_RANGE_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        { error: String(error), stack: error.stack, results },
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
    id: "author.media-range",
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
  resolve(work, "consumer/main.mjs"),
  `export const activate=${activate.toString()};\n`,
);
const args = [
  "--headless",
  "--silent",
  "--seconds",
  "7",
  "--plugin",
  resolve(work, "consumer"),
];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 15000,
    env: { ...process.env, VESTRA_MEDIA_RANGE: work },
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
assert.match(child.stderr, /MEDIA_RANGE_DONE/);
const counts = {};
for (const mode of ["raw", "processed"]) {
  await assert.rejects(readFile(resolve(work, mode + "-empty.mp4")), {
    code: "ENOENT",
  });
  const probe = spawnSync(
    "/opt/homebrew/opt/ffmpeg-full/bin/ffprobe",
    [
      "-v",
      "error",
      "-count_frames",
      "-show_streams",
      "-of",
      "json",
      resolve(work, mode + "-two.mp4"),
    ],
    { encoding: "utf8" },
  );
  assert.equal(probe.status, 0, probe.stderr);
  await writeFile(resolve(work, mode + "-probe.json"), probe.stdout);
  const info = JSON.parse(probe.stdout).streams;
  assert.equal(info.length, 1);
  assert.equal(+info[0].nb_read_frames, 2);
  counts[mode] = +info[0].nb_read_frames;
}
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify(
    {
      pass: true,
      work,
      counts,
      start: 4.31,
      emptyEnd: 4.32,
      twoEnd: 4.38,
      sourceFrameRate: 30,
      limits: [
        "Author host endpoint regression; not arbitrary VFR/start-time coverage.",
      ],
    },
    null,
    2,
  ),
);
console.log({ pass: true, work, counts });
