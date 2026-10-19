/** Experiment only: exact JSNES/Thwaite headless prerequisite, no host or devices. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile, copyFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const directory = import.meta.dirname;
const output = resolve(directory, 'active-game-run');
await mkdir(output, {recursive:true});
const preparation = JSON.parse(await readFile(resolve(directory, 'preparation.json'), 'utf8'));
const { NES, Controller } = await import(pathToFileURL(resolve(preparation.work, 'package/src/index.js')).href);
const rom = await readFile(resolve(directory, '../nes-fixture-preparation/thwaite.nes'));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
assert.equal(sha(rom), 'ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173');
await copyFile(resolve(preparation.work, 'package/LICENSE'), resolve(directory, 'jsnes-LICENSE'));
const started = performance.now();

// Match the exact package's official browser Screen.setBuffer, which ORs raw
// low24 BGR words with 0xff000000 for a little-endian RGBA canvas. The README's
// ARGB label alone is insufficient; avoid a native-endian buffer cast.
function rgba(words) {
  assert.equal(words.length, 256 * 240);
  const result = Buffer.alloc(words.length * 4);
  for (let i = 0; i < words.length; i++) {
    const p = words[i];
    result[i * 4] = p & 255;
    result[i * 4 + 1] = (p >>> 8) & 255;
    result[i * 4 + 2] = (p >>> 16) & 255;
    result[i * 4 + 3] = 255;
  }
  return result;
}
function machine() {
  const stats = { frames: 0, audioFrames: 0, peak: 0, nonzero: 0, last: null, frameHashes: [], pcmHash: createHash('sha256'), audioCounts: [] };
  const scratch = Buffer.alloc(8);
  const nes = new NES({
    sampleRate: 48000,
    onFrame(words) {
      stats.frames++;
      stats.last = rgba(words);
      stats.frameHashes.push(sha(stats.last));
      stats.audioCounts.push(stats.audioFrames);
    },
    onAudioSample(left, right) {
      assert.ok(Number.isFinite(left) && Number.isFinite(right) && Math.abs(left) <= 1 && Math.abs(right) <= 1);
      stats.audioFrames++;
      stats.peak = Math.max(stats.peak, Math.abs(left), Math.abs(right));
      if (left !== 0 || right !== 0) stats.nonzero++;
      scratch.writeFloatLE(left, 0); scratch.writeFloatLE(right, 4);
      stats.pcmHash.update(scratch);
    },
  });
  nes.loadROM(new Uint8Array(rom));
  return { nes, stats };
}
function releaseAll(nes) {
  for (let button = 0; button < 8; button++) nes.buttonUp(1, button);
}
function advance(nes, frames) { for (let i = 0; i < frames; i++) nes.frame(); }
function replay(nes) {
  for (let i = 0; i < 90; i++) {
    if (i === 0) nes.buttonDown(1, Controller.BUTTON_RIGHT);
    if (i === 24) nes.buttonUp(1, Controller.BUTTON_RIGHT);
    if (i === 30) nes.buttonDown(1, Controller.BUTTON_A);
    if (i === 34) nes.buttonUp(1, Controller.BUTTON_A);
    nes.frame();
  }
  releaseAll(nes);
}
function resetStats(stats) {
  stats.frames = 0; stats.audioFrames = 0; stats.nonzero = 0; stats.peak = 0;
  stats.frameHashes = []; stats.audioCounts = []; stats.pcmHash = createHash('sha256');
}

const a = machine();
try {
  advance(a.nes, 180);
  await writeFile(resolve(output, 'title.rgba'), a.stats.last);
  a.nes.buttonDown(1, Controller.BUTTON_START); advance(a.nes, 2);
  a.nes.buttonUp(1, Controller.BUTTON_START); advance(a.nes, 178);
  await writeFile(resolve(output, 'menu.rgba'), a.stats.last);
  const menuAudio = a.stats.nonzero;
  a.nes.buttonDown(1, Controller.BUTTON_START); advance(a.nes, 2);
  a.nes.buttonUp(1, Controller.BUTTON_START); advance(a.nes, 178);
  await writeFile(resolve(output, 'game.rgba'), a.stats.last);
  const warmup = { frames: a.stats.frames, audioFrames: a.stats.audioFrames, peak: a.stats.peak, nonzero: a.stats.nonzero, distinctFrames: new Set(a.stats.frameHashes).size, activeGameNonzeroAudio: a.stats.nonzero-menuAudio };
  assert.ok(warmup.audioFrames > 48000 && warmup.nonzero > 0 && warmup.distinctFrames > 2);
  releaseAll(a.nes);
  const savedText = JSON.stringify(a.nes.toJSON());
  assert.ok(Buffer.byteLength(savedText) < 16 * 1024 * 1024);
  await writeFile(resolve(output, 'state.json'), savedText);
  resetStats(a.stats);
  replay(a.nes);
  const expected = { frameHashes: a.stats.frameHashes, audioCounts: a.stats.audioCounts, pcmSha256: a.stats.pcmHash.digest('hex'), audioFrames: a.stats.audioFrames, peak: a.stats.peak };
  const b = machine();
  b.nes.fromJSON(JSON.parse(savedText));
  releaseAll(b.nes); resetStats(b.stats); replay(b.nes);
  const actual = { frameHashes: b.stats.frameHashes, audioCounts: b.stats.audioCounts, pcmSha256: b.stats.pcmHash.digest('hex'), audioFrames: b.stats.audioFrames, peak: b.stats.peak };
  await writeFile(resolve(output, 'restored.rgba'), b.stats.last);
  const result = {
    pass: false, jsnes: '2.1.0', romSha256: sha(rom), warmup,
    stateBytes: Buffer.byteLength(savedText), stateSha256: sha(savedText),
    expected, actual,
    frameSequenceExact: JSON.stringify(expected.frameHashes) === JSON.stringify(actual.frameHashes),
    audioCountsExact: JSON.stringify(expected.audioCounts) === JSON.stringify(actual.audioCounts),
    audioSamplesExact: expected.pcmSha256 === actual.pcmSha256,
    wallSeconds: (performance.now() - started) / 1000,
    limits: ['Headless core only: no host, producer transport, audio device, native UI or physical input.', 'Fixed package default timing and one freshly built mapper0 ROM; no PAL/all-ROM compatibility claim.', 'Color conversion matches this package official browser implementation, not calibrated hardware colors.', 'Direct core snapshot replay is not the plugin save-container format or atomic publication.'],
  };
  result.pass = result.frameSequenceExact && result.audioCountsExact && result.audioSamplesExact;
  await writeFile(resolve(output, 'result.json'), JSON.stringify(result, null, 2));
  assert.ok(result.pass, 'exact fixed-core state replay mismatch; inspect raw result');
  console.log(JSON.stringify({ pass: result.pass, warmup, stateBytes: result.stateBytes, frameSequenceExact: result.frameSequenceExact, audioCountsExact: result.audioCountsExact, audioSamplesExact: result.audioSamplesExact, wallSeconds: result.wallSeconds }));
} catch (error) {
  await writeFile(resolve(output, 'failure.json'), JSON.stringify({ message: String(error), stack: error.stack, frames: a.stats.frames, audioFrames: a.stats.audioFrames, peak: a.stats.peak }, null, 2));
  throw error;
}
