export const activate = async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const { pathToFileURL } = await import("node:url");
  const { createHash } = await import("node:crypto");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_PRODUCER_ROM_SMOKE;
  const { NES, Controller } = await import(pathToFileURL(path.join(work, "dependencies/package/src/index.js")).href);
  const rom = await fs.readFile(path.join(work, "thwaite.nes"));
  const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
  assert.equal(sha(rom), "ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173");
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const observations = [], inputs = [], frameHashes = [];
  const health = [];
  const pcmHash = createHash("sha256");
  const sampleBytes = Buffer.alloc(8);
  const audioScratch = new Float32Array(4800 * 2);
  let frameSamples = 0, lastRGBA, frameEvents = 0;
  let generatedFrames = 0, generatedAudioCursor = 0, peak = 0, nonzeroAudioFrames = 0;
  let connection, prepared, epoch = 1, sendCursor = 0, sequence = 0, operation = 0;
  const pressed = new Set();
  const core = new NES({ sampleRate: 48000,
    onFrame(words) {
      assert.equal(words.length, 256 * 240);
      const pixels = new Uint8Array(words.length * 4);
      for (let i = 0; i < words.length; i++) {
        const word = words[i];
        // Exact 2.1.0 official browser Screen.setBuffer channel ordering.
        pixels[i * 4] = word & 255; pixels[i * 4 + 1] = (word >>> 8) & 255;
        pixels[i * 4 + 2] = (word >>> 16) & 255; pixels[i * 4 + 3] = 255;
      }
      lastRGBA = pixels; frameEvents++;
      assert.ok(frameHashes.length < 2048, "bounded author frame evidence exhausted");
      frameHashes.push(sha(pixels));
    },
    onAudioSample(left, right) {
      assert.ok(Number.isFinite(left) && Number.isFinite(right) && Math.abs(left) <= 1 && Math.abs(right) <= 1);
      assert.ok(frameSamples < 4800, "single core frame exceeded packet bound");
      audioScratch[frameSamples * 2] = left; audioScratch[frameSamples * 2 + 1] = right;
      frameSamples++; generatedAudioCursor++;
      peak = Math.max(peak, Math.abs(left), Math.abs(right));
      if (left !== 0 || right !== 0) nonzeroAudioFrames++;
      sampleBytes.writeFloatLE(left, 0); sampleBytes.writeFloatLE(right, 4); pcmHash.update(sampleBytes);
    },
  });
  core.loadROM(new Uint8Array(rom));
  function coreFacts() {
    return { generatedFrames, generatedAudioCursor, peak, nonzeroAudioFrames, frameEvents,
      lastFrameSha256: lastRGBA ? sha(lastRGBA) : null, pcmSha256: pcmHash.copy().digest("hex"), sendCursor, epoch };
  }
  function step() {
    frameSamples = 0;
    const before = frameEvents;
    const generatedAudioStart = generatedAudioCursor;
    core.frame(); generatedFrames++;
    assert.equal(frameEvents, before + 1, "one NES.frame must produce one complete picture");
    assert.ok(frameSamples > 0 && frameSamples <= 1024, "fixed core/frame timing left the finite lead profile");
    return { generatedAudioStart, count: frameSamples, pcm: audioScratch.slice(0, frameSamples * 2), pixels: lastRGBA };
  }
  function button(button, down) {
    inputs.push({ generatedFrames, button, down });
    if (down) { pressed.add(button); core.buttonDown(1, button); }
    else { pressed.delete(button); core.buttonUp(1, button); }
  }
  function releaseAll(reason) {
    inputs.push({ generatedFrames, reason, released: [...pressed] });
    for (let button = 0; button < 8; button++) core.buttonUp(1, button);
    pressed.clear();
  }
  async function state() {
    const result = await host.producer.state(prepared.producerId);
    assert.ok(["ok", "recoverableXrun", "deviceChanged"].includes(result.health), JSON.stringify(result));
    assert.equal(result.error, null, JSON.stringify(result));
    return result;
  }
  async function until(label, read, predicate, milliseconds = 2500) {
    let last;
    const deadline = Date.now() + milliseconds;
    do {
      last = await read();
      if (predicate(last)) { observations.push({ label, state: last, core: coreFacts() }); return last; }
      await wait(15);
    } while (Date.now() < deadline);
    throw Error(`${label} timed out: ${JSON.stringify(last)}`);
  }
  async function emitFrame(consumed) {
    assert.ok(sendCursor >= consumed && sendCursor - consumed <= 3072);
    const frame = step();
    assert.ok(sendCursor + frame.count - consumed <= 4096, "generated PCM lead exceeded4096");
    const sampleTime = sendCursor;
    await connection.video({ epoch, sampleTime, sequence: sequence++ }, frame.pixels);
    await connection.audio({ epoch, sampleTime, sequence: sequence++ }, frame.pcm);
    sendCursor += frame.count;
    // Exactly one generated frame at a time, both writes awaited. Video PTS is
    // its first real audio sample position, never a guessed 800-sample stride.
    return { generatedFrames, generatedAudioStart: frame.generatedAudioStart, epoch, sampleTime, audioFrames: frame.count };
  }
  async function feed(milliseconds) {
    const initial = await state(), coreBefore = coreFacts(), frameBefore = await host.playback.state();
    const deadline = Date.now() + milliseconds;
    const mapping = [];
    while (Date.now() < deadline) {
      const current = await state();
      assert.equal(current.epoch, epoch); assert.equal(current.paused, false);
      assert.ok(sendCursor >= current.consumedFrames);
      if (sendCursor - current.consumedFrames <= 3072) mapping.push(await emitFrame(current.consumedFrames));
      else await wait(8);
      assert.ok(mapping.length <= 256, "bounded author feed exceeded frame limit");
    }
    const after = await state(), coreAfter = coreFacts();
    const frameAfter = await until("host frame count advances", () => host.playback.state(), (s) => s.videoFrames > frameBefore.videoFrames);
    assert.ok(coreAfter.generatedFrames > coreBefore.generatedFrames);
    assert.ok(coreAfter.generatedAudioCursor > coreBefore.generatedAudioCursor);
    assert.ok(after.consumedFrames > initial.consumedFrames + 4800);
    health.push({ initial, after });
    observations.push({ label: "bounded real-core feed", coreBefore, coreAfter, initial, after, frameBefore, frameAfter, mapping });
    return after;
  }
  async function control(paused) {
    // No concurrent frame loop exists: all prior NES.frame and socket writes
    // have returned before the request. Core execution remains frozen below.
    const boundary = coreFacts();
    releaseAll(paused ? "pause boundary" : "resume boundary");
    const prior = await host.source.state();
    const requestId = ++operation;
    const pending = await host.source.submit({ kind: "control", control: { kind: "pause", paused }, expected: prior.stamp }, requestId);
    assert.equal(pending.status, "pending");
    const noticed = await until("frame-boundary control notice", state, (s) => s.pendingControl?.paused === paused);
    const notice = noticed.pendingControl;
    assert.deepEqual(notice.stamp, pending.ticket.stamp);
    assert.equal(notice.cycle, pending.ticket.cycle);
    assert.equal(notice.producerId, prepared.producerId);
    const oldSendCursor = sendCursor;
    if (paused) {
      const discardedQueuedAudioFrames = Math.max(0, oldSendCursor - notice.baseSample);
      assert.ok(discardedQueuedAudioFrames <= 4096);
      observations.push({ label: "pause rebase and bounded dropped pre-generated PCM", boundary, hostConsumed: noticed.consumedFrames, notice, oldSendCursor, discardedQueuedAudioFrames,
        mapping: "subsequent new core PCM will map to the new epoch base; no checkpoint rewind or zero-drop claim" });
      epoch = notice.epoch; sendCursor = notice.baseSample;
      // Discard this old frame scratch; there is no unsent packet queue. Old
      // already queued host PCM is intentionally flushed by the epoch reset.
      frameSamples = 0; audioScratch.fill(0);
    } else {
      assert.equal(notice.epoch, epoch);
      assert.equal(notice.baseSample, sendCursor);
    }
    assert.equal(generatedFrames, boundary.generatedFrames);
    const ack = await host.producer.ack(notice);
    assert.equal(ack.status, "acknowledged"); assert.deepEqual(ack.control, notice);
    const completed = await until("original source control completes", () => host.source.operation(requestId), (r) => {
      assert.notEqual(r.status, "failed", JSON.stringify(r)); return r.status === "completed";
    });
    assert.deepEqual(completed.ticket, pending.ticket);
    const current = await until("host callback acknowledges control", state, (s) => s.epoch === notice.epoch && s.acknowledged && s.paused === paused && s.pendingControl === null);
    assert.equal(generatedFrames, boundary.generatedFrames);
    observations.push({ label: "real-core between-frame control", paused, boundary, notice, ack, completed, current, core: coreFacts() });
    return current;
  }
  try {
    // Same two Start transitions as the archived real-core prerequisite. This
    // deterministic warmup happens before opening any producer audio device.
    for (let i = 0; i < 180; i++) step();
    button(Controller.BUTTON_START, true); for (let i = 0; i < 2; i++) step();
    button(Controller.BUTTON_START, false); for (let i = 0; i < 178; i++) step();
    const menu = coreFacts();
    button(Controller.BUTTON_START, true); for (let i = 0; i < 2; i++) step();
    button(Controller.BUTTON_START, false); for (let i = 0; i < 178; i++) step();
    releaseAll("warmup complete");
    const warmup = coreFacts();
    assert.ok(nonzeroAudioFrames > menu.nonzeroAudioFrames, "game must generate real nonzero PCM");
    assert.ok(new Set(frameHashes).size > 2);
    await fs.writeFile(path.join(work, "warmup-game.rgba"), lastRGBA);
    await fs.writeFile(path.join(work, "warmup.json"), JSON.stringify({ menu, warmup, inputs, channelOrder: "low-byte R, then G, then B, alpha255" }, null, 2));
    prepared = await host.producer.prepare({ width: 256, height: 240 });
    assert.equal(prepared.sampleRate, 48000); assert.equal(prepared.channels, 2); assert.equal(prepared.epoch, 1);
    connection = await host.producer.connect(prepared);
    await until("candidate epoch acknowledged", state, (s) => s.acknowledged && s.paused);
    const prefill = [];
    while (sendCursor < 2048) prefill.push(await emitFrame(0));
    const ready = await until("real ROM AV ready", state, (s) => s.acknowledged && s.hasVideo && s.receivedFrames >= 1024);
    const prior = await host.source.state();
    const opened = await host.source.submit({ kind: "openProvider", producerId: prepared.producerId, startPaused: false, expected: prior.stamp }, ++operation);
    assert.notEqual(opened.status, "failed", JSON.stringify(opened));
    const live = await until("ROM source ready", () => host.source.state(), (s) => s.phase === "ready" && s.stamp.generation === opened.ticket.stamp.generation);
    await until("ROM callback running", state, (s) => s.active && s.acknowledged && !s.paused);
    const firstRun = await feed(700);
    const paused = await control(true);
    const freeze = coreFacts(), pausedSource = await host.source.state();
    await wait(220);
    const held = await state(), heldSource = await host.source.state();
    assert.equal(generatedFrames, freeze.generatedFrames);
    assert.equal(generatedAudioCursor, freeze.generatedAudioCursor);
    assert.equal(held.consumedFrames, paused.consumedFrames);
    assert.equal(heldSource.position, pausedSource.position); assert.equal(heldSource.paused, true);
    observations.push({ label: "actual core and clock frozen for220ms", freeze, paused, held, pausedSource, heldSource });
    const resumed = await control(false);
    const secondRun = await feed(700);
    const closeSource = await host.source.state();
    releaseAll("source close");
    const closed = await host.source.submit({ kind: "close", expected: closeSource.stamp }, ++operation);
    assert.notEqual(closed.status, "failed", JSON.stringify(closed));
    await until("ROM source closed", () => host.source.state(), (s) => s.phase === "empty");
    // Observe removal before destroying the client socket or explicit release.
    await assert.rejects(() => host.producer.state(prepared.producerId), (error) => error.code === "unavailable");
    connection.close();
    const result = { pass: true, jsnes: "2.1.0", romSha256: sha(rom), warmup, prepared, prefill, ready, live, firstRun, paused, held, resumed, secondRun, finalCore: coreFacts(), inputs, frameHashes, health, observations,
      limits: ["Author real-core/host AV slice only, not independent acceptance or a product plugin.", "No GUI/screen pixels, acoustic measurement, ten-minute stability, physical keyboard, shared save/reset/newROM controls or cross-owner ACK test.", "Pause freezes between NES.frame calls; the core may lead actual consumed PCM by at most4096 samples. Pause flushes this bounded pre-generated PCM; resume rebases subsequent samples to Notice.baseSample. No zero-drop or exact-consumed-frame checkpoint claim.", "Raw RGBA summaries and host videoFrames are not visible-screen or calibrated hardware-color proof.", "Recoverable Xruns/deviceChanged remain recorded, not an Xrun-free claim."] };
    await fs.writeFile(path.join(work, "consumer-result.json"), JSON.stringify(result, null, 2));
    console.error("PRODUCER_ROM_SMOKE_DONE");
    return () => connection.close();
  } catch (error) {
    connection?.close();
    await fs.writeFile(path.join(work, "consumer-failure.json"), JSON.stringify({ error: String(error), code: error.code, stack: error.stack, core: coreFacts(), inputs, health, observations }, null, 2));
    throw error;
  }
};
