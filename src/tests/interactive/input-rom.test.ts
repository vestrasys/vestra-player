// 实验样本自检：真实 JSNES 执行 CPU/PPU/APU；不宣称已验证物理键盘。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Emulator } from "../../plugins/nes/engine.ts";
import { inputKeys, inputRom, releaseFrames } from "./input-rom.ts";

test("interactive CC0 ROM gives each of eight real controller keys a distinct visible color", async () => {
  const emulator = new Emulator(inputRom());
  const frame = () => {
    let output = emulator.frame();
    for (let index = 0; index < 4; index++) output = emulator.frame();
    return output;
  };
  const pixel = (rgb: Buffer) => [
    ...rgb.subarray((120 * 256 + 128) * 3, (120 * 256 + 128) * 3 + 3),
  ];
  const baseline = pixel(frame().rgb);
  const oracle: Record<string, number[]> = { released: baseline };
  const colors = new Set<string>();
  for (const key of inputKeys) {
    emulator.input(key, true);
    const pressed = frame();
    const color = pixel(pressed.rgb);
    assert.notDeepEqual(
      color,
      baseline,
      `${key} must visibly differ from released`,
    );
    assert(
      pressed.pcm.some((byte) => byte !== 0),
      "real APU audio must be nonzero",
    );
    assert(
      !colors.has(color.join(",")),
      `${key} must have a distinct observable color`,
    );
    colors.add(color.join(","));
    oracle[key] = color;
    emulator.input(key, false);
    assert.deepEqual(
      pixel(frame().rgb),
      color,
      `${key} tap stays observable briefly`,
    );
    for (let index = 0; index <= releaseFrames; index++) emulator.frame();
    assert.deepEqual(
      pixel(frame().rgb),
      baseline,
      `${key} release must eventually clear the color`,
    );
    emulator.input(key, true);
    for (let index = 0; index <= releaseFrames; index++) emulator.frame();
    assert.deepEqual(
      pixel(frame().rgb),
      color,
      `${key} held input must renew TTL rather than clear`,
    );
    emulator.input(key, false);
    for (let index = 0; index <= releaseFrames; index++) emulator.frame();
  }
  const directory = fileURLToPath(
    new URL("../../../.cache/interactive/fixtures/", import.meta.url),
  );
  await mkdir(directory, { recursive: true });
  await writeFile(resolve(directory, "input-8keys.nes"), inputRom());
  await writeFile(
    resolve(directory, "input-8keys-oracle.json"),
    JSON.stringify(
      {
        license: "CC0-1.0",
        experimental: true,
        physicalInputVerified: false,
        releaseTimeoutFrames: releaseFrames,
        description:
          "Real 6502 ROM, serial $4016 controller reads; independent expected RGB for native input verification",
        oracle,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify(oracle));
});
