/** Author finite public-SDK style/offset check. Actual RGB; no font identity fence. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-subtitle-style-"));
await mkdir(resolve(work, "frames"));
await mkdir(resolve(work, "consumer"));
const media = spawnSync(
  "ffmpeg",
  [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "color=c=gray:size=640x360:rate=30",
    "-t",
    "18",
    "-c:v",
    "ffv1",
    resolve(work, "gray.mkv"),
  ],
  { encoding: "utf8" },
);
assert.equal(media.status, 0, media.stderr);
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    { createHash } = await import("node:crypto"),
    assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_STYLE_VERIFY,
    phases = [],
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  let op = 0,
    revision = 0;
  const records = async () => {
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
  };
  async function stable(name) {
    await wait(500);
    const first = (await records()).length;
    await wait(650);
    const last = (await records()).length;
    assert.ok(last - first >= 2);
    phases.push({
      name,
      first,
      last,
      source: await host.source.state(),
      subtitle: await host.subtitles.state(),
    });
  }
  try {
    const receipt = await host.source.submit({
      kind: "open",
      source: path.join(work, "gray.mkv"),
      startPaused: true,
    });
    for (let i = 0; i < 200; i++) {
      const state = await host.source.state();
      if (
        state.phase === "ready" &&
        state.stamp.generation === receipt.ticket.stamp.generation
      )
        break;
      await wait(25);
    }
    await stable("gray");
    const s = host.subtitles,
      state = await s.state(),
      bytes = Buffer.from("1\n00:00:00,000 --> 00:00:15,000\nSTYLE SAMPLE\n\n");
    await s.begin(1, state.source.generation, bytes.length);
    await s.append(1, 0, bytes.toString("base64"));
    await s.commit({
      operationId: ++op,
      uploadId: 1,
      resourceId: "sample",
      revision: 1,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      expected: state.selection.stamp,
    });
    await stable("default");
    const font = (await s.state()).presentation.fonts[0];
    assert.ok(font, "known local font required");
    const style = {
      fontId: font.id,
      fontSize: 16,
      color: "white",
      outline: 0,
      marginPercent: 5,
      offsetMs: 0,
    };
    async function apply(value) {
      const before = await s.state();
      const options = {
        operationId: ++op,
        revision: ++revision,
        expected: before.selection.stamp,
        style: value,
      };
      const result = await s.style(options);
      assert.deepEqual(await s.style(options), result);
      assert.deepEqual(await s.operation(op), { status: "completed", result });
      assert.equal(result.presentation.snapshotEligible, false);
      return result;
    }
    await apply(style);
    await stable("small-white");
    style.fontSize = 72;
    style.color = "yellow";
    style.outline = 2;
    await apply({ ...style });
    await stable("large-yellow");
    style.offsetMs = 3000;
    await apply({ ...style });
    await stable("delayed-hidden");
    style.offsetMs = 0;
    await apply({ ...style });
    await stable("delay-restored");
    await apply(null);
    await stable("revoked-default");
    const source = phases[0].source;
    for (const phase of phases) {
      assert.deepEqual(phase.source.stamp, source.stamp);
      assert.equal(phase.source.cycle, source.cycle);
      assert.equal(phase.source.paused, true);
      assert.ok(Math.abs(phase.source.position - source.position) < 0.001);
    }
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify({ pass: true, phases }, null, 2),
    );
    console.error("STYLE_VERIFY_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        { error: String(error), stack: error.stack, phases },
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
    id: "verify.subtitle-style",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "playback.read",
      "playback.control",
      "subtitles.read",
      "subtitles.control",
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
    timeout: 20000,
    env: {
      ...process.env,
      VESTRA_STYLE_VERIFY: work,
      VESTRA_STYLE_DIAGNOSTICS: "1",
    },
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
assert.match(child.stderr, /STYLE_VERIFY_DONE/);
const consumer = JSON.parse(
    await readFile(resolve(work, "consumer-result.json"), "utf8"),
  ),
  records = (await readFile(resolve(work, "frames/frames.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
async function ppm(record) {
  const data = await readFile(resolve(work, "frames", record.file));
  const header = /^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(
    data.toString("ascii", 0, 80),
  );
  assert.ok(header);
  return {
    width: +header[1],
    height: +header[2],
    rgb: data.subarray(header[0].length),
  };
}
function compare(a, b) {
  let changed = 0,
    yellow = 0,
    x0 = a.width,
    y0 = a.height,
    x1 = -1,
    y1 = -1,
    sum = 0;
  assert.equal(a.rgb.length, b.rgb.length);
  for (let y = 0; y < a.height; y++)
    for (let x = 0; x < a.width; x++) {
      const i = (y * a.width + x) * 3;
      let max = 0;
      for (let c = 0; c < 3; c++) {
        const d = Math.abs(a.rgb[i + c] - b.rgb[i + c]);
        sum += d;
        max = Math.max(max, d);
      }
      if (max > 12) {
        changed++;
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
      if (b.rgb[i] > 180 && b.rgb[i + 1] > 180 && b.rgb[i + 2] < 100) yellow++;
    }
  return {
    changedPixels: changed,
    yellowPixels: yellow,
    mae: sum / a.rgb.length,
    bbox: changed ? { x0, y0, x1, y1 } : null,
  };
}
const samples = {},
  analysis = {};
for (const phase of consumer.phases) {
  assert.ok(phase.last - phase.first >= 2);
  samples[phase.name] = await Promise.all(
    records.slice(phase.first, phase.last).map(ppm),
  );
}
const gray = samples.gray[0];
for (const [name, frames] of Object.entries(samples))
  analysis[name] = frames.map((frame) => compare(gray, frame));
try {
  const small = analysis["small-white"][0],
    large = analysis["large-yellow"][0];
  assert.ok(small.changedPixels > 20);
  assert.ok(large.changedPixels > small.changedPixels * 3);
  assert.ok(large.yellowPixels > 100);
  assert.equal(small.yellowPixels, 0);
  assert.ok(large.bbox.y1 - large.bbox.y0 > small.bbox.y1 - small.bbox.y0);
  for (const delta of analysis["delayed-hidden"])
    assert.equal(delta.changedPixels, 0);
  for (const frame of samples["delay-restored"])
    assert.equal(compare(samples["large-yellow"][0], frame).changedPixels, 0);
  for (const frame of samples["revoked-default"])
    assert.equal(compare(samples.default[0], frame).changedPixels, 0);
  await writeFile(
    resolve(work, "result.json"),
    JSON.stringify(
      {
        pass: true,
        analysis,
        limits: [
          "One static paused source; settled actual software RGB, not display timing.",
          "Requested installed family only: actual font identity and immutable font lease unverified.",
          "No arbitrary ASS/bitmap style, device audio, offline parity or injected backend rollback.",
        ],
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ pass: true, analysis }));
} catch (error) {
  await writeFile(
    resolve(work, "pixel-failure.json"),
    JSON.stringify(
      { error: String(error), stack: error.stack, analysis },
      null,
      2,
    ),
  );
  throw error;
}
