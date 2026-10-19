/** Independent real-host consumer; run only after maintainer freezes/builds M1.
 * No mock backend, no Cargo/GUI. Produces fresh media, packages and raw logs.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
const work = await mkdtemp(resolve(tmpdir(), "vestra-source-review-"));
const mediaA = resolve(work, "short-a.mp4");
const mediaB = resolve(work, "long-b.mp4");
function run(exe, args, options = {}) {
  const result = spawnSync(exe, args, { encoding: "utf8", ...options });
  return result;
}
for (const [path, duration, filter] of [
  [mediaA, "0.9", "testsrc2=size=160x90:rate=20"],
  [mediaB, "8", "smptebars=size=160x90:rate=20"],
]) {
  const result = run("ffmpeg", [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    filter,
    "-t",
    duration,
    "-c:v",
    "libx264",
    "-preset",
    "ultrafast",
    path,
  ]);
  assert.equal(result.status, 0, result.stderr);
}

async function consumer(host) {
  const assert = (await import("node:assert/strict")).default;
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const work = process.env.VESTRA_SOURCE_REVIEW;
  const a = path.join(work, "short-a.mp4"),
    b = path.join(work, "long-b.mp4");
  const states = [],
    backend = [],
    checks = {};
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const source = host.source;
  let operation = 0;
  const submit = (intent) => source.submit(intent, ++operation);
  async function until(predicate, label, ms = 5000) {
    const deadline = Date.now() + ms;
    while (Date.now() < deadline) {
      const state = await source.state();
      if (predicate(state)) return state;
      await wait(20);
    }
    throw Error(`${label}: ${JSON.stringify(await source.state())}`);
  }
  async function code(promise, expected) {
    await assert.rejects(promise, (e) => e?.code === expected);
  }
  try {
    const caps = await host.capabilities();
    assert.ok(caps.experimentalCapabilities?.includes("source.intent/0.1"));
    await host.events.subscribe("source.state", (s) => states.push(s));
    await host.events.subscribe("playback.backend", (e) => backend.push(e));
    // First prove actual asynchronous bad-file failure, separately from races.
    const bad = await submit({
      kind: "open",
      source: path.join(work, "missing"),
      startPaused: false,
    });
    assert.equal(bad.status, "completed");
    const failed = await until(
      (s) =>
        s.stamp.generation === bad.ticket.stamp.generation &&
        s.phase === "error",
      "missing file error",
    );
    assert.ok(failed.terminalReason);
    checks.realFailure = failed;

    const raceRecords = [];
    for (const old of [a, path.join(work, "missing-race"), a]) {
      const older = submit({ kind: "open", source: old, startPaused: false });
      const newer = submit({ kind: "open", source: b, startPaused: false });
      const [oldOp, newOp] = await Promise.all([older, newer]);
      assert.ok(newOp.ticket.stamp.generation > oldOp.ticket.stamp.generation);
      const ready = await until(
        (s) =>
          s.stamp.generation === newOp.ticket.stamp.generation &&
          s.phase === "ready",
        "B ready",
      );
      assert.equal(ready.source, b);
      const eventIndex = states.length;
      for (let i = 0; i < 15; i++) {
        const s = await source.state();
        assert.equal(s.stamp.generation, newOp.ticket.stamp.generation);
        assert.equal(s.source, b);
        assert.equal(s.phase, "ready");
        await wait(20);
      }
      for (const s of states.slice(eventIndex)) {
        assert.equal(s.stamp.generation, newOp.ticket.stamp.generation);
        assert.equal(s.source, b);
        assert.equal(s.phase, "ready");
      }
      raceRecords.push({ oldOp, newOp, ready });
    }
    checks.fastOpen = raceRecords;

    const short = await submit({ kind: "open", source: a, startPaused: false });
    const shortId = operation;
    const ended = await until(
      (s) =>
        s.stamp.generation === short.ticket.stamp.generation &&
        s.phase === "ended",
      "natural EOF under keep-open",
    );
    assert.equal(ended.terminalReason, "natural");
    assert.equal(ended.source, a);
    assert.ok(ended.duration > 0);
    const replayIntent = { kind: "open", source: a, startPaused: false };
    const queried = await source.operation(shortId);
    assert.deepEqual(await source.submit(replayIntent, shortId), queried);
    await code(source.submit({ kind: "close" }, shortId), "conflict");
    const untouched = await source.state();
    assert.deepEqual(untouched.stamp, ended.stamp);
    assert.equal(untouched.cycle, ended.cycle);
    assert.equal(untouched.phase, "ended");
    checks.operationReplay = { queried, unchanged: untouched };

    const restarted = await submit({ kind: "restart", expected: ended.stamp });
    assert.equal(restarted.ticket.stamp.generation, ended.stamp.generation);
    assert.ok(restarted.ticket.stamp.intentVersion > ended.stamp.intentVersion);
    assert.ok(restarted.ticket.cycle > ended.cycle);
    await code(
      submit({
        kind: "open",
        source: b,
        startPaused: false,
        expected: ended.stamp,
      }),
      "stale",
    );
    const reready = await until(
      (s) => s.phase === "ready" && s.cycle === restarted.ticket.cycle,
      "restart ready",
    );
    assert.equal(reready.source, a);
    assert.deepEqual(reready.stamp, restarted.ticket.stamp);
    checks.restartAndStaleAutoNext = { ended, restarted, reready };

    // Reuse the same package ID and operation 1 in a genuinely new process.
    // Public ticket.owner must differ; the new owner must not query old results.
    await submit({ kind: "open", source: b, startPaused: false });
    await until(
      (s) => s.phase === "ready" && s.source === b,
      "long file before reload",
    );
    const helper = path.join(work, "owner");
    async function helperResult(index) {
      const file = path.join(work, `owner-${index}.json`);
      const deadline = Date.now() + 4000;
      while (Date.now() < deadline) {
        try {
          return JSON.parse(await fs.readFile(file, "utf8"));
        } catch (error) {
          if (error.code !== "ENOENT") throw error;
        }
        await wait(20);
      }
      throw Error(`owner ${index} did not report`);
    }
    await host.plugins.load(helper);
    const first = await helperResult(1);
    assert.equal(first.priorQueryError, "unknown-expired");
    await host.plugins.disable("review.source-owner");
    const afterDisable = await source.state();
    await host.plugins.load(helper);
    const second = await helperResult(2);
    assert.equal(second.priorQueryError, "unknown-expired");
    assert.notEqual(
      first.operation.ticket.owner,
      second.operation.ticket.owner,
    );
    assert.equal(
      first.operation.ticket.stamp.generation,
      second.operation.ticket.stamp.generation,
    );
    await wait(900); // old owner's scheduled close would have fired if still alive.
    const afterReload = await source.state();
    assert.equal(afterReload.source, b);
    assert.equal(afterReload.phase, "ready");
    assert.equal(afterReload.paused, true);
    assert.deepEqual(afterReload.stamp, second.operation.ticket.stamp);
    await host.plugins.disable("review.source-owner");
    checks.ownerReload = { first, second, afterDisable, afterReload };

    for (let i = 1; i < states.length; i++)
      assert.ok(
        states[i].eventSequence >= states[i - 1].eventSequence,
        "public sequence must not regress",
      );
    const result = {
      pass: true,
      checks,
      states,
      backend,
      limits: [
        "Fast local opens do not guarantee a delayed old FileLoaded/error was delivered after B. This run verifies no regression under real races, not arbitrary late-event injection.",
        "Owner reload verifies fresh operation namespace and old scheduled work cannot affect the source. It does not force an already queued stale reply/callback to arrive after reload.",
        "Headless source contract evidence only; no native draw, menu, device audio or M1 UI acceptance.",
      ],
    };
    await fs.writeFile(
      path.join(work, "result.json"),
      JSON.stringify(result, null, 2),
    );
    console.error("SOURCE_REVIEW_PASS");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "failure.json"),
      JSON.stringify(
        { error: String(error), stack: error.stack, checks, states, backend },
        null,
        2,
      ),
    );
    throw error;
  }
}

async function owner(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const work = process.env.VESTRA_SOURCE_REVIEW;
  const count = path.join(work, "owner-count.txt");
  let index = 1;
  try {
    index = Number(await fs.readFile(count, "utf8")) + 1;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await fs.writeFile(count, String(index));
  let priorQueryError;
  try {
    await host.source.operation(1);
  } catch (error) {
    priorQueryError = error.code;
  }
  const state = await host.source.state();
  const operation = await host.source.submit(
    {
      kind: "control",
      control: { kind: "pause", paused: true },
      expected: state.stamp,
    },
    1,
  );
  await fs.writeFile(
    path.join(work, `owner-${index}.json`),
    JSON.stringify({ index, priorQueryError, operation }),
  );
  // Disable must terminate this genuinely scheduled old-owner work.
  if (index === 1)
    setTimeout(() => {
      void host.source.submit({ kind: "close" }, 2).catch(console.error);
    }, 700);
}

for (const [name, activate, capabilities] of [
  [
    "consumer",
    consumer,
    ["playback.read", "playback.control", "events", "plugins.manage"],
  ],
  ["owner", owner, ["playback.read", "playback.control"]],
]) {
  const directory = resolve(work, name);
  await mkdir(directory);
  await writeFile(
    resolve(directory, "manifest.json"),
    JSON.stringify({
      id: `review.source-${name}`,
      apiVersion: "vestra.experimental/0.1",
      entry: "main.mjs",
      capabilities,
    }),
  );
  await writeFile(
    resolve(directory, "main.mjs"),
    `export const activate = ${activate.toString()};\n`,
  );
}
const child = run(
  process.env.VESTRA_SOURCE_REVIEW_HOST ??
    resolve(source, "host/target/debug/vestra-host"),
  [
    "--headless",
    "--silent",
    "--seconds",
    "14",
    "--plugin",
    resolve(work, "consumer"),
  ],
  { timeout: 22000, env: { ...process.env, VESTRA_SOURCE_REVIEW: work } },
);
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(
  resolve(work, "command.json"),
  JSON.stringify(
    {
      exitCode: child.status,
      signal: child.signal,
      error: String(child.error ?? ""),
      work,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ work, exitCode: child.status }));
assert.equal(child.status, 0, child.stderr);
assert.match(child.stderr, /SOURCE_REVIEW_PASS/);
const result = JSON.parse(await readFile(resolve(work, "result.json"), "utf8"));
assert.equal(result.pass, true);
console.log(
  JSON.stringify(
    {
      pass: result.pass,
      checks: Object.keys(result.checks),
      limits: result.limits,
    },
    null,
    2,
  ),
);
