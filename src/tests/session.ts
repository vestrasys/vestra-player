import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import type { Host, PlaybackState } from "../sdk/index.ts";
const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
export async function activate(host: Host) {
  const states: PlaybackState[] = [];
  const off = await host.events.subscribe("playback.state", (value) =>
    states.push(value as PlaybackState),
  );
  const media = process.env.VESTRA_TEST_MEDIA;
  assert.ok(media);
  const opened = await host.playback.open(media);
  await delay(800);
  const before = await host.playback.state();
  assert.equal(before.session, opened.session);
  assert.ok(before.videoFrames > 5, "real libmpv must render frames");
  // Continuous property commits must preserve the player session and clock.
  const controlsStarted = performance.now();
  for (let i = 0; i < 45; i++) {
    await host.playback.set("brightness", (i % 21) - 10);
    await delay(20);
  }
  const controlsMs = performance.now() - controlsStarted;
  const after = await host.playback.state();
  assert.equal(after.session, before.session);
  assert.ok(
    (after.position ?? 0) > (before.position ?? 0) + 0.5,
    "clock advances while controls commit",
  );
  assert.ok(
    after.videoFrames > before.videoFrames + 12,
    "rendered frames advance during commits",
  );
  await host.playback.set("pause", true);
  await delay(250);
  const paused = await host.playback.state();
  assert.equal(paused.paused, true);
  await delay(250);
  const still = await host.playback.state();
  assert.ok(Math.abs((still.position ?? 0) - (paused.position ?? 0)) < 0.05);
  await host.playback.seek(3);
  await host.playback.set("pause", false);
  await delay(300);
  assert.ok(((await host.playback.state()).position ?? 0) >= 3);
  await assert.rejects(
    host.playback.set("speed", 0),
    (error: unknown) =>
      error instanceof Error && "code" in error && error.code === "invalid",
  );
  const helper = process.env.VESTRA_TEST_HELPER;
  assert.ok(helper);
  await host.plugins.load(helper);
  await delay(150);
  assert.ok(
    (await host.plugins.list()).some((plugin) => plugin.id === "test.helper"),
  );
  await assert.rejects(host.plugins.load(helper));
  await host.plugins.disable("test.helper");
  assert.ok(
    !(await host.plugins.list()).some((plugin) => plugin.id === "test.helper"),
  );
  assert.equal((await host.playback.state()).session, opened.session);
  const stalled = process.env.VESTRA_TEST_STALLED;
  assert.ok(stalled);
  const stallBefore = await host.playback.state();
  await host.plugins.load(stalled);
  await delay(1000);
  assert.ok(
    !(await host.plugins.list()).some((plugin) => plugin.id === "test.stalled"),
    "backpressured plugin must be disabled",
  );
  const stallAfter = await host.playback.state();
  assert.ok(
    stallAfter.videoFrames > stallBefore.videoFrames + 10,
    "stalled plugin must not block render/dispatcher",
  );
  await off();
  const result = {
    before,
    after,
    controlsMs,
    states: states.length,
    pass: true,
  };
  assert.ok(states.length >= 10);
  await writeFile(
    process.env.VESTRA_TEST_RESULT!,
    JSON.stringify(result, null, 2),
  );
  console.error("SESSION_INTEGRATION_PASS", JSON.stringify(result));
}
