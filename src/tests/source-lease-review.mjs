/** Independent frozen-host source FD identity consumer; prepare before freeze.
 * Own generated files only. RGB markers identify content, not atomic PTS fences.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-source-lease-review-"));
await mkdir(resolve(work, "frames"));
for (const [name, marker] of [
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
      `testsrc2=size=320x180:rate=30,drawbox=x=0:y=0:w=64:h=64:color=${marker}:t=fill`,
      "-t",
      "16",
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
async function activate(host) {
  const assert = (await import("node:assert/strict")).default,
    fs = await import("node:fs/promises"),
    path = await import("node:path");
  const work = process.env.VESTRA_SOURCE_LEASE_REVIEW,
    userPath = path.join(work, "user-source.mp4"),
    oldPath = path.join(work, "old-unlinked-a.mp4");
  const wait = (ms) => new Promise((r) => setTimeout(r, ms)),
    phases = [],
    events = [],
    facts = {};
  async function records() {
    try {
      return (
        await fs.readFile(path.join(work, "frames", "frames.jsonl"), "utf8")
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
      const s = await host.source.state();
      if (
        s.phase === "ready" &&
        s.stamp.generation === generation &&
        s.cycle >= cycle
      )
        return s;
      await wait(20);
    }
    throw Error("actual source ready timeout");
  }
  async function stable(name) {
    await wait(600);
    const first = (await records()).length;
    await wait(650);
    const last = (await records()).length;
    assert.ok(last - first >= 2, "stable diagnostic samples " + name);
    const source = await host.source.state();
    assert.equal(source.phase, "ready");
    assert.equal(source.paused, true);
    assert.equal(
      source.source,
      userPath,
      "public source retains user path, not fd URL",
    );
    assert.equal(
      source.capabilities.snapshotSource,
      true,
      "ordinary current source exposes actual lease candidate",
    );
    assert.equal(
      source.container,
      "mov,mp4,m4a,3gp,3g2,mj2",
      "actual mpv MP4 container confirmation",
    );
    phases.push({ name, first, last, source });
    return source;
  }
  try {
    for (const event of ["source.state", "playback.backend", "host.error"])
      await host.events.subscribe(event, (data) =>
        events.push({ event, data }),
      );
    await fs.copyFile(path.join(work, "a.mp4"), userPath);
    const opened = await host.source.submit({
      kind: "open",
      source: userPath,
      startPaused: true,
    });
    await ready(opened.ticket.stamp.generation);
    const a = await stable("a-original");
    facts.opened = opened;
    const identityBefore = await fs.stat(userPath);
    await fs.rename(userPath, oldPath);
    await fs.copyFile(path.join(work, "b.mp4"), userPath);
    await fs.unlink(oldPath);
    await assert.rejects(fs.stat(oldPath), { code: "ENOENT" });
    const identityAfter = await fs.stat(userPath);
    assert.notEqual(
      identityAfter.ino,
      identityBefore.ino,
      "replacement must actually change inode",
    );
    facts.filesystem = {
      identityBefore: {
        dev: identityBefore.dev,
        ino: identityBefore.ino,
        size: identityBefore.size,
      },
      identityAfter: {
        dev: identityAfter.dev,
        ino: identityAfter.ino,
        size: identityAfter.size,
      },
      oldPathUnlinked: true,
    };
    await host.source.submit({
      kind: "control",
      control: { kind: "seek", seconds: 2 },
      expected: a.stamp,
    });
    const seeked = await stable("a-after-unlink-seek");
    assert.equal(seeked.stamp.generation, a.stamp.generation);
    assert.ok(seeked.stamp.intentVersion > a.stamp.intentVersion);
    assert.equal(seeked.cycle, a.cycle);
    assert.ok(
      Math.abs(seeked.position - 2) < 0.1,
      "seek must reach requested public position",
    );
    await host.source.submit({ kind: "restart", expected: seeked.stamp });
    const restarted = await ready(a.stamp.generation, a.cycle + 1);
    // Restart deliberately starts playback. Pause once for settled marker evidence.
    await host.source.submit({
      kind: "control",
      control: { kind: "pause", paused: true },
      expected: restarted.stamp,
    });
    const restart = await stable("a-after-unlink-restart");
    assert.equal(restart.stamp.generation, a.stamp.generation);
    assert.ok(restart.cycle > a.cycle);
    assert.ok(
      restart.position >= 0 && restart.position < 0.5,
      "restart must return near start",
    );
    const newOpen = await host.source.submit({
      kind: "open",
      source: userPath,
      startPaused: true,
    });
    await ready(newOpen.ticket.stamp.generation);
    const b = await stable("b-replacement-open");
    assert.ok(b.stamp.generation > a.stamp.generation);
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify({ pass: true, phases, events, facts, final: b }, null, 2),
    );
    console.error("SOURCE_LEASE_REVIEW_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        {
          pass: false,
          error: String(error),
          stack: error.stack,
          phases,
          events,
          facts,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}
const fixture = resolve(work, "consumer");
await mkdir(fixture);
await writeFile(
  resolve(fixture, "manifest.json"),
  JSON.stringify({
    id: "review.source-lease",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: ["playback.read", "playback.control", "events"],
  }),
);
await writeFile(
  resolve(fixture, "main.mjs"),
  `export const activate=${activate.toString()};\n`,
);
const args = [
  "--headless",
  "--silent",
  "--seconds",
  "8",
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
    timeout: 16000,
    env: { ...process.env, VESTRA_SOURCE_LEASE_REVIEW: work },
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
assert.match(child.stderr, /SOURCE_LEASE_REVIEW_DONE/);
const c = JSON.parse(
    await readFile(resolve(work, "consumer-result.json"), "utf8"),
  ),
  records = (await readFile(resolve(work, "frames/frames.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse),
  analysis = {};
async function marker(r) {
  assert.equal(r.frameSourceAssociation, "none");
  const data = await readFile(resolve(work, "frames", r.file)),
    h = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(data.toString("ascii", 0, 80));
  assert.ok(h);
  const width = +h[1],
    height = +h[2],
    rgb = data.subarray(h[0].length);
  assert.equal(rgb.length, width * height * 3);
  const sum = [0, 0, 0];
  let n = 0;
  for (let y = 10; y < 80; y++)
    for (let x = 10; x < 80; x++) {
      const i = (y * width + x) * 3;
      for (let k = 0; k < 3; k++) sum[k] += rgb[i + k];
      n++;
    }
  return { ...r, markerRGB: sum.map((v) => v / n) };
}
try {
  for (const phase of c.phases) {
    const samples = await Promise.all(
      records.slice(phase.first, phase.last).map(marker),
    );
    assert.ok(samples.length >= 2);
    for (const s of samples) {
      if (phase.name.startsWith("a-"))
        assert.ok(
          s.markerRGB[0] > s.markerRGB[2] + 100,
          "actual old A red content required",
        );
      else
        assert.ok(
          s.markerRGB[2] > s.markerRGB[0] + 100,
          "actual replacement B blue content required",
        );
    }
    analysis[phase.name] = samples;
  }
  const result = {
    pass: true,
    analysis,
    facts: c.facts,
    source: c.phases.map((p) => ({ name: p.name, source: p.source })),
    limits: [
      "Actual content marker + public source position after rename/replace/unlink; no frame PTS/source atomic fence.",
      "Paused seek tested at public position 2s; no exact per-frame PTS identity proved. Restart deliberately plays briefly before one pause.",
      "No offline job/lease reads, independent worker cursor, continuous playback under worker load, subtitle/effect snapshot or filesystem race fuzzing covered.",
      "Repeated softwareSequence can be cached settled content; no GUI/physical display/device audio.",
    ],
  };
  await writeFile(
    resolve(work, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify({ pass: true, work }));
} catch (error) {
  await writeFile(
    resolve(work, "pixel-failure.json"),
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
