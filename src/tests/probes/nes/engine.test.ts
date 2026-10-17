// 实验验收：执行自制 ROM，观察像素/APU 而不是 mock 响应。
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { Emulator } from "../../../plugins/nes/engine.ts";
import { testRom } from "./rom.ts";
const hash = (bytes: Buffer) =>
  createHash("sha256").update(bytes).digest("hex");

test("N1: real 6502 ROM changes pixels on A press and emits nonzero stereo PCM", () => {
  const nes = new Emulator(testRom());
  let frame = nes.frame();
  for (let i = 0; i < 9; i++) frame = nes.frame();
  assert.equal(frame.rgb.length, 256 * 240 * 3);
  assert.ok(frame.pcm.length > 2000);
  assert.ok(frame.pcm.some((byte) => byte !== 0));
  const before = hash(frame.rgb);
  nes.input("z", true);
  for (let i = 0; i < 3; i++) frame = nes.frame();
  assert.notEqual(hash(frame.rgb), before);
  nes.input("z", false);
  for (let i = 0; i < 3; i++) frame = nes.frame();
  assert.equal(hash(frame.rgb), before);
});

test("N2: state replays exact subsequent pixels/audio and rejects foreign/corrupt saves", () => {
  const nes = new Emulator(testRom());
  for (let i = 0; i < 10; i++) nes.frame();
  nes.input("z", true);
  for (let i = 0; i < 3; i++) nes.frame();
  const state = nes.save();
  const expected = Array.from({ length: 4 }, () => {
    const f = nes.frame();
    return [hash(f.rgb), hash(f.pcm)];
  });
  nes.input("z", false);
  for (let i = 0; i < 10; i++) nes.frame();
  nes.restore(state);
  const restored = Array.from({ length: 4 }, () => {
    const f = nes.frame();
    return [hash(f.rgb), hash(f.pcm)];
  });
  assert.deepEqual(restored, expected);
  assert.throws(() => nes.restore("{}"), /invalid/);
  const foreign = testRom();
  foreign[16 + 0x100] = 1;
  assert.throws(() => new Emulator(foreign).restore(state), /another ROM/);
  assert.throws(() => new Emulator(Buffer.from("invalid")), /iNES/);
});
