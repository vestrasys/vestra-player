/** Independent frozen-host style pixels. Two bounded sequential headless runs;
 * no GUI/build/mock backend, no font identity or frame-source fence claim.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-subtitle-style-review-"));
const text = "STYLE SAMPLE";
await writeFile(
  resolve(work, "embedded.srt"),
  `1\n00:00:00,000 --> 00:00:15,000\n${text}\n\n`,
);
for (const [name, color] of [
  ["a", "red"],
  ["b", "blue"],
]) {
  const args = [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    `color=c=gray:size=640x360:rate=30,drawbox=x=0:y=0:w=70:h=70:color=${color}:t=fill`,
  ];
  if (name === "b") args.push("-i", resolve(work, "embedded.srt"));
  args.push("-t", "18", "-c:v", "ffv1");
  if (name === "b") args.push("-c:s", "srt", "-disposition:s:0", "default");
  args.push(resolve(work, `${name}.mkv`));
  const made = spawnSync("ffmpeg", args, { encoding: "utf8" });
  assert.equal(made.status, 0, made.stderr);
}
async function activate(host) {
  const assert = (await import("node:assert/strict")).default,
    fs = await import("node:fs/promises"),
    path = await import("node:path"),
    { createHash } = await import("node:crypto");
  const work = process.env.VESTRA_SUBTITLE_STYLE_REVIEW,
    mode = process.env.VESTRA_SUBTITLE_STYLE_MODE,
    dir = path.join(work, mode),
    s = host.subtitles;
  const phases = [],
    events = [],
    checks = {};
  let op = 0,
    revision = 0;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  async function records() {
    try {
      return (
        await fs.readFile(path.join(dir, "frames", "frames.jsonl"), "utf8")
      )
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
      const state = await host.source.state();
      if (
        state.phase === "ready" &&
        state.stamp.generation === generation &&
        state.cycle >= cycle
      )
        return state;
      await wait(20);
    }
    throw Error("ready timeout");
  }
  async function stable(name) {
    await wait(350);
    const first = (await records()).length;
    await wait(500);
    const last = (await records()).length;
    assert.ok(last - first >= 2, "bounded samples " + name);
    const source = await host.source.state();
    assert.equal(source.paused, true);
    assert.equal(source.phase, "ready");
    phases.push({ name, first, last, source, subtitles: await s.state() });
  }
  async function apply(style) {
    const state = await s.state(),
      payload = {
        operationId: ++op,
        revision: ++revision,
        expected: state.selection.stamp,
        style,
      };
    const result = await s.style(payload);
    assert.deepEqual(await s.style(payload), result);
    assert.deepEqual(await s.operation(op), { status: "completed", result });
    assert.equal(result.presentation.actualFont, null);
    assert.equal(result.presentation.snapshotEligible, false);
    return result;
  }
  async function rejection(promise, code) {
    try {
      await promise;
      throw Error("expected " + code);
    } catch (e) {
      assert.equal(e.code, code, String(e));
      return { code: e.code, message: e.message };
    }
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, (data) =>
        events.push({ event, data }),
      );
    if (mode.startsWith("embedded-")) {
      if (mode === "embedded-test") {
        const a = await host.source.submit({
          kind: "open",
          source: path.join(work, "a.mkv"),
          startPaused: true,
        });
        await ready(a.ticket.stamp.generation);
        const f = (await s.state()).presentation.fonts[0];
        assert.ok(f);
        await apply({
          fontId: f.id,
          fontSize: 72,
          color: "yellow",
          outline: 2,
          marginPercent: 5,
          offsetMs: 0,
        });
      }
      const b = await host.source.submit({
        kind: "open",
        source: path.join(work, "b.mkv"),
        startPaused: true,
      });
      await ready(b.ticket.stamp.generation);
      assert.equal((await s.state()).selection.target?.kind, "embedded");
      if (mode === "embedded-test") {
        await stable("b-yellow");
        await apply(null);
        await stable("b-revoked-default");
        assert.equal((await s.state()).presentation.effective, null);
      } else await stable("b-control-default");
      const first = phases[0].source;
      for (const phase of phases) {
        assert.deepEqual(phase.source.stamp, first.stamp);
        assert.equal(phase.source.cycle, first.cycle);
        assert.ok(Math.abs(phase.source.position - first.position) < 0.001);
      }
      await fs.writeFile(
        path.join(dir, "consumer-result.json"),
        JSON.stringify({ pass: true, phases, events, checks }, null, 2),
      );
      console.error("SUBTITLE_STYLE_REVIEW_DONE " + mode);
      return;
    }
    const open = await host.source.submit({
      kind: "open",
      source: path.join(work, "a.mkv"),
      startPaused: true,
    });
    await ready(open.ticket.stamp.generation);
    await stable("gray");
    const initial = await s.state(),
      bytes = Buffer.from("1\n00:00:00,000 --> 00:00:15,000\nSTYLE SAMPLE\n\n");
    await s.begin(1, initial.source.generation, bytes.length);
    await s.append(1, 0, bytes.toString("base64"));
    await s.commit({
      operationId: ++op,
      uploadId: 1,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      resourceId: "sample",
      revision: 1,
      expected: initial.selection.stamp,
    });
    const font = (await s.state()).presentation.fonts[0];
    assert.ok(font);
    assert.equal(font.resolution, "requestedFamily");
    assert.equal(font.snapshotEligible, false);
    const yellow = {
      fontId: font.id,
      fontSize: 72,
      color: "yellow",
      outline: 2,
      marginPercent: 5,
      offsetMs: 0,
    };
    if (mode === "main") {
      await stable("default");
      await apply({ ...yellow, fontSize: 16, color: "white", outline: 0 });
      await stable("small-white");
      await apply(yellow);
      await stable("large-yellow");
      const before = await s.state();
      await rejection(
        s.style({
          operationId: ++op,
          revision: revision + 1,
          expected: before.selection.stamp,
          style: { ...yellow, fontSize: 73 },
        }),
        "invalid",
      );
      await rejection(
        s.style({
          operationId: ++op,
          revision: revision + 1,
          expected: before.selection.stamp,
          style: { ...yellow, fontId: "review-missing-font" },
        }),
        "unsupported",
      );
      const staleOp = ++op,
        stalePayload = {
          operationId: staleOp,
          revision: revision + 1,
          expected: {
            ...before.selection.stamp,
            revision: before.selection.stamp.revision + 100,
          },
          style: yellow,
        };
      const error = await rejection(s.style(stalePayload), "stale");
      assert.deepEqual(await s.operation(staleOp), { status: "failed", error });
      assert.deepEqual(await s.state(), before);
      checks.rejectedPreservePublic = before;
      await stable("failed-keeps-yellow");
      await apply({ ...yellow, offsetMs: 3000 });
      await stable("delayed-hidden");
      await apply(yellow);
      await stable("delay-restored");
      await host.plugins.load(path.join(work, "helper"));
      const deadline = Date.now() + 4000;
      let helper;
      while (Date.now() < deadline) {
        try {
          helper = JSON.parse(
            await fs.readFile(path.join(work, "helper-result.json"), "utf8"),
          );
          break;
        } catch (e) {
          if (e.code !== "ENOENT") throw e;
        }
        await wait(20);
      }
      assert.ok(helper);
      assert.equal(
        (await s.state()).presentation.effective.manifestId,
        "review.subtitle-style-z",
      );
      await stable("z-cyan");
      await apply({ ...yellow, fontSize: 16, color: "white", outline: 0 });
      assert.equal(
        (await s.state()).presentation.effective.manifestId,
        "review.subtitle-style-z",
      );
      await stable("a-late-still-cyan");
      await host.plugins.disable("review.subtitle-style-z");
      const after = await s.state();
      assert.equal(
        after.presentation.effective.manifestId,
        "review.subtitle-style-a",
      );
      await stable("z-disabled-small-white");
      await apply(null);
      assert.equal((await s.state()).presentation.effective, null);
      await stable("revoked-default");
      checks.helper = helper;
    } else {
      await apply(yellow);
      await stable("a-yellow");
      const before = await host.source.state(),
        marker = events.length;
      await host.source.submit({ kind: "restart", expected: before.stamp });
      const restarted = await ready(before.stamp.generation, before.cycle + 1);
      // Public restart deliberately starts playback; pause once for settled evidence.
      // This is not a subtitle mutation or a claim of paused continuity across restart.
      await host.source.submit({
        kind: "control",
        control: { kind: "pause", paused: true },
        expected: restarted.stamp,
      });
      // No subtitle commit/select/style after restart; test actual Ready replay alone.
      await stable("restart-yellow");
      checks.restartEvents = events.slice(marker);
      const b = await host.source.submit({
        kind: "open",
        source: path.join(work, "b.mkv"),
        startPaused: true,
      });
      await ready(b.ticket.stamp.generation);
      // B contains a default embedded SRT; no subtitle mutation after Ready.
      const bstate = await s.state();
      assert.equal(
        bstate.selection.target?.kind,
        "embedded",
        "B Ready must select actual embedded subtitle without new mutation",
      );
      assert.equal(bstate.presentation.effective.style.color, "yellow");
      await stable("b-ready-yellow");
      checks.bReady = bstate;
    }
    const first = phases[0].source;
    for (const phase of phases) {
      if (mode === "main") {
        assert.ok(Math.abs(phase.source.position - first.position) < 0.001);
        assert.deepEqual(phase.source.stamp, first.stamp);
        assert.equal(phase.source.cycle, first.cycle);
      }
    }
    await fs.writeFile(
      path.join(dir, "consumer-result.json"),
      JSON.stringify({ pass: true, phases, events, checks }, null, 2),
    );
    console.error("SUBTITLE_STYLE_REVIEW_DONE " + mode);
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
          checks,
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
    s = host.subtitles,
    state = await s.state();
  const result = await s.style({
    operationId: 1,
    revision: 1,
    expected: state.selection.stamp,
    style: {
      fontId: state.presentation.fonts[0].id,
      fontSize: 72,
      color: "cyan",
      outline: 2,
      marginPercent: 5,
      offsetMs: 0,
    },
  });
  await fs.writeFile(
    path.join(process.env.VESTRA_SUBTITLE_STYLE_REVIEW, "helper-result.json"),
    JSON.stringify(result, null, 2),
  );
}
for (const [name, fn, id] of [
  ["consumer", activate, "review.subtitle-style-a"],
  ["helper", helper, "review.subtitle-style-z"],
]) {
  const d = resolve(work, name);
  await mkdir(d);
  await writeFile(
    resolve(d, "manifest.json"),
    JSON.stringify({
      id,
      apiVersion: "vestra.experimental/0.1",
      entry: "main.mjs",
      capabilities: [
        "subtitles.read",
        "subtitles.control",
        "playback.read",
        ...(name === "consumer"
          ? ["playback.control", "events", "plugins.manage"]
          : []),
      ],
    }),
  );
  await writeFile(
    resolve(d, "main.mjs"),
    `export const activate=${fn.toString()};\n`,
  );
}
async function ppm(record, dir) {
  assert.equal(record.frameSourceAssociation, "none");
  const data = await readFile(resolve(dir, "frames", record.file));
  const h = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(data.toString("ascii", 0, 80));
  assert.ok(h);
  const rgb = data.subarray(h[0].length),
    width = +h[1],
    height = +h[2];
  assert.equal(rgb.length, width * height * 3);
  return { rgb, width, height, record };
}
function delta(a, b) {
  assert.equal(a.rgb.length, b.rgb.length);
  let changed = 0,
    exactDiffPixels = 0,
    exactDiffChannels = 0,
    maxAbsDiff = 0,
    yellow = 0,
    cyan = 0,
    sum = 0,
    x0 = a.width,
    y0 = a.height,
    x1 = -1,
    y1 = -1;
  // Ignore source identity marker in the top-left, inspect subtitle half only.
  for (let y = Math.floor(a.height * 0.4); y < a.height; y++)
    for (let x = 0; x < a.width; x++) {
      const i = (y * a.width + x) * 3;
      let max = 0;
      for (let c = 0; c < 3; c++) {
        const d = Math.abs(a.rgb[i + c] - b.rgb[i + c]);
        sum += d;
        max = Math.max(max, d);
        if (d !== 0) exactDiffChannels++;
        maxAbsDiff = Math.max(maxAbsDiff, d);
      }
      if (max > 0) exactDiffPixels++;
      if (max > 12) {
        changed++;
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
      if (b.rgb[i] > 180 && b.rgb[i + 1] > 180 && b.rgb[i + 2] < 100) yellow++;
      if (b.rgb[i] < 100 && b.rgb[i + 1] > 180 && b.rgb[i + 2] > 180) cyan++;
    }
  return {
    changedPixels: changed,
    exactDiffPixels,
    exactDiffChannels,
    maxAbsDiff,
    exactWithinSubtitleROI: exactDiffChannels === 0,
    yellowPixels: yellow,
    cyanPixels: cyan,
    mae: sum / a.rgb.length,
    bbox: changed ? { x0, y0, x1, y1 } : null,
  };
}
const combined = {};
const embeddedOnly = process.argv.includes("--embedded-revoke");
for (const mode of embeddedOnly
  ? ["embedded-control", "embedded-test"]
  : ["main", "replay"]) {
  const dir = resolve(work, mode);
  await mkdir(dir);
  await mkdir(resolve(dir, "frames"));
  const args = [
    "--headless",
    "--silent",
    "--seconds",
    mode === "main" ? "12" : "7",
    "--frame-evidence-dir",
    resolve(dir, "frames"),
    "--plugin",
    resolve(work, "consumer"),
  ];
  const child = spawnSync(
    resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
    args,
    {
      encoding: "utf8",
      timeout: 20000,
      env: {
        ...process.env,
        VESTRA_SUBTITLE_STYLE_REVIEW: work,
        VESTRA_SUBTITLE_STYLE_MODE: mode,
      },
    },
  );
  await writeFile(resolve(dir, "host.stderr.log"), child.stderr ?? "");
  await writeFile(resolve(dir, "host.stdout.log"), child.stdout ?? "");
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
  assert.match(child.stderr, /SUBTITLE_STYLE_REVIEW_DONE/);
  const c = JSON.parse(
      await readFile(resolve(dir, "consumer-result.json"), "utf8"),
    ),
    records = (await readFile(resolve(dir, "frames/frames.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map(JSON.parse),
    samples = {},
    analysis = {},
    comparisons = {},
    stability = {};
  try {
    for (const p of c.phases) {
      samples[p.name] = await Promise.all(
        records.slice(p.first, p.last).map((r) => ppm(r, dir)),
      );
      assert.ok(samples[p.name].length >= 2);
      stability[p.name] = samples[p.name]
        .slice(1)
        .map((frame) => delta(samples[p.name][0], frame));
      for (const d of stability[p.name])
        assert.equal(
          d.changedPixels,
          0,
          "stable subtitle RGB within tolerance",
        );
    }
    const gray = samples.gray?.[0] ?? samples[Object.keys(samples)[0]][0];
    for (const [name, frames] of Object.entries(samples))
      analysis[name] = frames.map((frame) => delta(gray, frame));
    const equal = (a, b) => {
      const diffs = samples[b].map((frame) => delta(samples[a][0], frame));
      comparisons[`${a} -> ${b}`] = diffs;
      for (const d of diffs)
        assert.equal(
          d.changedPixels,
          0,
          `${a} -> ${b} settled output within tolerance`,
        );
    };
    if (mode.startsWith("embedded-")) {
      for (const frames of Object.values(samples))
        for (const f of frames) {
          const i = (10 * f.width + 10) * 3;
          assert.ok(
            f.rgb[i + 2] > f.rgb[i] + 50,
            "actual B blue marker required",
          );
        }
      if (mode === "embedded-control")
        combined.controlFrame = samples["b-control-default"][0];
      else {
        const control = combined.controlFrame;
        assert.ok(control);
        comparisons["control-default -> b-yellow"] = samples["b-yellow"].map(
          (f) => delta(control, f),
        );
        for (const d of comparisons["control-default -> b-yellow"])
          assert.ok(d.yellowPixels > 100 && d.changedPixels > 200);
        comparisons["control-default -> b-revoked-default"] = samples[
          "b-revoked-default"
        ].map((f) => delta(control, f));
        for (const d of comparisons["control-default -> b-revoked-default"])
          assert.equal(
            d.changedPixels,
            0,
            "embedded revoke must restore independent actual default",
          );
      }
    } else if (mode === "main") {
      const small = analysis["small-white"][0],
        large = analysis["large-yellow"][0];
      assert.ok(small.changedPixels > 20);
      assert.ok(large.changedPixels > small.changedPixels * 3);
      assert.ok(large.bbox.y1 - large.bbox.y0 > small.bbox.y1 - small.bbox.y0);
      assert.equal(small.yellowPixels, 0);
      assert.ok(large.yellowPixels > 100);
      equal("large-yellow", "failed-keeps-yellow");
      for (const d of analysis["delayed-hidden"])
        assert.equal(d.changedPixels, 0);
      equal("large-yellow", "delay-restored");
      for (const d of analysis["z-cyan"])
        assert.ok(d.cyanPixels > 100 && d.yellowPixels === 0);
      equal("z-cyan", "a-late-still-cyan");
      equal("small-white", "z-disabled-small-white");
      equal("default", "revoked-default");
    } else {
      for (const name of ["a-yellow", "restart-yellow", "b-ready-yellow"])
        for (const d of analysis[name])
          assert.ok(d.yellowPixels > 100 && d.changedPixels > 200);
      equal("a-yellow", "restart-yellow");
      equal("a-yellow", "b-ready-yellow");
      for (const name of ["a-yellow", "restart-yellow", "b-ready-yellow"])
        for (const f of samples[name]) {
          const i = (10 * f.width + 10) * 3;
          if (name === "b-ready-yellow")
            assert.ok(f.rgb[i + 2] > f.rgb[i] + 50, "actual blue B marker");
          else assert.ok(f.rgb[i] > f.rgb[i + 2] + 50, "actual red A marker");
        }
      const a = c.phases.find((p) => p.name === "a-yellow").source,
        r = c.phases.find((p) => p.name === "restart-yellow").source,
        b = c.phases.find((p) => p.name === "b-ready-yellow").source;
      assert.equal(a.stamp.generation, r.stamp.generation);
      assert.ok(r.cycle > a.cycle);
      assert.ok(b.stamp.generation > a.stamp.generation);
    }
    combined[mode] = {
      pass: true,
      analysis,
      comparisons,
      stability,
      limits: [
        "Actual settled software RGB, no exact font identity or physical display fence.",
        "Pure validation/stale failures preserve pixels; no actual partial-backend rollback injected.",
        "Ready replay uses no new subtitle mutation after restart/B Ready; B default embedded fixture is observed independently via blue marker.",
        "frameSourceAssociation none; public source snapshots and repeated softwareSequence are not PTS/source fences.",
      ],
    };
    await writeFile(
      resolve(dir, "result.json"),
      JSON.stringify(combined[mode], null, 2),
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
          comparisons,
          stability,
          consumer: c,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}
delete combined.controlFrame;
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify({ pass: true, runs: combined }, null, 2),
);
console.log(JSON.stringify({ pass: true, work }));
