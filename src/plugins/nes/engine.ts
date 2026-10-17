/// <reference path="./jsnes.d.ts" />
// 实验 NES 适配器；最终插件 API 可能相差很大。JSNES 执行真实 6502/PPU/APU。
import { createHash } from "node:crypto";
import { NES, Controller, type EmulatorData } from "jsnes";

const buttons = new Map<string, number>([
  ["z", Controller.BUTTON_A],
  ["x", Controller.BUTTON_B],
  ["Shift", Controller.BUTTON_SELECT],
  ["Enter", Controller.BUTTON_START],
  ["ArrowUp", Controller.BUTTON_UP],
  ["ArrowDown", Controller.BUTTON_DOWN],
  ["ArrowLeft", Controller.BUTTON_LEFT],
  ["ArrowRight", Controller.BUTTON_RIGHT],
]);
export interface OutputFrame {
  rgb: Buffer;
  pcm: Buffer;
}
export class Emulator {
  private readonly nes: NES;
  private rgb = Buffer.alloc(256 * 240 * 3);
  private samples: number[] = [];
  readonly digest: string;
  constructor(rom: Buffer) {
    if (
      rom.length < 16 ||
      !rom.subarray(0, 4).equals(Buffer.from([78, 69, 83, 26]))
    ) {
      throw new Error("Not an iNES ROM");
    }
    this.digest = createHash("sha256").update(rom).digest("hex");
    this.nes = new NES({
      sampleRate: 44100,
      emulateSound: true,
      onFrame: (pixels) => {
        for (let i = 0; i < pixels.length; i++) {
          const color = pixels[i];
          this.rgb[i * 3] = color & 255;
          this.rgb[i * 3 + 1] = (color >> 8) & 255;
          this.rgb[i * 3 + 2] = (color >> 16) & 255;
        }
      },
      onAudioSample: (left, right) => {
        this.samples.push(Math.round(Math.max(-1, Math.min(1, left)) * 32767));
        this.samples.push(Math.round(Math.max(-1, Math.min(1, right)) * 32767));
      },
    });
    this.nes.loadROM(rom);
  }
  frame(): OutputFrame {
    this.samples = [];
    this.nes.frame();
    const pcm = Buffer.alloc(this.samples.length * 2);
    this.samples.forEach((sample, index) =>
      pcm.writeInt16LE(sample, index * 2),
    );
    return { rgb: Buffer.from(this.rgb), pcm };
  }
  input(key: string, pressed: boolean) {
    const button = buttons.get(key.length === 1 ? key.toLowerCase() : key);
    if (button !== undefined) {
      if (pressed) this.nes.buttonDown(1, button);
      else this.nes.buttonUp(1, button);
    }
  }
  save(): string {
    return JSON.stringify({
      version: 1,
      rom: this.digest,
      state: this.nes.toJSON(),
    });
  }
  restore(serialized: string) {
    const value: unknown = JSON.parse(serialized);
    if (
      !value ||
      typeof value !== "object" ||
      !("version" in value) ||
      value.version !== 1 ||
      !("rom" in value) ||
      value.rom !== this.digest ||
      !("state" in value) ||
      !isState(value.state)
    ) {
      throw new Error("Save state is invalid or belongs to another ROM");
    }
    // Envelope validation; callers restore into a candidate core before replacing the live instance.
    this.nes.fromJSON(value.state);
  }
}
function isState(value: unknown): value is EmulatorData {
  return (
    !!value &&
    typeof value === "object" &&
    ["cpu", "mmap", "ppu", "papu"].every(
      (key) =>
        key in value &&
        typeof Reflect.get(value, key) === "object" &&
        Reflect.get(value, key) !== null,
    )
  );
}
