/** Independent automatic right-angle geometry review. Frozen binary only.
 * No geometry env override, GUI/build or render-target-ratio workaround.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

async function activate(host) {
  const assert = (await import("node:assert/strict")).default,
    fs = await import("node:fs/promises"),
    path = await import("node:path");
  const work = process.env.VESTRA_ROTATION_REVIEW,
    mode = process.env.VESTRA_ROTATION_MODE,
    dir = path.join(work, mode),
    phases = [],
    events = [],
    rawJobs = [];
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  let mediaOp = 0,
    processingRevision = 0;
  async function records() {
    try {
      return (await fs.readFile(path.join(dir, "frames/frames.jsonl"), "utf8"))
        .split("\n")
        .slice(0, -1)
        .filter(Boolean)
        .map(JSON.parse);
    } catch (e) {
      if (e.code === "ENOENT") return [];
      throw e;
    }
  }
  async function ready(generation, cycle = 0) {
    const end = Date.now() + 4500;
    while (Date.now() < end) {
      const s = await host.source.state();
      if (
        s.phase === "ready" &&
        s.stamp.generation === generation &&
        s.cycle >= cycle
      )
        return s;
      await wait(20);
    }
    throw Error("ready timeout");
  }
  async function stable(name, fixture) {
    await wait(350);
    const first = (await records()).length;
    await wait(500);
    const last = (await records()).length;
    assert.ok(last - first >= 2, "enough bounded settled samples " + name);
    const source = await host.source.state();
    assert.equal(source.phase, "ready");
    assert.equal(source.paused, true);
    assert.equal(source.source, path.join(work, fixture + ".mp4"));
    phases.push({
      name,
      fixture,
      first,
      last,
      source,
      processing: await host.processing.state(),
    });
    return source;
  }
  async function raw(fixture) {
    const source = await host.source.state();
    assert.equal(source.capabilities.snapshotSource, true);
    const snap = await host.snapshot.capture({
        operationId: ++mediaOp,
        expected: source.stamp,
        time: 1,
      }),
      destination = path.join(dir, "artifacts", fixture + ".png"),
      job = await host.jobs.start({
        operationId: ++mediaOp,
        snapshotId: snap.snapshotId,
        kind: "rawPng",
        destination,
      });
    await host.snapshot.release(snap.snapshotId, ++mediaOp);
    const observed = [];
    let state;
    const end = Date.now() + 4500;
    while (Date.now() < end) {
      state = await host.jobs.state(job.jobId);
      observed.push(state);
      if (["completed", "failed", "cancelled"].includes(state.status)) break;
      await wait(20);
    }
    assert.equal(state.status, "completed", JSON.stringify(state));
    rawJobs.push({ fixture, snap, job, observed, destination });
  }
  async function open(fixture, name = fixture) {
    const receipt = await host.source.submit({
      kind: "open",
      source: path.join(work, fixture + ".mp4"),
      startPaused: true,
    });
    await ready(receipt.ticket.stamp.generation);
    return stable(name, fixture);
  }
  async function eq(brightness) {
    const source = await host.source.state();
    await host.processing.replace({
      stage: "video",
      resourceId: "tone",
      revision: ++processingRevision,
      expectedGeneration: source.stamp.generation,
      nodes: [{ kind: "eq", brightness, contrast: 1, saturation: 1, gamma: 1 }],
    });
    return source;
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, (data) =>
        events.push({ event, data }),
      );
    if (mode === "main") {
      await open("r0");
      await raw("r0");
      const ninety = await open("r90");
      await raw("r90");
      await host.source.submit({ kind: "restart", expected: ninety.stamp });
      const restarted = await ready(ninety.stamp.generation, ninety.cycle + 1);
      await host.source.submit({
        kind: "control",
        control: { kind: "pause", paused: true },
        expected: restarted.stamp,
      });
      const r = await stable("r90-restart", "r90");
      assert.equal(r.stamp.generation, ninety.stamp.generation);
      assert.ok(r.cycle > ninety.cycle);
      const before = await eq(0.03);
      const e1 = await stable("r90-effect-1", "r90");
      assert.deepEqual(e1.stamp, before.stamp);
      assert.equal(e1.cycle, before.cycle);
      assert.ok(Math.abs(e1.position - before.position) < 0.001);
      await eq(0.05);
      const e2 = await stable("r90-effect-2", "r90");
      assert.deepEqual(e2.stamp, e1.stamp);
      assert.equal(e2.cycle, e1.cycle);
      await host.plugins.load(path.join(work, "helper"));
      const end = Date.now() + 3500;
      let helper;
      while (Date.now() < end) {
        try {
          helper = JSON.parse(
            await fs.readFile(path.join(dir, "helper-ready.json"), "utf8"),
          );
          break;
        } catch (e) {
          if (e.code !== "ENOENT") throw e;
        }
        await wait(15);
      }
      assert.ok(helper);
      const mounted = await stable("r90-owner-effect", "r90");
      await host.plugins.disable("review.rotation-helper");
      const removed = await stable("r90-owner-revoked", "r90");
      assert.deepEqual(removed.stamp, mounted.stamp);
      assert.ok(
        !(await host.processing.state()).video.some(
          (x) => x.resource.manifestId === "review.rotation-helper",
        ),
      );
      await open("r0", "plain-after-rotated");
      await open("rn90");
      await raw("rn90");
    } else {
      for (const fixture of ["r180", "sar90", "sar0"]) {
        await open(fixture);
        await raw(fixture);
      }
    }
    await fs.writeFile(
      path.join(dir, "consumer-result.json"),
      JSON.stringify({ pass: true, phases, events, rawJobs }, null, 2),
    );
    console.error("ROTATION_REVIEW_DONE " + mode);
  } catch (error) {
    await fs.writeFile(
      path.join(dir, "consumer-failure.json"),
      JSON.stringify(
        {
          pass: false,
          error: String(error),
          stack: error.stack,
          phases,
          events,
          rawJobs,
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
    s = await host.source.state();
  const result = await host.processing.replace({
    stage: "video",
    resourceId: "owner-tone",
    revision: 1,
    expectedGeneration: s.stamp.generation,
    nodes: [
      { kind: "eq", brightness: 0.02, contrast: 1, saturation: 1, gamma: 1 },
    ],
  });
  await fs.writeFile(
    path.join(
      process.env.VESTRA_ROTATION_REVIEW,
      process.env.VESTRA_ROTATION_MODE,
      "helper-ready.json",
    ),
    JSON.stringify(result),
  );
}
function command(binary, args, options = {}) {
  const r = spawnSync(binary, args, { encoding: "utf8", ...options });
  assert.equal(r.status, 0, r.stderr?.toString());
  return r;
}
function decode(bytes) {
  const h = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(bytes.toString("ascii", 0, 100));
  assert.ok(h);
  const width = +h[1],
    height = +h[2],
    rgb = bytes.subarray(h[0].length);
  assert.equal(rgb.length, width * height * 3);
  return { width, height, rgb };
}
async function ppm(file) {
  return decode(await readFile(file));
}
function mean(image, x, y, r = 3) {
  const sums = [0, 0, 0];
  let n = 0;
  for (let j = Math.max(0, y - r); j <= Math.min(image.height - 1, y + r); j++)
    for (
      let i = Math.max(0, x - r);
      i <= Math.min(image.width - 1, x + r);
      i++
    ) {
      const p = (j * image.width + i) * 3;
      for (let c = 0; c < 3; c++) sums[c] += image.rgb[p + c];
      n++;
    }
  return sums.map((v) => v / n);
}
function label(rgb) {
  const palette = {
    red: [255, 0, 0],
    green: [0, 128, 0],
    blue: [0, 0, 255],
    yellow: [255, 255, 0],
  };
  return Object.entries(palette).sort(
    (a, b) =>
      a[1].reduce((s, v, i) => s + (v - rgb[i]) ** 2, 0) -
      b[1].reduce((s, v, i) => s + (v - rgb[i]) ** 2, 0),
  )[0][0];
}
function geometry(image) {
  let x0 = image.width,
    y0 = image.height,
    x1 = -1,
    y1 = -1;
  for (let y = 0; y < image.height; y++)
    for (let x = 0; x < image.width; x++) {
      const i = (y * image.width + x) * 3;
      if (Math.max(...image.rgb.subarray(i, i + 3)) > 25) {
        x0 = Math.min(x0, x);
        x1 = Math.max(x1, x);
        y0 = Math.min(y0, y);
        y1 = Math.max(y1, y);
      }
    }
  assert.ok(x1 > x0 && y1 > y0);
  const points = [
    [0.15, 0.15],
    [0.85, 0.15],
    [0.15, 0.85],
    [0.85, 0.85],
  ].map(([x, y]) => [
    Math.round(x0 + (x1 - x0) * x),
    Math.round(y0 + (y1 - y0) * y),
  ]);
  return {
    bbox: { x0, y0, x1, y1 },
    corners: points.map(([x, y]) => ({
      x,
      y,
      rgb: mean(image, x, y),
      color: label(mean(image, x, y)),
    })),
  };
}
function compareGeometry(image, reference) {
  const actual = geometry(image),
    expected = geometry(reference);
  assert.equal(image.width, 960);
  assert.equal(image.height, 540);
  for (const k of ["x0", "y0", "x1", "y1"])
    assert.ok(
      Math.abs(actual.bbox[k] - expected.bbox[k]) <= 3,
      `active bbox ${k}: ${JSON.stringify({ actual, expected })}`,
    );
  assert.deepEqual(
    actual.corners.map((x) => x.color),
    expected.corners.map((x) => x.color),
    "actual four corner orientation must match independent autorotate reference",
  );
  let blackSamples = 0,
    maxBlack = 0;
  for (let y = 5; y < image.height; y += 20)
    for (let x = 5; x < image.width; x += 20) {
      if (
        x < expected.bbox.x0 - 4 ||
        x > expected.bbox.x1 + 4 ||
        y < expected.bbox.y0 - 4 ||
        y > expected.bbox.y1 + 4
      ) {
        maxBlack = Math.max(maxBlack, ...mean(image, x, y, 1));
        blackSamples++;
      }
    }
  assert.ok(maxBlack < 12, "actual letterbox remains black");
  return { actual, expected, blackSamples, maxBlack };
}
function rgbDifference(a, b) {
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

if (process.argv.includes("--run")) {
  assert.ok(
    !Object.keys(process.env).some((k) => /^VESTRA_.*(ROTAT|GEOMETRY)/.test(k)),
    "no inherited fixed geometry/rotation probe env allowed",
  );
  const work = await mkdtemp(resolve(tmpdir(), "vestra-rotation-review-"));
  await mkdir(resolve(work, "references"));
  const draw =
    "color=c=gray:size=320x180:rate=30,drawbox=x=0:y=0:w=160:h=90:color=red:t=fill,drawbox=x=160:y=0:w=160:h=90:color=green:t=fill,drawbox=x=0:y=90:w=160:h=90:color=blue:t=fill,drawbox=x=160:y=90:w=160:h=90:color=yellow:t=fill";
  for (const [name, sar] of [
    ["plain", 1],
    ["sar", 2],
  ])
    command("ffmpeg", [
      "-v",
      "error",
      "-f",
      "lavfi",
      "-i",
      draw + `,setsar=${sar}`,
      "-t",
      "16",
      "-c:v",
      "libx264",
      "-preset",
      "ultrafast",
      resolve(work, name + ".mp4"),
    ]);
  const specs = [
    ["r0", "plain", 0],
    ["r90", "plain", 90],
    ["rn90", "plain", -90],
    ["r180", "plain", 180],
    ["sar90", "sar", 90],
    ["sar0", "sar", 0],
  ];
  for (const [fixture, base, angle] of specs) {
    command("ffmpeg", [
      "-v",
      "error",
      "-display_rotation:v:0",
      String(angle),
      "-i",
      resolve(work, base + ".mp4"),
      "-c",
      "copy",
      resolve(work, fixture + ".mp4"),
    ]);
    const metadata = command("ffprobe", [
      "-v",
      "error",
      "-show_streams",
      "-of",
      "json",
      resolve(work, fixture + ".mp4"),
    ]).stdout;
    await writeFile(resolve(work, fixture + "-metadata.json"), metadata);
    const stream = JSON.parse(metadata).streams.find(
        (s) => s.codec_type === "video",
      ),
      rotation =
        stream.side_data_list?.find((s) => typeof s.rotation === "number")
          ?.rotation ?? 0;
    assert.equal(
      ((rotation % 360) + 360) % 360,
      ((angle % 360) + 360) % 360,
      "fixture has actual requested rotation",
    );
    assert.equal(stream.sample_aspect_ratio, base === "sar" ? "2:1" : "1:1");
    for (const kind of ["software", "raw"]) {
      const vf =
        "scale=w='max(1,round(iw*sar))':h=ih,setsar=1" +
        (kind === "software"
          ? ",scale=960:540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2:black"
          : "");
      command("ffmpeg", [
        "-v",
        "error",
        "-ss",
        "1",
        "-i",
        resolve(work, fixture + ".mp4"),
        "-frames:v",
        "1",
        "-vf",
        vf,
        "-pix_fmt",
        "rgb24",
        resolve(work, "references", fixture + "-" + kind + ".ppm"),
      ]);
    }
  }
  for (const [name, fn] of [
    ["consumer", activate],
    ["helper", helper],
  ]) {
    const d = resolve(work, name);
    await mkdir(d);
    await writeFile(
      resolve(d, "manifest.json"),
      JSON.stringify({
        id: `review.rotation-${name}`,
        apiVersion: "vestra.experimental/0.1",
        entry: "main.mjs",
        capabilities: [
          "playback.read",
          "processing.video",
          ...(name === "consumer"
            ? [
                "playback.control",
                "events",
                "plugins.manage",
                "media.read",
                "media.control",
              ]
            : []),
        ],
      }),
    );
    await writeFile(
      resolve(d, "main.mjs"),
      `export const activate=${fn.toString()};\n`,
    );
  }
  const result = {
    pass: true,
    modes: {},
    scope: "Automatic right-angle geometry + explicit SAR2 fixtures only",
  };
  for (const mode of ["main", "tail"]) {
    const dir = resolve(work, mode);
    await mkdir(dir);
    await mkdir(resolve(dir, "frames"));
    await mkdir(resolve(dir, "artifacts"));
    const args = [
        "--headless",
        "--silent",
        "--seconds",
        mode === "main" ? "12" : "7",
        "--frame-evidence-dir",
        resolve(dir, "frames"),
        "--plugin",
        resolve(work, "consumer"),
      ],
      child = spawnSync(
        resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
        args,
        {
          encoding: "utf8",
          timeout: 20000,
          env: {
            ...process.env,
            VESTRA_ROTATION_REVIEW: work,
            VESTRA_ROTATION_MODE: mode,
          },
        },
      );
    await writeFile(resolve(dir, "host.stdout.log"), child.stdout ?? "");
    await writeFile(resolve(dir, "host.stderr.log"), child.stderr ?? "");
    await writeFile(
      resolve(dir, "command.json"),
      JSON.stringify(
        { work, mode, args, exitCode: child.status, signal: child.signal },
        null,
        2,
      ),
    );
    console.log(JSON.stringify({ work, mode, exitCode: child.status }));
    assert.equal(child.status, 0, child.stderr);
    assert.match(child.stderr, /ROTATION_REVIEW_DONE/);
    const c = JSON.parse(
        await readFile(resolve(dir, "consumer-result.json"), "utf8"),
      ),
      records = (await readFile(resolve(dir, "frames/frames.jsonl"), "utf8"))
        .trim()
        .split("\n")
        .map(JSON.parse),
      analysis = { software: {}, raw: {} };
    try {
      for (const phase of c.phases) {
        const ref = await ppm(
            resolve(work, "references", phase.fixture + "-software.ppm"),
          ),
          samples = [];
        for (const record of records.slice(phase.first, phase.last)) {
          assert.equal(record.frameSourceAssociation, "none");
          samples.push({
            record,
            geometry: compareGeometry(
              await ppm(resolve(dir, "frames", record.file)),
              ref,
            ),
          });
        }
        assert.ok(samples.length >= 2);
        analysis.software[phase.name] = samples;
      }
      for (const job of c.rawJobs) {
        const png = await readFile(job.destination);
        assert.deepEqual(
          png.subarray(0, 8),
          Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
        );
        const decoded = command(
            "ffmpeg",
            [
              "-v",
              "error",
              "-i",
              job.destination,
              "-frames:v",
              "1",
              "-f",
              "image2pipe",
              "-vcodec",
              "ppm",
              "-",
            ],
            { encoding: null, maxBuffer: 8 * 1024 * 1024 },
          ),
          actual = decode(decoded.stdout),
          ref = await ppm(
            resolve(work, "references", job.fixture + "-raw.ppm"),
          ),
          difference = rgbDifference(actual, ref);
        assert.ok(
          difference.mae <= 1,
          "raw direction/SAR must match independent autorotate reference",
        );
        assert.deepEqual(
          geometry(actual).corners.map((x) => x.color),
          geometry(ref).corners.map((x) => x.color),
        );
        analysis.raw[job.fixture] = {
          width: actual.width,
          height: actual.height,
          difference,
          geometry: geometry(actual),
        };
      }
      result.modes[mode] = { pass: true, analysis };
      await writeFile(
        resolve(dir, "result.json"),
        JSON.stringify(result.modes[mode], null, 2),
      );
    } catch (error) {
      await writeFile(
        resolve(dir, "pixel-failure.json"),
        JSON.stringify(
          {
            pass: false,
            error: String(error),
            stack: error.stack,
            analysis,
            consumer: c,
          },
          null,
          2,
        ),
      );
      throw error;
    }
  }
  result.limits = [
    "Only 0/+90/-90/180 metadata, explicit SAR2 cases, four-color static fixture; no arbitrary-angle support claim.",
    "Software comparison checks actual orientation/active bounding box/black letterbox, permits small effect color changes; no render-target aspect hack or fixed-env direction.",
    "Raw actual PNG fully decoded and compared to independently generated autorotate/square-pixel reference; no processed export claim.",
    "frameSourceAssociation none; snapshots are not PTS/source fences. Restart briefly starts playback then pauses by public source control.",
  ];
  await writeFile(
    resolve(work, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify({ pass: true, work }));
}
