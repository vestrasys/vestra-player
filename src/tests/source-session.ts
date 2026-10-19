import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import type { Host } from "../sdk/index.ts";
import type { SourceState } from "../sdk/source.ts";
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
export async function activate(host: Host) {
  const states: SourceState[] = [];
  await host.events.subscribe("source.state", (data) =>
    states.push(data as SourceState),
  );
  async function until(phase: SourceState["phase"], generation?: number) {
    const deadline = Date.now() + 3000;
    while (Date.now() < deadline) {
      const s = await host.source.state();
      if (
        s.phase === phase &&
        (generation === undefined || s.stamp.generation === generation)
      )
        return s;
      await wait(15);
    }
    throw Error(
      `source did not reach ${phase}: ${JSON.stringify(await host.source.state())}`,
    );
  }
  const source = process.env.VESTRA_SOURCE_MEDIA!;
  const bad = await host.source.submit(
    { kind: "open", source: "/tmp/vestra-no-such-file", startPaused: false },
    1,
  );
  assert.equal(bad.status, "completed");
  const failed = await until("error");
  assert.ok(failed.terminalReason);
  // Native loadfile result carries entry ID before the event queue is drained.
  await Promise.all([
    host.source.submit(
      {
        kind: "open",
        source: "/tmp/vestra-another-missing-file",
        startPaused: false,
      },
      2,
    ),
    host.source.submit({ kind: "open", source, startPaused: false }, 3),
  ]);
  const ready = await until("ready", 3);
  assert.equal(ready.source, source);
  const query = await host.source.operation(3);
  assert.equal(query.status, "completed");
  const replay = await host.source.submit(
    { kind: "open", source, startPaused: false },
    3,
  );
  assert.deepEqual(replay, query);
  assert.equal((await host.source.state()).stamp.generation, 3);
  const ended = await until("ended", 3);
  assert.equal(ended.terminalReason, "natural");
  await host.source.submit(
    {
      kind: "control",
      control: { kind: "seek", seconds: 0.2 },
      expected: ended.stamp,
    },
    4,
  );
  const seekready = await until("ready", 3);
  assert.ok(seekready.cycle > ended.cycle);
  await host.source.submit(
    {
      kind: "control",
      control: { kind: "pause", paused: false },
      expected: seekready.stamp,
    },
    5,
  );
  const endedAgain = await until("ended", 3);
  assert.equal(endedAgain.terminalReason, "natural");
  assert.ok(endedAgain.cycle > ended.cycle);
  const restarted = await host.source.submit(
    { kind: "restart", expected: endedAgain.stamp },
    6,
  );
  assert.equal(restarted.status, "completed");
  await assert.rejects(
    host.source.submit(
      { kind: "open", source, startPaused: false, expected: ended.stamp },
      7,
    ),
    (e: unknown) => e instanceof Error && "code" in e && e.code === "stale",
  );
  const reready = await until("ready", 3);
  assert.ok(reready.cycle > ended.cycle);
  assert.ok(reready.stamp.intentVersion > ended.stamp.intentVersion);
  await host.source.submit(
    {
      kind: "control",
      control: { kind: "pause", paused: true },
      expected: reready.stamp,
    },
    8,
  );
  const paused = await host.source.state();
  assert.equal(paused.paused, true);
  const p = paused.position;
  await wait(200);
  assert.ok(
    Math.abs(((await host.source.state()).position ?? 0) - (p ?? 0)) < 0.05,
  );
  await host.source.submit({ kind: "close", expected: paused.stamp }, 9);
  const empty = await until("empty");
  assert.equal(empty.stamp.generation, 3);
  const record = {
    pass: true,
    failed,
    ready,
    ended,
    restarted: reready,
    endedAgain,
    paused,
    empty,
    events: states.length,
  };
  await writeFile(
    process.env.VESTRA_SOURCE_RESULT!,
    JSON.stringify(record, null, 2),
  );
  console.error("SOURCE_INTEGRATION_PASS");
}
