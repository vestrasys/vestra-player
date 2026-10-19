/** Finite author producer smoke. Not NES, a provider pause fence, or independent AV acceptance. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const work = await mkdtemp(resolve(tmpdir(), "vestra-producer-author-"));
const consumer = resolve(work, "consumer");
await mkdir(consumer);
const commands = [];
const ffmpeg = process.env.VESTRA_FFMPEG ?? "ffmpeg";
async function make(name, args) {
  const child = spawnSync(
    ffmpeg,
    [
      "-nostdin",
      "-v",
      "error",
      "-threads",
      "1",
      "-filter_threads",
      "1",
      ...args,
    ],
    {
      encoding: "utf8",
      timeout: 20000,
      maxBuffer: 1024 * 1024,
    },
  );
  commands.push({
    name,
    binary: ffmpeg,
    args,
    exitCode: child.status,
    signal: child.signal,
    error: String(child.error ?? ""),
  });
  await writeFile(resolve(work, `${name}.stdout.log`), child.stdout ?? "");
  await writeFile(resolve(work, `${name}.stderr.log`), child.stderr ?? "");
  await writeFile(
    resolve(work, "fixture-commands.json"),
    JSON.stringify(commands, null, 2),
  );
  assert.equal(child.status, 0, child.stderr || String(child.error));
}
await make("pcm", [
  "-f",
  "lavfi",
  "-i",
  "sine=frequency=440:sample_rate=48000:duration=1",
  "-af",
  "volume=0.2",
  "-ac",
  "2",
  "-c:a",
  "pcm_f32le",
  "-f",
  "f32le",
  resolve(work, "audio.f32"),
]);
await make("rgba", [
  "-f",
  "lavfi",
  "-i",
  "testsrc2=size=256x240:rate=60",
  "-frames:v",
  "8",
  "-an",
  "-pix_fmt",
  "rgba",
  "-c:v",
  "rawvideo",
  "-threads",
  "1",
  "-f",
  "rawvideo",
  resolve(work, "video.rgba"),
]);
await make("file", [
  "-f",
  "lavfi",
  "-i",
  "testsrc2=size=320x180:rate=30",
  "-t",
  "8",
  "-an",
  "-c:v",
  "libx264",
  "-threads",
  "1",
  "-pix_fmt",
  "yuv420p",
  resolve(work, "source.mp4"),
]);
const hashes = {};
for (const name of ["audio.f32", "video.rgba", "source.mp4"]) {
  const bytes = await readFile(resolve(work, name));
  hashes[name] = {
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
assert.equal(hashes["audio.f32"].bytes, 48000 * 2 * 4);
assert.equal(hashes["video.rgba"].bytes, 8 * 256 * 240 * 4);
await writeFile(
  resolve(work, "fixture-hashes.json"),
  JSON.stringify(hashes, null, 2),
);

// Serialized as a standalone production-JS consumer. All calls use the public
// SDK passed by the real runtime; there is no hand-written socket/wire protocol.
async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_PRODUCER_SMOKE;
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const observations = [];
  const healthObservations = [];
  let omittedHealthObservations = 0;
  let latestHealth;
  const connections = [];
  let operation = 0;
  let sequence = 0;
  let videoNumber = 0;
  async function producerState(id) {
    const state = await host.producer.state(id);
    const health = {
      producerId: id,
      health: state.health,
      deviceErrors: state.deviceErrors,
      xrunEvents: state.xrunEvents,
      deviceChangeEvents: state.deviceChangeEvents,
      errorDetails: state.errorDetails,
      errorOverflow: state.errorOverflow,
      buffer: state.buffer,
      error: state.error,
    };
    if (JSON.stringify(health) !== JSON.stringify(latestHealth)) {
      latestHealth = health;
      if (healthObservations.length < 128) healthObservations.push(health);
      else omittedHealthObservations++;
    }
    assert.ok(
      ["ok", "recoverableXrun", "deviceChanged"].includes(state.health),
      `fatal audio health: ${JSON.stringify(state)}`,
    );
    assert.equal(
      state.error,
      null,
      `producer transport failed: ${JSON.stringify(state)}`,
    );
    return state;
  }
  const pcmBytes = await fs.readFile(path.join(work, "audio.f32"));
  const view = new DataView(
    pcmBytes.buffer,
    pcmBytes.byteOffset,
    pcmBytes.byteLength,
  );
  const pcm = new Float32Array(pcmBytes.length / 4);
  for (let i = 0; i < pcm.length; i++) pcm[i] = view.getFloat32(i * 4, true);
  const rgba = await fs.readFile(path.join(work, "video.rgba"));
  function samples(start, frames) {
    const result = new Float32Array(frames * 2);
    for (let i = 0; i < frames; i++) {
      const source = ((start + i) % 48000) * 2;
      result[i * 2] = pcm[source];
      result[i * 2 + 1] = pcm[source + 1];
    }
    return result;
  }
  async function video(connection, epoch, sampleTime) {
    const start = (videoNumber++ % 8) * 256 * 240 * 4;
    await connection.video(
      { epoch, sampleTime, sequence: sequence++ },
      new Uint8Array(rgba.buffer, rgba.byteOffset + start, 256 * 240 * 4),
    );
  }
  async function audio(connection, epoch, sampleTime, frames) {
    await connection.audio(
      { epoch, sampleTime, sequence: sequence++ },
      samples(sampleTime, frames),
    );
  }
  async function until(label, read, predicate, milliseconds = 2500) {
    const deadline = Date.now() + milliseconds;
    let last;
    do {
      last = await read();
      if (predicate(last)) {
        observations.push({ label, state: last });
        return last;
      }
      await wait(15);
    } while (Date.now() < deadline);
    throw Error(`${label} timed out: ${JSON.stringify(last)}`);
  }
  async function submit(intent) {
    const result = await host.source.submit(intent, ++operation);
    assert.notEqual(result.status, "failed", JSON.stringify(result));
    return result;
  }
  async function pauseControl(id, paused, boundary) {
    // Synthetic provider emission has stopped after awaited complete packets.
    // This self-controlled author sample has no external core/in-flight frame;
    // its promise is not independent cross-process or NES frame-bound proof.
    const prior = await host.source.state();
    const intent = {
      kind: "control",
      control: { kind: "pause", paused },
      expected: prior.stamp,
    };
    const requestId = ++operation;
    const pending = await host.source.submit(intent, requestId);
    assert.equal(
      pending.status,
      "pending",
      "pause must wait for the provider acknowledgement",
    );
    const noticed = await until(
      "provider control notice",
      () => producerState(id),
      (state) => state.pendingControl?.paused === paused,
    );
    const notice = noticed.pendingControl;
    assert.equal(notice.producerId, id);
    assert.deepEqual(notice.stamp, pending.ticket.stamp);
    assert.equal(notice.cycle, pending.ticket.cycle);
    const pendingSource = await host.source.state();
    const replay = await host.source.submit(intent, requestId);
    assert.deepEqual(
      replay,
      pending,
      "same request replay must retain the original pending ticket",
    );
    const replayState = await producerState(id);
    assert.equal(replayState.epoch, noticed.epoch);
    assert.deepEqual(replayState.pendingControl, notice);
    assert.deepEqual((await host.source.state()).stamp, pendingSource.stamp);
    await wait(150);
    const callbackOnly = await producerState(id);
    const stillPending = await host.source.operation(requestId);
    assert.deepEqual(
      stillPending,
      pending,
      "host callback epoch ack cannot complete the provider operation",
    );
    assert.equal(callbackOnly.epoch, noticed.epoch);
    const badEchoes = [];
    for (const invalid of [
      { ...notice, epoch: notice.epoch + 1 },
      { ...notice, baseSample: notice.baseSample + 1 },
    ]) {
      await assert.rejects(
        () => host.producer.ack(invalid),
        (error) => {
          badEchoes.push({ invalid, code: error.code, message: String(error) });
          return error.code === "stale";
        },
      );
      assert.deepEqual(
        await host.source.operation(requestId),
        pending,
        "bad echo changed the pending operation",
      );
      assert.deepEqual((await host.source.state()).stamp, pendingSource.stamp);
      const afterBadEcho = await producerState(id);
      assert.equal(afterBadEcho.epoch, noticed.epoch);
      assert.deepEqual(afterBadEcho.pendingControl, notice);
    }
    const acknowledged = await host.producer.ack(notice);
    assert.equal(acknowledged.status, "acknowledged");
    assert.deepEqual(acknowledged.control, notice);
    const completed = await until(
      "original pause operation completes",
      () => host.source.operation(requestId),
      (result) => {
        assert.notEqual(result.status, "failed", JSON.stringify(result));
        return result.status === "completed";
      },
    );
    assert.deepEqual(completed.ticket, pending.ticket);
    const state = await until(
      "provider and audio callback control acknowledged",
      () => producerState(id),
      (s) =>
        s.epoch === notice.epoch &&
        s.acknowledged &&
        s.paused === paused &&
        s.pendingControl === null,
    );
    observations.push({
      label: "synthetic provider pause/resume ACK lifecycle",
      requestId,
      paused,
      boundary,
      pending,
      notice,
      replay,
      pendingSource,
      callbackOnly,
      stillPending,
      badEchoes,
      acknowledged,
      completed,
      state,
    });
    return state;
  }
  async function rejectIntent(label, intent, code) {
    const id = ++operation;
    let error;
    try {
      const result = await host.source.submit(intent, id);
      assert.equal(
        result.status,
        "failed",
        `${label} unexpectedly accepted: ${JSON.stringify(result)}`,
      );
      error = { code: result.error, result };
    } catch (failure) {
      if (failure.name === "AssertionError") throw failure;
      error = { code: failure.code, message: String(failure) };
    }
    assert.equal(error.code, code, `${label}: ${JSON.stringify(error)}`);
    let query;
    try {
      query = await host.source.operation(id);
    } catch (failure) {
      query = { error: failure.code, message: String(failure) };
    }
    assert.notEqual(
      query.status,
      "pending",
      `${label} left a pending operation`,
    );
    assert.notEqual(
      query.error,
      "timeout",
      `${label} operation query timed out`,
    );
    observations.push({ label, error, operation: query });
  }
  async function absent(id) {
    try {
      await host.producer.state(id);
      assert.fail("released producer remains addressable");
    } catch (failure) {
      if (failure.name === "AssertionError") throw failure;
      assert.equal(failure.code, "unavailable", String(failure));
      return { code: failure.code, message: String(failure) };
    }
  }
  async function release(id) {
    await host.producer.release(id);
    return absent(id);
  }
  async function openFile() {
    const accepted = await submit({
      kind: "open",
      source: path.join(work, "source.mp4"),
      startPaused: false,
    });
    return until(
      "ordinary file ready",
      () => host.source.state(),
      (s) =>
        s.phase === "ready" &&
        s.stamp.generation === accepted.ticket.stamp.generation,
      4000,
    );
  }
  async function feed(
    id,
    connection,
    epoch,
    nextAudio,
    nextVideo,
    milliseconds,
  ) {
    const start = await producerState(id);
    const firstFrame = await host.playback.state();
    const deadline = Date.now() + milliseconds;
    let last = start;
    while (Date.now() < deadline) {
      last = await producerState(id);
      assert.equal(last.epoch, epoch);
      assert.equal(last.paused, false);
      // At most four packets per turn and 4096 frames ahead. Every packet is
      // awaited on the SDK's socket write completion; no unbounded Promise fanout.
      for (let n = 0; n < 4 && nextAudio - last.consumedFrames < 3072; n++) {
        assert.ok(
          nextAudio >= last.consumedFrames,
          "producer underrun consumed absent PCM",
        );
        const count = Math.min(1024, 4096 - (nextAudio - last.consumedFrames));
        assert.ok(count >= 512 && count <= 1024);
        await audio(connection, epoch, nextAudio, count);
        nextAudio += count;
      }
      for (let n = 0; n < 3 && nextVideo <= last.consumedFrames + 1600; n++) {
        await video(connection, epoch, nextVideo);
        nextVideo += 800;
      }
      await wait(10);
    }
    last = await producerState(id);
    const finalFrame = await until(
      "producer video frame counter advances",
      () => host.playback.state(),
      (s) => s.videoFrames > firstFrame.videoFrames,
    );
    assert.ok(
      last.consumedFrames > start.consumedFrames + 4800,
      "matching PCM media clock did not advance",
    );
    assert.ok(last.underrunFrames >= start.underrunFrames);
    observations.push({
      label: "feed",
      start,
      last,
      firstFrame,
      finalFrame,
      nextAudio,
      nextVideo,
    });
    return { nextAudio, nextVideo, state: last };
  }
  try {
    const initial = await openFile();
    const prepared = await host.producer.prepare({ width: 256, height: 240 });
    assert.equal(prepared.epoch, 1);
    assert.equal(prepared.sampleRate, 48000);
    assert.equal(prepared.channels, 2);
    assert.equal(prepared.videoCapacity, 8);
    await rejectIntent(
      "not ready preserves ordinary source",
      {
        kind: "openProvider",
        producerId: prepared.producerId,
        startPaused: false,
        expected: initial.stamp,
      },
      "unavailable",
    );
    assert.deepEqual((await host.source.state()).stamp, initial.stamp);
    const connection = await host.producer.connect(prepared);
    connections.push(connection);
    await until(
      "candidate callback epoch acknowledged",
      () => producerState(prepared.producerId),
      (s) => s.epoch === 1 && s.acknowledged && s.paused,
    );
    await video(connection, 1, 0);
    await audio(connection, 1, 0, 2048);
    const ready = await until(
      "actual prefilled AV candidate",
      () => producerState(prepared.producerId),
      (s) => s.acknowledged && s.hasVideo && s.receivedFrames >= 1024,
    );
    assert.equal(ready.active, false);
    const opened = await submit({
      kind: "openProvider",
      producerId: prepared.producerId,
      startPaused: false,
      expected: initial.stamp,
    });
    const live = await until(
      "producer public source ready",
      () => host.source.state(),
      (s) =>
        s.phase === "ready" &&
        s.stamp.generation === opened.ticket.stamp.generation,
    );
    for (const key of ["seek", "rate", "snapshotSource"])
      assert.equal(live.capabilities[key], false, key);
    await until(
      "producer unpaused callback ack",
      () => producerState(prepared.producerId),
      (s) => s.active && s.acknowledged && !s.paused,
    );
    let cursor = await feed(prepared.producerId, connection, 1, 2048, 800, 600);
    let before = await host.source.state();
    for (const control of [
      { kind: "seek", seconds: 1 },
      { kind: "rate", rate: 1.5 },
    ]) {
      await rejectIntent(
        `unsupported ${control.kind}`,
        { kind: "control", control, expected: before.stamp },
        "unsupported",
      );
      assert.deepEqual((await host.source.state()).stamp, before.stamp);
    }
    const paused = await pauseControl(prepared.producerId, true, {
      emittedFrames: videoNumber,
      nextAudio: cursor.nextAudio,
      nextVideo: cursor.nextVideo,
    });
    assert.ok(paused.epoch > 1);
    const pausedSource = await host.source.state();
    await audio(connection, 1, cursor.nextAudio, 512); // explicit stale epoch packet
    await wait(320);
    const held = await producerState(prepared.producerId);
    const heldSource = await host.source.state();
    assert.equal(held.epoch, paused.epoch);
    assert.equal(
      held.receivedFrames,
      paused.receivedFrames,
      "old-epoch PCM polluted new epoch",
    );
    assert.equal(held.consumedFrames, paused.consumedFrames);
    assert.equal(heldSource.paused, true);
    assert.equal(heldSource.position, pausedSource.position);
    observations.push({
      label: "pause epoch isolation after synthetic provider acknowledgement",
      paused,
      held,
      pausedSource,
      heldSource,
    });
    const base = held.consumedFrames;
    await video(connection, held.epoch, base);
    await audio(connection, held.epoch, base, 2048);
    await until(
      "new pause epoch prefilled",
      () => producerState(prepared.producerId),
      (s) => s.receivedFrames >= held.receivedFrames + 2048 && s.hasVideo,
    );
    const resumed = await pauseControl(prepared.producerId, false, {
      emittedFrames: videoNumber,
      base,
      prefilledAudioEnd: base + 2048,
    });
    assert.equal(resumed.epoch, held.epoch);
    cursor = await feed(
      prepared.producerId,
      connection,
      held.epoch,
      base + 2048,
      base + 800,
      600,
    );
    before = await host.source.state();
    await host.playback.set("muted", true);
    await host.playback.set("volume", 17);
    const output = await until(
      "public output controls confirmed",
      () => host.source.state(),
      (s) => s.output.muted && s.output.volume === 17,
    );
    assert.deepEqual(
      output.stamp,
      before.stamp,
      "device output changed source intent",
    );
    observations.push({
      label: "shared output state (not audible measurement)",
      source: output,
    });
    await host.playback.set("muted", false);
    await host.playback.set("volume", 100);
    before = await host.source.state();
    await submit({ kind: "close", expected: before.stamp });
    const empty = await until(
      "producer source closed",
      () => host.source.state(),
      (s) => s.phase === "empty",
    );
    // Query BEFORE closing the socket or calling release: source.close itself
    // must have removed the active producer, not a later cleanup action.
    const gone = await absent(prepared.producerId);
    connection.close();
    const unused = await host.producer.prepare({ width: 256, height: 240 });
    const releasedCandidate = await release(unused.producerId);
    observations.push({
      label: "explicit release of never-activated candidate",
      unused,
      releasedCandidate,
    });
    const again = await host.producer.prepare({ width: 256, height: 240 });
    const second = await host.producer.connect(again);
    connections.push(second);
    await until(
      "second candidate ack",
      () => producerState(again.producerId),
      (s) => s.acknowledged && s.paused,
    );
    await video(second, again.epoch, 0);
    await audio(second, again.epoch, 0, 2048);
    await until(
      "second candidate AV ready",
      () => producerState(again.producerId),
      (s) => s.acknowledged && s.hasVideo && s.receivedFrames >= 1024,
    );
    await rejectIntent(
      "old source CAS cannot activate ready candidate",
      {
        kind: "openProvider",
        producerId: again.producerId,
        startPaused: false,
        expected: initial.stamp,
      },
      "stale",
    );
    assert.deepEqual((await host.source.state()).stamp, empty.stamp);
    const activatedAgain = await submit({
      kind: "openProvider",
      producerId: again.producerId,
      startPaused: false,
      expected: empty.stamp,
    });
    const secondLive = await until(
      "second producer actually active",
      () => host.source.state(),
      (s) =>
        s.phase === "ready" &&
        s.stamp.generation === activatedAgain.ticket.stamp.generation,
    );
    await until(
      "second producer running callback ack",
      () => producerState(again.producerId),
      (s) => s.active && s.acknowledged && !s.paused,
    );
    const secondRun = await feed(
      again.producerId,
      second,
      again.epoch,
      2048,
      800,
      400,
    );
    const reopened = await openFile();
    // Direct active-provider -> file transition must revoke the producer before
    // the client socket is closed. Do not use explicit release to mask a leak.
    const fileSwitchGone = await absent(again.producerId);
    second.close();
    const fileFrame = await host.playback.state();
    const movingFile = await until(
      "ordinary file frames resume",
      () => host.playback.state(),
      (s) => s.videoFrames > fileFrame.videoFrames && s.position > 0,
    );
    await absent(prepared.producerId);
    const result = {
      pass: true,
      prepared,
      ready,
      live,
      paused,
      held,
      resumed,
      output,
      gone,
      unused,
      releasedCandidate,
      again,
      secondLive,
      secondRun,
      fileSwitchGone,
      reopened,
      movingFile,
      observations,
      healthObservations,
      omittedHealthObservations,
      latestHealth,
      limits: [
        "Author public API smoke; not independent acceptance or NES execution.",
        "Provider ACK is a same-process synthetic emission-boundary promise, not cross-process/core frame-bound pause proof. Source operation completion is checked separately from ACK acceptance.",
        "Recoverable Xruns/deviceChanged are allowed and recorded with counters/details; passing does not mean Xrun-free.",
        "videoFrames is a host presentation counter, not a screenshot/pixel fence.",
        "No acoustic output, device drain/latency, physical GUI/input, missing-provider-ACK timeout, owner-death cleanup acknowledgement or non-Unix coverage.",
      ],
    };
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(result, null, 2),
    );
    console.error("PRODUCER_SMOKE_DONE");
    return () => {
      for (const connection of connections) connection.close();
    };
  } catch (error) {
    for (const connection of connections) connection.close();
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify(
        {
          error: String(error),
          code: error.code,
          stack: error.stack,
          observations,
          healthObservations,
          omittedHealthObservations,
          latestHealth,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}

await writeFile(
  resolve(consumer, "manifest.json"),
  JSON.stringify({
    id: "author.producer-smoke",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: [
      "producer.av",
      "playback.read",
      "playback.control",
      "events",
    ],
  }),
);
await writeFile(
  resolve(consumer, "main.mjs"),
  `export const activate = ${activate.toString()};\n`,
);
const args = [
  "--headless",
  "--silent",
  "--seconds",
  "18",
  "--plugin",
  consumer,
];
const binary =
  process.env.VESTRA_HOST_BINARY ??
  resolve(import.meta.dirname, "../host/target/debug/vestra-host");
const child = spawnSync(binary, args, {
  encoding: "utf8",
  timeout: 25000,
  maxBuffer: 8 * 1024 * 1024,
  env: { ...process.env, VESTRA_PRODUCER_SMOKE: work },
});
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(
  resolve(work, "command.json"),
  JSON.stringify(
    {
      binary,
      args,
      exitCode: child.status,
      signal: child.signal,
      error: String(child.error ?? ""),
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify({ work, exitCode: child.status, signal: child.signal }),
);
assert.equal(child.status, 0, child.stderr || String(child.error));
assert.match(child.stderr, /PRODUCER_SMOKE_DONE/);
const result = JSON.parse(
  await readFile(resolve(work, "consumer-result.json"), "utf8"),
);
assert.equal(result.pass, true);
await writeFile(
  resolve(work, "result.json"),
  JSON.stringify({ ...result, work, hashes }, null, 2),
);
console.log(JSON.stringify({ pass: true, work, limitations: result.limits }));
