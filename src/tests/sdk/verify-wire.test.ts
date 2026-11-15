// The wire trace differential (scripts/verify-wire.mjs, core thread plan
// S1.1): two traces of one behaviour compare equal whatever their owner
// numbers, request ids, times, paths and heartbeats; a line reordered by
// hand is caught, per plugin; the allowed-reorders mode (rule R4) lets the
// UI-bound and UI-independent streams of a plugin interleave differently
// but nothing else; the stream map matches the design's method table.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

// @ts-expect-error: a JavaScript build script without declarations.
import * as tool from "../../scripts/verify-wire.mjs";

const {
  RUNNER,
  STREAMS_FILE,
  checkTrace,
  compareTraces,
  compareWithNoise,
  noiseOf,
  noiseSize,
  diffPaths,
  normaliseTrace,
  readTrace,
  streamOf,
  streamsFromDesign,
} = tool;

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, "..", "..", "scripts", "verify-wire.mjs");
const DESIGN = join(
  HERE,
  "..",
  "..",
  "..",
  "doc",
  "architecture",
  "core-thread.md",
);
const STREAMS = JSON.parse(readFileSync(STREAMS_FILE, "utf8"));

type Line = Record<string, unknown>;
type Step = [string, "in" | "out", Line];

/** A trace as the host writes it, with this run's owners, ids and times. */
function trace(
  steps: Step[],
  run: { owner: number; idBase: number; t0: number; profile: string },
) {
  const seq: Record<string, number> = {};
  const records: string[] = [JSON.stringify({ start: 4242 })];
  steps.forEach(([plugin, dir, line], index) => {
    const shifted: Line = { ...line };
    if (typeof shifted.id === "number") shifted.id += run.idBase;
    const text = JSON.stringify(shifted).replaceAll(
      "<profile>",
      run.profile.replaceAll("\\", "\\\\"),
    );
    seq[plugin] = (seq[plugin] ?? 0) + 1;
    records.push(
      JSON.stringify({
        plugin,
        seq: seq[plugin],
        dir,
        owner: run.owner,
        ms: run.t0 + index * 7,
        line: JSON.parse(text),
      }),
    );
  });
  return records.join("\n") + "\n";
}

const A = { owner: 4, idBase: 0, t0: 100, profile: "C:\\out\\a\\profile" };
const B = { owner: 9, idBase: 50, t0: 380, profile: "D:\\work\\b\\profile" };

/** One plugin's talk: an independent read, a UI-bound mount, a broadcast. */
const STEPS: Step[] = [
  ["x.one", "in", { id: 1, method: "source.state", params: {} }],
  ["x.one", "in", { id: 2, method: "ui.mount", params: { slot: "sidebar" } }],
  [
    "x.one",
    "in",
    { id: 3, method: "storage.session.get", params: { key: "k" } },
  ],
  ["x.one", "out", { id: 1, result: { phase: "idle", position: 1.25 } }],
  ["x.one", "out", { id: 3, result: null }],
  ["x.one", "out", { id: 2, result: { ok: true } }],
  ["x.one", "out", { event: "source.state", data: { position: 3.5, at: 17 } }],
  ["x.one", "out", { event: "source.state", data: { position: 3.6, at: 18 } }],
  [
    "x.one",
    "out",
    { event: "plugin.start", data: { dataDirectory: "<profile>\\data\\x" } },
  ],
  [RUNNER, "out", { event: "runner.ping", data: { seq: 1 } }],
  [
    RUNNER,
    "in",
    { event: "runner.pong", data: { seq: 1, stats: { heapUsed: 5 } } },
  ],
  ["y.two", "in", { event: "runtime.ready" }],
  ["y.two", "out", { id: 7, method: "source.resolveRequest", params: {} }],
  ["y.two", "in", { id: 7, result: { url: "x" } }],
];

const normalise = (text: string, run: typeof A) =>
  normaliseTrace(readTrace(text), {
    streams: STREAMS,
    roots: [[run.profile, "<profile>"]],
  });

test("the stream map is the design's method table (126 rows: 111 I, 15 B)", () => {
  const streams = streamsFromDesign(readFileSync(DESIGN, "utf8"));
  assert.deepEqual(STREAMS, streams, "regenerate: verify-wire.mjs streams");
  assert.equal(streams.rows, 126);
  assert.equal(streams.uiBound.length, 15);
  assert.equal(streams.uiIndependent.length, 111);
  for (const name of [
    "ui.open",
    "ui.mount",
    "settings.set",
    "settings.show",
    "media.sources.locate",
    "commands.run",
    "window.setVideoArea",
  ])
    assert.equal(streamOf(STREAMS, name).stream, "B", name);
  for (const name of ["settings.register", "media.sources.get", "source.state"])
    assert.deepEqual(streamOf(STREAMS, name), { stream: "I", known: true });
  // A prefix route covers its names.
  assert.deepEqual(streamOf(STREAMS, "snapshot.take"), {
    stream: "I",
    known: true,
  });
  assert.deepEqual(streamOf(STREAMS, "no.such"), { stream: "I", known: false });
});

test("two runs of one behaviour compare equal: owners, ids, times, paths, heartbeats", () => {
  const a = normaliseTrace(readTrace(trace(STEPS, A)), {
    streams: STREAMS,
    roots: [[A.profile, "<profile>"]],
  });
  const b = normaliseTrace(readTrace(trace(STEPS, B)), {
    streams: STREAMS,
    roots: [[B.profile, "<profile>"]],
  });
  assert.deepEqual(compareTraces(a, b), { differences: [], reorders: [] });
  // Heartbeats are dropped; the two broadcasts, equal once their times are
  // normalised, are one; the reply carries its request's number.
  assert.equal(a.plugins[RUNNER], undefined);
  const out = a.plugins["x.one"].out.map(
    (entry: { text: string }) => entry.text,
  );
  assert.equal(
    out.filter((text: string) => text.includes("source.state")).length,
    1,
  );
  assert.ok(out[0].includes('"id":"#1"') && out[2].includes('"id":"#2"'));
  assert.ok(out.some((text: string) => text.includes("<profile>")));
  const two = a.plugins["y.two"];
  assert.ok(two.out[0].text.includes('"id":"h#1"'));
  assert.ok(two.in[1].text.includes('"id":"h#1"'));
  // The UI-bound request and its reply are in stream B.
  assert.equal(a.plugins["x.one"].in[1].stream, "B");
  assert.equal(a.plugins["x.one"].out[2].stream, "B");
});

test("a line reordered by hand is caught, and named per plugin", () => {
  const swapped = [...STEPS];
  // y.two's two lines read from it change places.
  [swapped[11], swapped[13]] = [swapped[13], swapped[11]];
  const { differences } = compareTraces(
    normalise(trace(STEPS, A), A),
    normalise(trace(swapped, B), B),
  );
  assert.equal(differences.length, 1, differences.join("\n"));
  assert.match(differences[0], /^y\.two \(read from it\), line 1:/);
  // A changed value too.
  const changed = STEPS.map((step): Step =>
    step[2].id === 3 && step[1] === "out"
      ? [step[0], step[1], { id: 3, result: "other" }]
      : step,
  );
  const second = compareTraces(
    normalise(trace(STEPS, A), A),
    normalise(trace(changed, B), B),
  );
  assert.equal(second.differences.length, 1);
  assert.match(second.differences[0], /^x\.one \(written to it\), line 2:/);
  // A plugin only one side has.
  const fewer = STEPS.filter((step) => step[0] !== "y.two");
  const third = compareTraces(
    normalise(trace(STEPS, A), A),
    normalise(trace(fewer, B), B),
  );
  assert.deepEqual(third.differences, [
    "y.two: only in A (2 lines read, 1 written)",
  ]);
});

test("allowed reorders: the two streams may interleave differently, nothing else", () => {
  // The UI-bound reply (#2) overtaken by the later independent one (#3)'s
  // predecessor: B answers #2 last in A, first in B.
  const reordered = [...STEPS];
  const [mount] = reordered.splice(5, 1);
  reordered.splice(3, 0, mount);
  const a = normalise(trace(STEPS, A), A);
  const b = normalise(trace(reordered, B), B);
  assert.equal(
    compareTraces(a, b).differences.length,
    1,
    "strict: a difference",
  );
  const allowed = compareTraces(a, b, { allowReorders: true });
  assert.deepEqual(allowed.differences, []);
  assert.equal(allowed.reorders.length, 1);
  assert.match(allowed.reorders[0], /^x\.one \(written to it\): the UI-bound/);
  // Two independent replies swapped: a reorder inside one stream fails.
  const inside = [...STEPS];
  [inside[3], inside[4]] = [inside[4], inside[3]];
  const broken = compareTraces(a, normalise(trace(inside, B), B), {
    allowReorders: true,
  });
  assert.equal(broken.differences.length, 1);
  assert.match(broken.differences[0], /UI-independent stream, line 1:/);
});

test("the command line compares V2 run directories step by step", () => {
  const root = mkdtempSync(join(tmpdir(), "verify-wire-"));
  for (const [side, run, swap] of [
    ["a", A, false],
    ["b", B, false],
    ["c", B, true],
  ] as const) {
    const directory = join(root, side, "raw", "idle");
    mkdirSync(directory, { recursive: true });
    writeFileSync(
      join(root, side, "run.json"),
      JSON.stringify({ profiles: { idle: run.profile }, stage: null }),
    );
    const steps = [...STEPS];
    if (swap) [steps[0], steps[2]] = [steps[2], steps[0]];
    writeFileSync(join(directory, "first.wire.jsonl"), trace(steps, run));
  }
  const same = diffPaths(join(root, "a"), join(root, "b"));
  assert.equal(same.failed, false, same.text);
  assert.match(same.text, /raw\/idle\/first\.wire\.jsonl: no difference/);
  const run = (args: string[]) =>
    spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8" });
  assert.equal(run(["diff", join(root, "a"), join(root, "b")]).status, 0);
  const caught = run(["diff", join(root, "a"), join(root, "c")]);
  assert.equal(caught.status, 1, caught.stdout);
  assert.match(caught.stdout, /x\.one \(read from it\), line 1:/);
  assert.match(caught.stdout, /RESULT: FAIL, 1 difference/);
  assert.equal(run(["diff", join(root, "a")]).status, 2);
  assert.equal(
    run(["streams", "--check", STREAMS_FILE]).status,
    0,
    "the data file matches the design",
  );
});

test("the noise of two base runs hides their differences, not new ones", () => {
  // Two base runs: x.one's first source.state reply differs (a timing value
  // the normaliser keeps), and y.two's ready and reply change places.
  const base2 = STEPS.map((step): Step =>
    step[1] === "out" && step[2].id === 1
      ? [step[0], step[1], { id: 1, result: { phase: "loading" } }]
      : step,
  );
  [base2[11], base2[13]] = [base2[13], base2[11]];
  const a = normalise(trace(STEPS, A), A);
  const b = normalise(trace(base2, B), B);
  const noise = noiseOf(a, b);
  assert.deepEqual(Object.keys(noise).sort(), ["x.one", "y.two"]);
  assert.deepEqual(Object.keys(noise["x.one"].out.kinds), [
    "reply:source.state",
  ]);
  assert.equal(noiseSize(noise).plugins, 2);
  // A third run like either base: nothing outside the noise.
  const same = compareWithNoise(a, normalise(trace(base2, B), B), noise);
  assert.deepEqual(same.differences, []);
  assert.ok(same.suppressed >= 2);
  // A new value in the noisy class, and a change in a quiet class.
  const fresh = STEPS.map((step): Step =>
    step[1] === "out" && step[2].id === 1
      ? [step[0], step[1], { id: 1, result: { phase: "never" } }]
      : step[1] === "out" && step[2].id === 3
        ? [step[0], step[1], { id: 3, result: "changed" }]
        : step,
  );
  const { differences } = compareWithNoise(
    a,
    normalise(trace(fresh, B), B),
    noise,
  );
  assert.equal(differences.length, 2, differences.join("\n"));
  assert.match(
    differences[0],
    /reply:source.state: a line neither base run had/,
  );
  assert.match(
    differences[1],
    /reply:storage.session.get, its line 1 \(not noisy/,
  );
});

type Raw = { plugin: string; dir: "in" | "out"; line: Line; ms?: number };
const records = (lines: Raw[]) =>
  lines.map((entry, i) => ({ seq: i + 1, owner: 3, ms: i * 10, ...entry }));
const P = "x.one";

test("check: pairing, the UI-bound order, ui.preparing and R13 as the wire shows them", () => {
  const area = { width: 640, height: 360 };
  const good = records([
    {
      plugin: P,
      dir: "in",
      line: { id: 1, method: "ui.mount", params: { slot: "sidebar" } },
    },
    { plugin: P, dir: "in", line: { id: 2, method: "ui.set", params: {} } },
    { plugin: P, dir: "out", line: { id: 2, result: null } },
    {
      plugin: P,
      dir: "out",
      line: { event: "ui.preparing", data: { slot: "sidebar" } },
    },
    { plugin: P, dir: "out", line: { id: 1, result: {} } },
    {
      plugin: P,
      dir: "in",
      line: { id: 3, method: "layout.state", params: {} },
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 3, result: { sidebar: { panels: [P] } } },
    },
    {
      plugin: P,
      dir: "out",
      line: {
        event: "window.state",
        data: { videoArea: { width: 1, height: 1 } },
      },
    },
    {
      plugin: P,
      dir: "in",
      line: { id: 4, method: "window.setVideoArea", params: area },
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 4, result: { ...area, clamped: false } },
    },
    {
      plugin: P,
      dir: "out",
      line: { event: "window.state", data: { videoArea: area } },
    },
    {
      plugin: P,
      dir: "in",
      line: { id: 5, method: "window.state", params: {} },
    },
    { plugin: P, dir: "out", line: { id: 5, result: { videoArea: area } } },
  ]);
  assert.deepEqual(checkTrace(good, STREAMS), { violations: [], notes: [] });

  const bad = records([
    // Two ordered UI-bound requests answered the other way round.
    { plugin: P, dir: "in", line: { id: 1, method: "ui.set", params: {} } },
    { plugin: P, dir: "in", line: { id: 2, method: "ui.show", params: {} } },
    { plugin: P, dir: "out", line: { id: 2, result: null } },
    { plugin: P, dir: "out", line: { id: 1, result: null } },
    // A reply to nothing.
    { plugin: P, dir: "out", line: { id: 9, result: null } },
    // ui.preparing after the mount's reply.
    {
      plugin: P,
      dir: "in",
      line: { id: 3, method: "ui.mount", params: { slot: "bottom" } },
    },
    { plugin: P, dir: "out", line: { id: 3, result: {} } },
    {
      plugin: P,
      dir: "out",
      line: { event: "ui.preparing", data: { slot: "bottom" } },
    },
    // The broadcast of the change before the reply, and a stale read.
    {
      plugin: P,
      dir: "in",
      line: { id: 4, method: "window.setVideoArea", params: area },
    },
    {
      plugin: P,
      dir: "out",
      line: { event: "window.state", data: { videoArea: area } },
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 4, result: { ...area, clamped: false } },
    },
    {
      plugin: P,
      dir: "in",
      line: { id: 5, method: "window.state", params: {} },
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 5, result: { videoArea: { width: 1, height: 1 } } },
    },
    // Left unanswered: a note.
    {
      plugin: P,
      dir: "in",
      line: { id: 6, method: "source.state", params: {} },
    },
  ]);
  const { violations, notes } = checkTrace(bad, STREAMS);
  assert.equal(violations.length, 5, violations.join("\n"));
  assert.match(
    violations[0],
    /ui\.show .* answered before the earlier UI-bound request ui\.set \(R4\)/,
  );
  assert.match(violations[1], /not pending \(pairing\)/);
  assert.match(
    violations[2],
    /ui\.preparing for bottom after its ui\.mount reply/,
  );
  assert.match(
    violations[3],
    /went out before the window\.setVideoArea reply \(R13\)/,
  );
  assert.match(
    violations[4],
    /first window\.state read .* not \{"width":640,"height":360\}/,
  );
  assert.equal(notes.length, 1);
  assert.match(notes[0], /1 request\(s\) unanswered .*source\.state/);

  // On the 1 s timeout path a stale read is today's behaviour, not a fault.
  const late = records([
    {
      plugin: P,
      dir: "in",
      line: { id: 1, method: "window.setVideoArea", params: area },
      ms: 0,
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 1, result: { ...area, clamped: false } },
      ms: 1005,
    },
    {
      plugin: P,
      dir: "in",
      line: { id: 2, method: "window.state", params: {} },
      ms: 1010,
    },
    {
      plugin: P,
      dir: "out",
      line: { id: 2, result: { videoArea: { width: 1, height: 1 } } },
      ms: 1020,
    },
  ]);
  assert.deepEqual(checkTrace(late, STREAMS).violations, []);
});
