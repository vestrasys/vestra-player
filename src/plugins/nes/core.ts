import type { I18n } from '@vestra-player/sdk';
import { NES } from 'jsnes';

/** `plugin.i18n.error`: the player shows these failures translated (the open's failure, the window). */
export type Fail = I18n['error'];

/** JSNES returns 0x00BBGGRR pixels; the public producer uses byte-order RGBA. */
export function rgbaFrame(pixels: Uint32Array, error: Fail): Uint8Array {
  if (pixels.length !== 256 * 240) throw error('The emulator frame has an invalid size.');
  const rgba = new Uint8Array(pixels.length * 4);
  for (let i = 0; i < pixels.length; i++) {
    const pixel = pixels[i]!;
    rgba[i * 4] = pixel & 255;
    rgba[i * 4 + 1] = (pixel >>> 8) & 255;
    rgba[i * 4 + 2] = (pixel >>> 16) & 255;
    rgba[i * 4 + 3] = 255;
  }
  return rgba;
}

export class NesCore {
  private video: Uint8Array = new Uint8Array(256 * 240 * 4);
  private samples: number[] = [];
  readonly nes: NES;
  private readonly error: Fail;
  constructor(rom: Uint8Array, error: Fail) {
    this.error = error;
    if (rom.length < 16 || rom[0] !== 78 || rom[1] !== 69 || rom[2] !== 83 || rom[3] !== 26)
      throw error('This is not an NES ROM in iNES format.');
    // Header: PRG banks × 16 KiB, CHR banks × 8 KiB, an optional 512-byte trainer.
    if (rom.length < 16 + (rom[6]! & 4 ? 512 : 0) + rom[4]! * 16384 + rom[5]! * 8192)
      throw error('The ROM file is incomplete: shorter than its header says.');
    this.nes = new NES({ sampleRate: 48000, emulateSound: true,
      onFrame: pixels => { this.video = rgbaFrame(pixels, error); },
      onAudioSample: (left, right) => {
        this.samples.push(Math.max(-1, Math.min(1, left)), Math.max(-1, Math.min(1, right)));
      },
    });
    try {
      this.nes.loadROM(rom);
    } catch (cause) {
      const message = String((cause as Error)?.message ?? cause);
      const mapper = /mapper:?\s*(\d+)/i.exec(message)?.[1];
      // JSNES's own reason is data (English).
      throw mapper ? error('JSNES does not support mapper {} of this ROM.', [mapper]) : error('JSNES could not load this ROM: {}', [message]);
    }
  }
  frame() {
    this.samples = [];
    this.nes.frame();
    const audio = Float32Array.from(this.samples);
    if (audio.length === 0 || audio.some(value => !Number.isFinite(value)))
      throw this.error('The emulator produced invalid audio data.');
    return { video: this.video, audio };
  }
}
