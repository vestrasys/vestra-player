/** Author fail-closed file/media time-domain regression, no supported nonzero claim. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtemp,
  mkdir,
  copyFile,
  writeFile,
  readFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-media-origin-author-"));
await mkdir(resolve(work, "consumer"));
const ff = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg";
await writeFile(resolve(work,"text.srt"),"1\n00:00:01,000 --> 00:00:04,000\nORIGIN 中文\n");
const generated=spawnSync(ff,["-v","error","-f","lavfi","-i","testsrc2=size=320x180:rate=30:duration=8","-i",resolve(work,"text.srt"),"-map","0:v","-map","1:s","-c:v","ffv1","-c:s","srt","-disposition:s:0","default",resolve(work,"zero.mkv")],{encoding:"utf8"});
assert.equal(generated.status,0,generated.stderr);
const made = spawnSync(
  ff,
  [
    "-v",
    "error",
    "-i",
    resolve(work, "zero.mkv"),
    "-map",
    "0",
    "-c",
    "copy",
    "-output_ts_offset",
    "2",
    resolve(work, "offset.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(made.status, 0, made.stderr);
for (const name of ["zero", "offset"]) {
  const probe = spawnSync(
    "/opt/homebrew/opt/ffmpeg-full/bin/ffprobe",
    [
      "-v",
      "error",
      "-show_entries",
      "format=start_time:stream=index,start_time,codec_name",
      "-of",
      "json",
      resolve(work, name + ".mkv"),
    ],
    { encoding: "utf8" },
  );
  assert.equal(probe.status, 0, probe.stderr);
  await writeFile(resolve(work, name + "-metadata.json"), probe.stdout);
  assert.equal(
    +JSON.parse(probe.stdout).format.start_time,
    name === "zero" ? 0 : 2,
  );
}
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    assert = (await import("node:assert/strict")).default,
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const work = process.env.VESTRA_MEDIA_ORIGIN;
  let op = 0;
  const results = [];
  try {
    for (const name of ["zero", "offset"]) {
      const open = await host.source.submit({
        kind: "open",
        source: path.join(work, name + ".mkv"),
        startPaused: true,
      });
      let source;
      for (let i = 0; i < 150; i++) {
        source = await host.source.state();
        if (
          source.phase === "ready" &&
          source.stamp.generation === open.ticket.stamp.generation
        )
          break;
        await wait(20);
      }
      assert.equal(source.phase, "ready");
      await wait(150);
      for (const mode of ["raw", "processed"]) {
        source = await host.source.state();
        const s = await host.subtitles.state(),
          p = await host.processing.state();
        const snapshot = await host.snapshot.capture({
          operationId: ++op,
          expected: source.stamp,
          mode,
          time: 1.5,
          ...(mode === "processed"
            ? {
                expectedProcessingRevision: p.configurationRevision,
                expectedSubtitle: {
                  selection: s.selection.stamp,
                  presentationRevision: s.presentation.revision,
                },
              }
            : {}),
        });
        assert.equal(
          snapshot.timeline.validation,
          "pending-worker-origin-match",
        );
        if (mode === "processed")
          assert.equal(
            snapshot.subtitle.native.timeline,
            "pending-file-origin-validation",
          );
        const destination = path.join(work, name + "-" + mode + ".png");
        const started = await host.jobs.start({
          operationId: ++op,
          snapshotId: snapshot.snapshotId,
          kind: mode === "raw" ? "rawPng" : "processedPng",
          destination,
        });
        await host.snapshot.release(snapshot.snapshotId, ++op);
        let terminal;
        for (let i = 0; i < 200; i++) {
          terminal = await host.jobs.state(started.jobId);
          if (["completed", "failed", "cancelled"].includes(terminal.status))
            break;
          await wait(20);
        }
        assert.equal(
          terminal.status,
          "completed",
          JSON.stringify(terminal),
        );
        assert.equal(
          terminal.facts.sourceTimeline.containerStartTime,
          name === "zero" ? 0 : 2,
        );
        assert.equal(terminal.facts.sourceTimeline.validation,"normalized-origin-matched");
        assert.ok((await fs.readFile(destination)).length > 8);
        if(name==="offset") assert.deepEqual(await fs.readFile(destination),await fs.readFile(path.join(work,"zero-"+mode+".png")),"normalized PNG source/字幕时轴应相同");
        results.push({ name, mode, source, snapshot, terminal });
      }
    }
    // Container duration remains metadata (10), while the normalized video ends at 8.
    const source=await host.source.state();
    const snapshot=await host.snapshot.capture({operationId:++op,expected:source.stamp,time:7.5});
    const destination=path.join(work,"past-actual-end.mp4");
    const started=await host.jobs.start({operationId:++op,snapshotId:snapshot.snapshotId,kind:"rawMp4",end:9,destination});
    let terminal;
    for(let i=0;i<250;i++){terminal=await host.jobs.state(started.jobId);if(["failed","completed","cancelled"].includes(terminal.status))break;await wait(20);}
    assert.equal(terminal.status,"failed",JSON.stringify(terminal));
    assert.match(terminal.outcome.error.message,/shorter than requested/);
    assert.equal(terminal.publicationStarted,false);
    await assert.rejects(fs.readFile(destination),{code:"ENOENT"});
    results.push({case:"actual-video-tail",snapshot,terminal});
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify({ pass: true, results }, null, 2),
    );
    console.error("MEDIA_ORIGIN_DONE");
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
    id: "author.media-origin",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "playback.read",
      "playback.control",
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
  "12",
  "--plugin",
  resolve(work, "consumer"),
];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 25000,
    env: { ...process.env, VESTRA_MEDIA_ORIGIN: work },
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
assert.match(child.stderr, /MEDIA_ORIGIN_DONE/);
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify(
    {
      pass: true,
      work,
      limits: [
        "Finite lavf probe+rebase normalized-media zero/origin2 evidence; backend metadata duration does not guarantee a frame at each time.",
      ],
    },
    null,
    2,
  ),
);
console.log({ pass: true, work });
