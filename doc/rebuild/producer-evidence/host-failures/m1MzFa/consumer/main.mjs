export const activate = async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_PRODUCER_SMOKE;
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const observations = [];
  const connections = [];
  let operation = 0;
  let sequence = 0;
  let videoNumber = 0;
  const pcmBytes = await fs.readFile(path.join(work, "audio.f32"));
  const view = new DataView(pcmBytes.buffer, pcmBytes.byteOffset, pcmBytes.byteLength);
  const pcm = new Float32Array(pcmBytes.length / 4);
  for (let i = 0; i < pcm.length; i++) pcm[i] = view.getFloat32(i * 4, true);
  const rgba = await fs.readFile(path.join(work, "video.rgba"));
  function samples(start, frames) {
    const result = new Float32Array(frames * 2);
    for (let i = 0; i < frames; i++) {
      const source = ((start + i) % 48000) * 2;
      result[i * 2] = pcm[source]; result[i * 2 + 1] = pcm[source + 1];
    }
    return result;
  }
  async function video(connection, epoch, sampleTime) {
    const start = (videoNumber++ % 8) * 256 * 240 * 4;
    await connection.video({ epoch, sampleTime, sequence: sequence++ }, new Uint8Array(rgba.buffer, rgba.byteOffset + start, 256 * 240 * 4));
  }
  async function audio(connection, epoch, sampleTime, frames) {
    await connection.audio({ epoch, sampleTime, sequence: sequence++ }, samples(sampleTime, frames));
  }
  async function until(label, read, predicate, milliseconds = 2500) {
    const deadline = Date.now() + milliseconds;
    let last;
    do {
      last = await read();
      if (predicate(last)) { observations.push({ label, state: last }); return last; }
      await wait(15);
    } while (Date.now() < deadline);
    throw Error(`${label} timed out: ${JSON.stringify(last)}`);
  }
  async function submit(intent) {
    const result = await host.source.submit(intent, ++operation);
    assert.notEqual(result.status, "failed", JSON.stringify(result));
    return result;
  }
  async function rejectIntent(label, intent, code) {
    const id = ++operation;
    let error;
    try {
      const result = await host.source.submit(intent, id);
      assert.equal(result.status, "failed", `${label} unexpectedly accepted: ${JSON.stringify(result)}`);
      error = { code: result.error, result };
    } catch (failure) {
      if (failure.name === "AssertionError") throw failure;
      error = { code: failure.code, message: String(failure) };
    }
    assert.equal(error.code, code, `${label}: ${JSON.stringify(error)}`);
    let query;
    try { query = await host.source.operation(id); }
    catch (failure) { query = { error: failure.code, message: String(failure) }; }
    assert.notEqual(query.status, "pending", `${label} left a pending operation`);
    assert.notEqual(query.error, "timeout", `${label} operation query timed out`);
    observations.push({ label, error, operation: query });
  }
  async function absent(id) {
    try { await host.producer.state(id); assert.fail("released producer remains addressable"); }
    catch (failure) {
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
    const accepted = await submit({ kind: "open", source: path.join(work, "source.mp4"), startPaused: false });
    return until("ordinary file ready", () => host.source.state(), (s) => s.phase === "ready" && s.stamp.generation === accepted.ticket.stamp.generation, 4000);
  }
  async function feed(id, connection, epoch, nextAudio, nextVideo, milliseconds) {
    const start = await host.producer.state(id);
    const firstFrame = await host.playback.state();
    const deadline = Date.now() + milliseconds;
    let last = start;
    while (Date.now() < deadline) {
      last = await host.producer.state(id);
      assert.equal(last.error, null, JSON.stringify(last));
      assert.equal(last.epoch, epoch); assert.equal(last.paused, false);
      // At most four packets per turn and 4096 frames ahead. Every packet is
      // awaited on the SDK's socket write completion; no unbounded Promise fanout.
      for (let n = 0; n < 4 && nextAudio - last.consumedFrames < 3072; n++) {
        assert.ok(nextAudio >= last.consumedFrames, "producer underrun consumed absent PCM");
        const count = Math.min(1024, 4096 - (nextAudio - last.consumedFrames));
        assert.ok(count >= 512 && count <= 1024);
        await audio(connection, epoch, nextAudio, count); nextAudio += count;
      }
      for (let n = 0; n < 3 && nextVideo <= last.consumedFrames + 1600; n++) {
        await video(connection, epoch, nextVideo); nextVideo += 800;
      }
      await wait(10);
    }
    last = await host.producer.state(id);
    const finalFrame = await until("producer video frame counter advances", () => host.playback.state(), (s) => s.videoFrames > firstFrame.videoFrames);
    assert.ok(last.consumedFrames > start.consumedFrames + 4800, "matching PCM media clock did not advance");
    assert.ok(last.underrunFrames >= start.underrunFrames);
    observations.push({ label: "feed", start, last, firstFrame, finalFrame, nextAudio, nextVideo });
    return { nextAudio, nextVideo, state: last };
  }
  try {
    const initial = await openFile();
    const prepared = await host.producer.prepare({ width: 256, height: 240 });
    assert.equal(prepared.epoch, 1); assert.equal(prepared.sampleRate, 48000);
    assert.equal(prepared.channels, 2); assert.equal(prepared.videoCapacity, 8);
    await rejectIntent("not ready preserves ordinary source", { kind: "openProvider", producerId: prepared.producerId, startPaused: false, expected: initial.stamp }, "unavailable");
    assert.deepEqual((await host.source.state()).stamp, initial.stamp);
    const connection = await host.producer.connect(prepared); connections.push(connection);
    await until("candidate callback epoch acknowledged", () => host.producer.state(prepared.producerId), (s) => {assert.equal(s.deviceErrors,0,JSON.stringify(s));return s.epoch === 1 && s.acknowledged && s.paused;});
    await video(connection, 1, 0); await audio(connection, 1, 0, 2048);
    const ready = await until("actual prefilled AV candidate", () => host.producer.state(prepared.producerId), (s) => s.acknowledged && s.hasVideo && s.receivedFrames >= 1024);
    assert.equal(ready.active, false);
    const opened = await submit({ kind: "openProvider", producerId: prepared.producerId, startPaused: false, expected: initial.stamp });
    const live = await until("producer public source ready", () => host.source.state(), (s) => s.phase === "ready" && s.stamp.generation === opened.ticket.stamp.generation);
    for (const key of ["seek", "rate", "snapshotSource"]) assert.equal(live.capabilities[key], false, key);
    await until("producer unpaused callback ack", () => host.producer.state(prepared.producerId), (s) => s.active && s.acknowledged && !s.paused);
    let cursor = await feed(prepared.producerId, connection, 1, 2048, 800, 600);
    let before = await host.source.state();
    for (const control of [{ kind: "seek", seconds: 1 }, { kind: "rate", rate: 1.5 }]) {
      await rejectIntent(`unsupported ${control.kind}`, { kind: "control", control, expected: before.stamp }, "unsupported");
      assert.deepEqual((await host.source.state()).stamp, before.stamp);
    }
    await submit({ kind: "control", control: { kind: "pause", paused: true }, expected: before.stamp });
    const paused = await until("new pause epoch acknowledged", () => host.producer.state(prepared.producerId), (s) => s.epoch > 1 && s.acknowledged && s.paused);
    const pausedSource = await host.source.state();
    await audio(connection, 1, cursor.nextAudio, 512); // explicit stale epoch packet
    await wait(320);
    const held = await host.producer.state(prepared.producerId);
    const heldSource = await host.source.state();
    assert.equal(held.epoch, paused.epoch);
    assert.equal(held.receivedFrames, paused.receivedFrames, "old-epoch PCM polluted new epoch");
    assert.equal(held.consumedFrames, paused.consumedFrames);
    assert.equal(heldSource.paused, true); assert.equal(heldSource.position, pausedSource.position);
    observations.push({ label: "host pause epoch isolation (not provider frame-bound ack)", paused, held, pausedSource, heldSource });
    const base = held.consumedFrames;
    await video(connection, held.epoch, base); await audio(connection, held.epoch, base, 2048);
    await until("new pause epoch prefilled", () => host.producer.state(prepared.producerId), (s) => s.receivedFrames >= held.receivedFrames + 2048 && s.hasVideo);
    before = await host.source.state();
    await submit({ kind: "control", control: { kind: "pause", paused: false }, expected: before.stamp });
    await until("same epoch resume acknowledged", () => host.producer.state(prepared.producerId), (s) => s.epoch === held.epoch && s.acknowledged && !s.paused);
    cursor = await feed(prepared.producerId, connection, held.epoch, base + 2048, base + 800, 600);
    before = await host.source.state();
    await host.playback.set("muted", true); await host.playback.set("volume", 17);
    const output = await until("public output controls confirmed", () => host.source.state(), (s) => s.output.muted && s.output.volume === 17);
    assert.deepEqual(output.stamp, before.stamp, "device output changed source intent");
    observations.push({ label: "shared output state (not audible measurement)", source: output });
    await host.playback.set("muted", false); await host.playback.set("volume", 100);
    before = await host.source.state();
    await submit({ kind: "close", expected: before.stamp });
    const empty = await until("producer source closed", () => host.source.state(), (s) => s.phase === "empty");
    // Query BEFORE closing the socket or calling release: source.close itself
    // must have removed the active producer, not a later cleanup action.
    const gone = await absent(prepared.producerId);
    connection.close();
    const unused = await host.producer.prepare({ width: 256, height: 240 });
    const releasedCandidate = await release(unused.producerId);
    observations.push({ label: "explicit release of never-activated candidate", unused, releasedCandidate });
    const again = await host.producer.prepare({ width: 256, height: 240 });
    const second = await host.producer.connect(again); connections.push(second);
    await until("second candidate ack", () => host.producer.state(again.producerId), (s) => s.acknowledged && s.paused);
    await video(second, again.epoch, 0); await audio(second, again.epoch, 0, 2048);
    await until("second candidate AV ready", () => host.producer.state(again.producerId), (s) => s.acknowledged && s.hasVideo && s.receivedFrames >= 1024);
    await rejectIntent("old source CAS cannot activate ready candidate", { kind: "openProvider", producerId: again.producerId, startPaused: false, expected: initial.stamp }, "stale");
    assert.deepEqual((await host.source.state()).stamp, empty.stamp);
    const activatedAgain = await submit({ kind: "openProvider", producerId: again.producerId, startPaused: false, expected: empty.stamp });
    const secondLive = await until("second producer actually active", () => host.source.state(), (s) => s.phase === "ready" && s.stamp.generation === activatedAgain.ticket.stamp.generation);
    await until("second producer running callback ack", () => host.producer.state(again.producerId), (s) => s.active && s.acknowledged && !s.paused);
    const secondRun = await feed(again.producerId, second, again.epoch, 2048, 800, 400);
    const reopened = await openFile();
    // Direct active-provider -> file transition must revoke the producer before
    // the client socket is closed. Do not use explicit release to mask a leak.
    const fileSwitchGone = await absent(again.producerId);
    second.close();
    const fileFrame = await host.playback.state();
    const movingFile = await until("ordinary file frames resume", () => host.playback.state(), (s) => s.videoFrames > fileFrame.videoFrames && s.position > 0);
    await absent(prepared.producerId);
    const result = { pass: true, prepared, ready, live, paused, held, output, gone, unused, releasedCandidate, again, secondLive, secondRun, fileSwitchGone, reopened, movingFile, observations,
      limits: ["Author public API smoke; not independent acceptance or NES execution.", "Epoch acknowledged is host audio-callback acknowledgement, not provider frame-bound pause acknowledgement.", "videoFrames is a host presentation counter, not a screenshot/pixel fence.", "No acoustic output, device drain/latency, physical GUI/input, owner-death cleanup acknowledgement or non-Unix coverage."] };
    await fs.writeFile(path.join(work, "consumer-result.json"), JSON.stringify(result, null, 2));
    console.error("PRODUCER_SMOKE_DONE");
    return () => { for (const connection of connections) connection.close(); };
  } catch (error) {
    for (const connection of connections) connection.close();
    await fs.writeFile(path.join(work, "consumer-failure.json"), JSON.stringify({ error: String(error), code: error.code, stack: error.stack, observations }, null, 2));
    throw error;
  }
};
