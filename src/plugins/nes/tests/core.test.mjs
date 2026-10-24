import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { NesCore, rgbaFrame } from '../core.ts';

import { homebrew } from './homebrew.mjs';

test('JSNES runs original 6502 ROM and yields distinct PPU frames plus audible PCM', () => {
  const hashes = [];
  for (const color of [0x21, 0x16]) {
    const core = new NesCore(homebrew(color));
    let samples = 0, energy = 0, frame;
    for (let index = 0; index < 120; index++) {
      frame = core.frame();
      assert.equal(frame.video.length, 256 * 240 * 4);
      assert.equal(frame.audio.length % 2, 0);
      for (const sample of frame.audio) {
        assert.ok(Number.isFinite(sample) && Math.abs(sample) <= 1);
        energy += sample * sample;
      }
      samples += frame.audio.length / 2;
    }
    assert.ok(samples > 95000 && samples < 97000, `48 kHz sample count: ${samples}`);
    assert.ok(energy > 1, `APU signal energy: ${energy}`);
    assert.ok(frame.video.every((value, i) => i % 4 !== 3 || value === 255));
    hashes.push(createHash('sha256').update(frame.video).digest('hex'));
    console.log(JSON.stringify({ color, frames: 120, stereoFrames: samples, energy, videoSha256: hashes.at(-1) }));
  }
  assert.notEqual(hashes[0], hashes[1], '6502-selected PPU colors must change the rendered frame');
});

test('pixel channel conversion and malformed ROM errors', () => {
  const pixels = new Uint32Array(256 * 240);
  pixels[0] = 0x00563412;
  assert.deepEqual([...rgbaFrame(pixels).slice(0, 4)], [0x12, 0x34, 0x56, 255]);
  assert.throws(() => rgbaFrame(new Uint32Array(1)), /invalid size/);
  assert.throws(() => new NesCore(new Uint8Array(100)), /iNES/);
  const unsupported = homebrew();
  unsupported[6] = 0xf0;
  unsupported[7] = 0xf0;
  assert.throws(() => new NesCore(unsupported), { message: 'JSNES does not support mapper 255 of this ROM.' });
  assert.throws(() => new NesCore(homebrew().subarray(0, 16 + 16384)), /incomplete/); // CHR bank missing
  const trainer = new Uint8Array(512 + homebrew().length);
  trainer.set(homebrew().subarray(0, 16));
  trainer[6] |= 4; // a trainer precedes PRG: 512 bytes more are needed
  assert.doesNotThrow(() => new NesCore(trainer.subarray(0, 512 + homebrew().length)));
  assert.throws(() => new NesCore(trainer.subarray(0, homebrew().length)), /incomplete/);
});
