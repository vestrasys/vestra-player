/** Independent real-host M3 consumer scaffold; NOT RUNNABLE until actual wire freezes.
 * Own generated artifacts only. No mock media, no GUI/build, no old source copies.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

// Enable only after the actual public SDK/router and final binary are frozen.
const wireConfirmed = true; // Actual SDK/router shape read; execution still waits for explicit final binary freeze.
export const necessaryCases = [
  "A snapshot survives rename/replace/unlink and current B; actual published PNG remains red A",
  "release snapshot after job admission does not revoke admitted job lease",
  "existing destination is never clobbered; original bytes/hash preserved",
  "same token query/replay and different payload conflict; stale source expected rejected",
  "accepted cancel and owner disable reach actual terminal; no published artifact or leftover worker temp",
  "while worker actually running, source position and renderer frame count advance; not merely process alive",
];
export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
export async function decodePng(filename, includePixels = false) {
  const png = await readFile(filename);
  assert.deepEqual(
    png.subarray(0, 8),
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    "actual PNG signature",
  );
  const decoded = spawnSync(
    "ffmpeg",
    [
      "-v",
      "error",
      "-i",
      filename,
      "-frames:v",
      "1",
      "-f",
      "image2pipe",
      "-vcodec",
      "ppm",
      "-",
    ],
    { maxBuffer: 16 * 1024 * 1024 },
  );
  assert.equal(decoded.status, 0, decoded.stderr?.toString());
  const ppm = decoded.stdout;
  const h = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(ppm.toString("ascii", 0, 100));
  assert.ok(h, "PNG decodes to RGB PPM");
  const width = +h[1],
    height = +h[2],
    rgb = ppm.subarray(h[0].length);
  assert.equal(rgb.length, width * height * 3);
  const sums = [0, 0, 0];
  let n = 0;
  for (let y = Math.floor(height * 0.04); y < Math.floor(height * 0.14); y++)
    for (let x = Math.floor(width * 0.04); x < Math.floor(width * 0.14); x++) {
      const i = (y * width + x) * 3;
      for (let c = 0; c < 3; c++) sums[c] += rgb[i + c];
      n++;
    }
  return {
    filename,
    sha256: sha256(png),
    sizeBytes: png.length,
    width,
    height,
    markerRGB: sums.map((v) => v / n),
    ...(includePixels ? { rgb } : {}),
  };
}
export async function createFixtures(work, onlyA = false) {
  await mkdir(resolve(work, "artifacts"));
  await mkdir(resolve(work, "frames"));
  for (const [name, color] of onlyA
    ? [["a", "red"]]
    : [
        ["a", "red"],
        ["b", "blue"],
      ]) {
    const made = spawnSync(
      "ffmpeg",
      [
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        `testsrc2=size=1280x720:rate=30,drawbox=x=0:y=0:w=256:h=256:color=${color}:t=fill`,
        "-t",
        "24",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        resolve(work, `${name}.mp4`),
      ],
      { encoding: "utf8" },
    );
    assert.equal(made.status, 0, made.stderr);
  }
  const existing = Buffer.from("INDEPENDENT NO-CLOBBER SENTINEL\n");
  await writeFile(resolve(work, "artifacts", "existing.png"), existing);
  return { existingSha256: sha256(existing) };
}

async function consumer(host) {
  const assert = (await import("node:assert/strict")).default,
    fs = await import("node:fs/promises"),
    path = await import("node:path"),
    { createHash } = await import("node:crypto"),
    { execFileSync } = await import("node:child_process");
  const work = process.env.VESTRA_MEDIA_JOBS_REVIEW,
    s = host.snapshot,
    j = host.jobs,
    checks = {},
    events = [],
    timeline = [];
  const wait = (ms) => new Promise((r) => setTimeout(r, ms)),
    hash = (b) => createHash("sha256").update(b).digest("hex");
  let op = 0;
  function children() {
    return execFileSync("ps", ["-axo", "pid=,ppid=,command="], {
      encoding: "utf8",
    })
      .split("\n")
      .map((line) => /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line))
      .filter(
        (m) =>
          m &&
          +m[2] === process.ppid &&
          /ffmpeg/.test(m[3]) &&
          m[3].includes("/dev/fd/0"),
      )
      .map((m) => ({ pid: +m[1], ppid: +m[2], command: m[3] }));
  }
  async function ready(generation) {
    const end = Date.now() + 5000;
    while (Date.now() < end) {
      const x = await host.source.state();
      if (x.phase === "ready" && x.stamp.generation === generation) return x;
      await wait(20);
    }
    throw Error("source ready timeout");
  }
  async function reject(promise, code) {
    try {
      await promise;
      throw Error("expected " + code);
    } catch (e) {
      assert.equal(e.code, code, String(e));
      return { code: e.code, message: e.message };
    }
  }
  async function terminal(id) {
    const end = Date.now() + 7000;
    while (Date.now() < end) {
      const x = await j.state(id);
      if (["completed", "failed", "cancelled"].includes(x.status)) return x;
      await wait(15);
    }
    throw Error("job terminal timeout " + id);
  }
  async function cleanup(dir) {
    const end = Date.now() + 4000;
    let entries, ps;
    while (Date.now() < end) {
      entries = await fs.readdir(dir);
      ps = children();
      if (
        !entries.some((name) => !/^owner-[01]\.png$/.test(name)) &&
        !ps.length
      )
        return {
          entries,
          children: ps,
          noPublicationObserved: entries.length === 0,
        };
      await wait(20);
    }
    throw Error(
      "owner worker/temp did not clean " + JSON.stringify({ entries, ps }),
    );
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, (data) =>
        events.push({ event, data }),
      );
    const userPath = path.join(work, "user.mp4");
    await fs.copyFile(path.join(work, "a.mp4"), userPath);
    const opened = await host.source.submit({
        kind: "open",
        source: userPath,
        startPaused: true,
      }),
      a = await ready(opened.ticket.stamp.generation);
    assert.equal(a.capabilities.snapshotSource, true);
    assert.deepEqual(await j.operation(1), { status: "unknown" });
    const capturePayload = { operationId: ++op, expected: a.stamp, time: 12 },
      snapshot = await s.capture(capturePayload);
    assert.equal(snapshot.kind, "raw");
    assert.equal(snapshot.source, userPath);
    assert.equal(snapshot.time, 12);
    assert.deepEqual(await s.state(snapshot.snapshotId), snapshot);
    assert.deepEqual(await s.capture(capturePayload), snapshot);
    assert.deepEqual(await j.operation(op), {
      status: "completed",
      result: snapshot,
    });
    await reject(s.capture({ ...capturePayload, time: 13 }), "conflict");
    const before = await fs.stat(userPath);
    await fs.rename(userPath, path.join(work, "old-a.mp4"));
    await fs.copyFile(path.join(work, "b.mp4"), userPath);
    await fs.unlink(path.join(work, "old-a.mp4"));
    const after = await fs.stat(userPath);
    assert.notEqual(before.ino, after.ino);
    const bReceipt = await host.source.submit({
        kind: "open",
        source: userPath,
        startPaused: false,
      }),
      b = await ready(bReceipt.ticket.stamp.generation);
    assert.ok(b.stamp.generation > a.stamp.generation);
    const staleId = ++op,
      stalePayload = { operationId: staleId, expected: a.stamp, time: 12 },
      staleError = await reject(s.capture(stalePayload), "stale");
    assert.deepEqual(await j.operation(staleId), {
      status: "failed",
      error: staleError,
    });
    assert.deepEqual(
      await reject(s.capture(stalePayload), "stale"),
      staleError,
    );
    checks.snapshotIdentity = {
      snapshot,
      a,
      b,
      identityBefore: { dev: before.dev, ino: before.ino },
      identityAfter: { dev: after.dev, ino: after.ino },
    };
    const starts = [];
    for (const filename of ["a-output.png", "a-second.png"]) {
      const payload = {
          operationId: ++op,
          snapshotId: snapshot.snapshotId,
          kind: "rawPng",
          destination: path.join(work, "artifacts", filename),
        },
        started = await j.start(payload);
      starts.push(started);
      assert.deepEqual(await j.start(payload), started);
      assert.deepEqual(await j.operation(op), {
        status: "completed",
        result: started,
      });
      await reject(
        j.start({
          ...payload,
          destination: payload.destination + ".different",
        }),
        "conflict",
      );
    }
    await s.release(snapshot.snapshotId, ++op);
    await reject(s.state(snapshot.snapshotId), "unknown-expired");
    const deadline = Date.now() + 7000;
    while (Date.now() < deadline) {
      const jobs = await Promise.all(starts.map((x) => j.state(x.jobId))),
        source = await host.source.state(),
        render = await host.playback.state(),
        ps = children();
      timeline.push({ wallMs: Date.now(), jobs, source, render, children: ps });
      if (
        jobs.every((x) =>
          ["completed", "failed", "cancelled"].includes(x.status),
        )
      )
        break;
      await wait(20);
    }
    const done = await Promise.all(starts.map((x) => terminal(x.jobId)));
    for (const x of done)
      assert.equal(x.status, "completed", JSON.stringify(x));
    const active = timeline.filter(
      (x) =>
        x.children.length &&
        x.jobs.some((j) => j.status === "running" && !j.publicationStarted),
    );
    let interval = null;
    for (let first = 0; first < active.length && !interval; first++)
      for (let last = first + 1; last < active.length; last++)
        if (
          active[last].children.some((child) =>
            active[first].children.some((prior) => prior.pid === child.pid),
          ) &&
          active[last].source.position > active[first].source.position + 0.03 &&
          active[last].render.videoFrames > active[first].render.videoFrames
        ) {
          interval = { before: active[first], after: active[last] };
          break;
        }
    checks.releasedSnapshotJobs = {
      starts,
      done,
      observedWorkerProgress: interval,
      workerProgressCovered: interval !== null,
    };
    const current = await host.source.state(),
      bSnapshot = await s.capture({
        operationId: ++op,
        expected: current.stamp,
        time: 20,
      });
    const existing = path.join(work, "artifacts", "existing.png"),
      original = await fs.readFile(existing),
      collision = await j.start({
        operationId: ++op,
        snapshotId: bSnapshot.snapshotId,
        kind: "rawPng",
        destination: existing,
      }),
      collisionDone = await terminal(collision.jobId);
    assert.equal(collisionDone.status, "failed");
    assert.deepEqual(await fs.readFile(existing), original);
    checks.noClobber = { collisionDone, sha256: hash(original) };
    const cancelledPath = path.join(work, "cancelled", "cancelled.png"),
      cancelJob = await j.start({
        operationId: ++op,
        snapshotId: bSnapshot.snapshotId,
        kind: "rawPng",
        destination: cancelledPath,
      }),
      cancel = await j.cancel(cancelJob.jobId, ++op),
      cancelDone = await terminal(cancelJob.jobId);
    if (cancel.cancel === "requested") {
      assert.equal(cancelDone.status, "cancelled");
      await assert.rejects(fs.stat(cancelledPath), { code: "ENOENT" });
      assert.deepEqual(await fs.readdir(path.dirname(cancelledPath)), []);
    } else assert.ok(["tooLate", "terminal"].includes(cancel.cancel));
    checks.cancel = {
      cancel,
      cancelDone,
      acceptedCancellationCovered: cancel.cancel === "requested",
    };
    await host.plugins.load(path.join(work, "helper"));
    let helper;
    const helperDeadline = Date.now() + 5000;
    while (Date.now() < helperDeadline) {
      try {
        helper = JSON.parse(
          await fs.readFile(path.join(work, "helper-started.json"), "utf8"),
        );
        break;
      } catch (e) {
        if (e.code !== "ENOENT") throw e;
      }
      await wait(5);
    }
    assert.ok(helper);
    const beforeDisableChildren = children();
    await host.plugins.disable("review.media-jobs-helper");
    const ownerCleanup = await cleanup(path.join(work, "owner-cancel"));
    assert.ok(
      !(await j.list()).some((x) =>
        helper.jobs.some((h) => h.jobId === x.jobId),
      ),
    );
    checks.ownerDisable = {
      helper,
      beforeDisableChildren,
      cleanup: ownerCleanup,
      limits:
        "Final owner PNGs, if any, can have won Publishing before disable; noPublicationObserved records the actual case. Owner jobs become inaccessible on disable; no post-disable public terminal is claimed. Pending pre-publication jobs cancel; already publishing can complete.",
    };
    const sourceAfter = await host.source.state(),
      renderAfter = await host.playback.state();
    assert.equal(sourceAfter.phase, "ready");
    assert.equal(sourceAfter.stamp.generation, b.stamp.generation);
    assert.ok(sourceAfter.position > b.position);
    assert.ok(renderAfter.videoFrames > 0);
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        { pass: true, checks, timeline, events, sourceAfter, renderAfter },
        null,
        2,
      ),
    );
    console.error("MEDIA_JOBS_REVIEW_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        {
          pass: false,
          error: String(error),
          stack: error.stack,
          checks,
          timeline,
          events,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}
async function helper(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    work = process.env.VESTRA_MEDIA_JOBS_REVIEW,
    state = await host.source.state();
  const prior = await host.jobs.operation(1),
    snapshot = await host.snapshot.capture({
      operationId: 1,
      expected: state.stamp,
      time: 20,
    }),
    jobs = [];
  for (let i = 0; i < 2; i++)
    jobs.push(
      await host.jobs.start({
        operationId: i + 2,
        snapshotId: snapshot.snapshotId,
        kind: "rawPng",
        destination: path.join(work, "owner-cancel", `owner-${i}.png`),
      }),
    );
  const states = await Promise.all(jobs.map((j) => host.jobs.state(j.jobId)));
  await fs.writeFile(
    path.join(work, "helper-started.json"),
    JSON.stringify({ prior, snapshot, jobs, states }, null, 2),
  );
}
if (process.argv.includes("--run")) {
  assert.equal(wireConfirmed, true);
  const work = await mkdtemp(resolve(tmpdir(), "vestra-media-jobs-review-")),
    fixtures = await createFixtures(work);
  await mkdir(resolve(work, "cancelled"));
  await mkdir(resolve(work, "owner-cancel"));
  for (const [name, fn] of [
    ["consumer", consumer],
    ["helper", helper],
  ]) {
    const dir = resolve(work, name);
    await mkdir(dir);
    await writeFile(
      resolve(dir, "manifest.json"),
      JSON.stringify({
        id: `review.media-jobs-${name}`,
        apiVersion: "vestra.experimental/0.1",
        entry: "main.mjs",
        capabilities: [
          "playback.read",
          "media.read",
          "media.control",
          ...(name === "consumer"
            ? ["playback.control", "events", "plugins.manage"]
            : []),
        ],
      }),
    );
    await writeFile(
      resolve(dir, "main.mjs"),
      `export const activate=${fn.toString()};\n`,
    );
  }
  const args = [
    "--headless",
    "--silent",
    "--seconds",
    "18",
    "--frame-evidence-dir",
    resolve(work, "frames"),
    "--plugin",
    resolve(work, "consumer"),
  ];
  const child = spawnSync(
    resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
    args,
    {
      encoding: "utf8",
      timeout: 26000,
      env: { ...process.env, VESTRA_MEDIA_JOBS_REVIEW: work },
    },
  );
  await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
  await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
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
  assert.match(child.stderr, /MEDIA_JOBS_REVIEW_DONE/);
  const c = JSON.parse(
      await readFile(resolve(work, "consumer-result.json"), "utf8"),
    ),
    pngs = await Promise.all(
      ["a-output.png", "a-second.png"].map((name) =>
        decodePng(resolve(work, "artifacts", name)),
      ),
    );
  for (const png of pngs)
    assert.ok(
      png.markerRGB[0] > png.markerRGB[2] + 100,
      "actual old A red PNG required after current B/path replacement",
    );
  assert.equal(c.checks.noClobber.sha256, fixtures.existingSha256);
  assert.deepEqual(c.checks.ownerDisable.helper.prior, { status: "unknown" });
  // Independent references use the retained original A fixture, never replaced user.mp4.
  for (const time of [0, 12]) {
    const ref = spawnSync(
      "ffmpeg",
      [
        "-v",
        "error",
        "-ss",
        String(time),
        "-i",
        resolve(work, "a.mp4"),
        "-frames:v",
        "1",
        "-an",
        "-vf",
        "scale=w='max(1,round(iw*sar))':h=ih,setsar=1",
        "-pix_fmt",
        "rgb24",
        "-c:v",
        "png",
        resolve(work, `reference-a-${time}.png`),
      ],
      { encoding: "utf8" },
    );
    assert.equal(ref.status, 0, ref.stderr);
  }
  const reference12 = await decodePng(
      resolve(work, "reference-a-12.png"),
      true,
    ),
    reference0 = await decodePng(resolve(work, "reference-a-0.png"), true);
  function rgbDelta(a, b) {
    assert.equal(a.width, b.width);
    assert.equal(a.height, b.height);
    let sum = 0,
      maxAbsDiff = 0,
      exactDiffChannels = 0;
    for (let i = 0; i < a.rgb.length; i++) {
      const d = Math.abs(a.rgb[i] - b.rgb[i]);
      sum += d;
      maxAbsDiff = Math.max(maxAbsDiff, d);
      if (d) exactDiffChannels++;
    }
    return { mae: sum / a.rgb.length, maxAbsDiff, exactDiffChannels };
  }
  const fixedTime = {
    reference12vs0: rgbDelta(reference12, reference0),
    jobs: [],
  };
  assert.ok(
    fixedTime.reference12vs0.mae > 1,
    "fixture must distinguish requested 12s from 0s",
  );
  for (const png of pngs) {
    const decoded = await decodePng(png.filename, true),
      at12 = rgbDelta(decoded, reference12),
      at0 = rgbDelta(decoded, reference0);
    assert.ok(
      at12.mae <= 1,
      "raw snapshot must match requested fixed-time 12s reference",
    );
    assert.ok(
      at0.mae > at12.mae + 1,
      "arbitrary A frame at 0s cannot satisfy capture time",
    );
    fixedTime.jobs.push({ filename: png.filename, vs12: at12, vs0: at0 });
  }
  await writeFile(
    resolve(work, "fixed-time-analysis.json"),
    JSON.stringify(fixedTime, null, 2),
  );
  const frameRecords = (
      await readFile(resolve(work, "frames/frames.jsonl"), "utf8")
    )
      .trim()
      .split("\n")
      .map(JSON.parse),
    lastFrame = frameRecords.at(-1);
  assert.ok(lastFrame);
  assert.equal(lastFrame.frameSourceAssociation, "none");
  const frameData = await readFile(resolve(work, "frames", lastFrame.file)),
    header = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(
      frameData.toString("ascii", 0, 80),
    );
  assert.ok(header);
  const frameRgb = frameData.subarray(header[0].length),
    fi = (20 * +header[1] + 20) * 3;
  assert.ok(
    frameRgb[fi + 2] > frameRgb[fi] + 100,
    "actual current B blue playback frame required",
  );
  const result = {
    pass: true,
    pngs,
    fixedTime,
    currentPlaybackFrame: lastFrame,
    checks: c.checks,
    limits: [
      "Raw PNG subset only; no processed snapshot/effect/subtitle/font or full M3 completion.",
      "Worker progress coverage is explicit: null interval means not independently demonstrated, not pass by Node survival.",
      "Owner disabled jobs have no public retained terminal; actual no files/child process is finite cleanup evidence, not arbitrary scheduler proof.",
      "Held file is metadata-checked, not immutable against in-place modification or malicious writers; no PTS/source atomic frame fence.",
    ],
  };
  await writeFile(
    resolve(work, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(
    JSON.stringify({
      pass: true,
      work,
      workerProgressCovered:
        c.checks.releasedSnapshotJobs.workerProgressCovered,
      cancelCovered: c.checks.cancel.acceptedCancellationCovered,
    }),
  );
}

async function sameSourceConsumer(host) {
  const assert = (await import("node:assert/strict")).default,
    fs = await import("node:fs/promises"),
    path = await import("node:path"),
    { execFileSync } = await import("node:child_process");
  const work = process.env.VESTRA_MEDIA_JOBS_REVIEW,
    timeline = [],
    events = [];
  let snapshot,
    starts = [];
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  function children() {
    return execFileSync("ps", ["-axo", "pid=,ppid=,command="], {
      encoding: "utf8",
    })
      .split("\n")
      .map((line) => /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line))
      .filter(
        (m) =>
          m &&
          +m[2] === process.ppid &&
          /ffmpeg/.test(m[3]) &&
          m[3].includes("/dev/fd/0"),
      )
      .map((m) => ({ pid: +m[1], ppid: +m[2], command: m[3] }));
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, (data) =>
        events.push({ event, data }),
      );
    const receipt = await host.source.submit({
      kind: "open",
      source: path.join(work, "a.mp4"),
      startPaused: false,
    });
    const readyEnd = Date.now() + 5000;
    let source;
    while (Date.now() < readyEnd) {
      source = await host.source.state();
      if (
        source.phase === "ready" &&
        source.stamp.generation === receipt.ticket.stamp.generation
      )
        break;
      await wait(20);
    }
    assert.equal(source.phase, "ready");
    assert.equal(source.paused, false);
    assert.equal(source.capabilities.snapshotSource, true);
    snapshot = await host.snapshot.capture({
      operationId: 1,
      expected: source.stamp,
      time: 12,
    });
    for (let i = 0; i < 2; i++)
      starts.push(
        await host.jobs.start({
          operationId: i + 2,
          snapshotId: snapshot.snapshotId,
          kind: "rawPng",
          destination: path.join(work, "artifacts", `same-a-${i}.png`),
        }),
      );
    const deadline = Date.now() + 7000;
    while (Date.now() < deadline) {
      const jobs = await Promise.all(
          starts.map((x) => host.jobs.state(x.jobId)),
        ),
        state = await host.source.state(),
        render = await host.playback.state(),
        ps = children();
      assert.equal(state.source, path.join(work, "a.mp4"));
      assert.equal(state.stamp.generation, source.stamp.generation);
      assert.equal(state.paused, false);
      timeline.push({
        wallMs: Date.now(),
        jobs,
        source: state,
        render,
        children: ps,
      });
      if (
        jobs.every((j) =>
          ["completed", "failed", "cancelled"].includes(j.status),
        )
      )
        break;
      await wait(20);
    }
    const final = await Promise.all(
      starts.map((x) => host.jobs.state(x.jobId)),
    );
    for (const job of final)
      assert.equal(job.status, "completed", JSON.stringify(job));
    let interval = null;
    const active = timeline.filter(
      (row) =>
        row.children.length &&
        row.jobs.some((j) => j.status === "running" && !j.publicationStarted),
    );
    for (let i = 0; i < active.length && !interval; i++)
      for (let k = i + 1; k < active.length; k++) {
        const sameChildPids = active[k].children
          .filter((x) => active[i].children.some((y) => x.pid === y.pid))
          .map((x) => x.pid);
        if (
          sameChildPids.length &&
          active[k].source.position > active[i].source.position + 0.03 &&
          active[k].render.videoFrames > active[i].render.videoFrames
        ) {
          interval = {
            sameChildPids,
            elapsedMs: active[k].wallMs - active[i].wallMs,
            before: active[i],
            after: active[k],
          };
          break;
        }
      }
    const cleanEnd = Date.now() + 3000;
    let residualChildren;
    while (Date.now() < cleanEnd) {
      residualChildren = children();
      if (!residualChildren.length) break;
      await wait(20);
    }
    const entries = await fs.readdir(path.join(work, "artifacts"));
    assert.deepEqual(
      entries.filter((x) => x.startsWith(".tmp")),
      [],
    );
    assert.deepEqual(residualChildren, []);
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        {
          pass: interval !== null,
          snapshot,
          starts,
          final,
          interval,
          timeline,
          events,
          cleanup: { entries, children: residualChildren },
        },
        null,
        2,
      ),
    );
    console.error(
      interval
        ? "MEDIA_JOBS_SAME_SOURCE_DONE"
        : "MEDIA_JOBS_SAME_SOURCE_INSUFFICIENT_WINDOW",
    );
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        {
          pass: false,
          error: String(error),
          stack: error.stack,
          snapshot,
          starts,
          timeline,
          events,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}
if (process.argv.includes("--same-source")) {
  const work = await mkdtemp(
    resolve(tmpdir(), "vestra-media-jobs-same-source-"),
  );
  await createFixtures(work, true);
  const fixture = resolve(work, "consumer");
  await mkdir(fixture);
  await writeFile(
    resolve(fixture, "manifest.json"),
    JSON.stringify({
      id: "review.media-jobs-same-source",
      apiVersion: "vestra.experimental/0.1",
      entry: "main.mjs",
      capabilities: [
        "playback.read",
        "playback.control",
        "media.read",
        "media.control",
        "events",
      ],
    }),
  );
  await writeFile(
    resolve(fixture, "main.mjs"),
    `export const activate=${sameSourceConsumer.toString()};\n`,
  );
  const args = [
    "--headless",
    "--silent",
    "--seconds",
    "12",
    "--frame-evidence-dir",
    resolve(work, "frames"),
    "--plugin",
    fixture,
  ];
  const child = spawnSync(
    resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
    args,
    {
      encoding: "utf8",
      timeout: 20000,
      env: { ...process.env, VESTRA_MEDIA_JOBS_REVIEW: work },
    },
  );
  await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
  await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
  await writeFile(
    resolve(work, "command.json"),
    JSON.stringify(
      {
        work,
        args,
        mode: "same-source",
        exitCode: child.status,
        signal: child.signal,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ work, exitCode: child.status }));
  assert.equal(child.status, 0, child.stderr);
  const c = JSON.parse(
      await readFile(resolve(work, "consumer-result.json"), "utf8"),
    ),
    pngs = [];
  const ref = spawnSync(
    "ffmpeg",
    [
      "-v",
      "error",
      "-ss",
      "12",
      "-i",
      resolve(work, "a.mp4"),
      "-frames:v",
      "1",
      "-an",
      "-vf",
      "scale=w='max(1,round(iw*sar))':h=ih,setsar=1",
      "-pix_fmt",
      "rgb24",
      "-c:v",
      "png",
      resolve(work, "reference-a-12.png"),
    ],
    { encoding: "utf8" },
  );
  assert.equal(ref.status, 0, ref.stderr);
  const reference = await decodePng(resolve(work, "reference-a-12.png"), true);
  for (let i = 0; i < 2; i++) {
    const png = await decodePng(
      resolve(work, "artifacts", `same-a-${i}.png`),
      true,
    );
    assert.equal(png.width, reference.width);
    assert.equal(png.height, reference.height);
    let sum = 0,
      exactDiffChannels = 0,
      maxAbsDiff = 0;
    for (let k = 0; k < png.rgb.length; k++) {
      const d = Math.abs(png.rgb[k] - reference.rgb[k]);
      sum += d;
      if (d) exactDiffChannels++;
      maxAbsDiff = Math.max(maxAbsDiff, d);
    }
    const vs12 = { mae: sum / png.rgb.length, exactDiffChannels, maxAbsDiff };
    assert.ok(vs12.mae <= 1);
    assert.ok(png.markerRGB[0] > png.markerRGB[2] + 100);
    const { rgb, ...metadata } = png;
    pngs.push({ ...metadata, vs12 });
  }
  const result = {
    pass: c.pass,
    coverage: c.interval
      ? "sameSourceProgressObserved"
      : "insufficientSameChildWindow",
    snapshot: c.snapshot,
    pngs,
    interval: c.interval,
    cleanup: c.cleanup,
    limits: [
      "One finite same-object A playback/worker trial; actual same FFmpeg PID at interval endpoints and public source/render counters.",
      "No read-system-call trace or long stress/device-audio guarantee; no proof against in-place adversarial file writes.",
      "Raw time12 output independently decoded/reference checked; no processed export/font/subtitle snapshot or atomic PTS fence.",
    ],
  };
  await writeFile(
    resolve(work, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(
    JSON.stringify({ pass: result.pass, coverage: result.coverage, work }),
  );
  // Insufficient observation is kept as such; no fake delay/retry loop manufactures coverage.
  assert.equal(
    result.pass,
    true,
    "same-source interval not captured; retain raw as insufficient",
  );
}
