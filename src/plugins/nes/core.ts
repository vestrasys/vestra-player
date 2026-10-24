import { NES } from 'jsnes';

/** JSNES returns 0x00BBGGRR pixels; the public producer uses byte-order RGBA. */
export function rgbaFrame(pixels: Uint32Array): Uint8Array {
  if (pixels.length !== 256 * 240) throw new Error('模拟器画面尺寸无效。');
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
  constructor(rom: Uint8Array) {
    if (rom.length < 16 || rom[0] !== 78 || rom[1] !== 69 || rom[2] !== 83 || rom[3] !== 26)
      throw new Error('这不是 iNES 格式的 NES ROM 文件。');
    // Header: PRG banks × 16 KiB, CHR banks × 8 KiB, an optional 512-byte trainer.
    if (rom.length < 16 + (rom[6]! & 4 ? 512 : 0) + rom[4]! * 16384 + rom[5]! * 8192)
      throw new Error('ROM 文件不完整：比文件头声明的短。');
    this.nes = new NES({ sampleRate: 48000, emulateSound: true,
      onFrame: pixels => { this.video = rgbaFrame(pixels); },
      onAudioSample: (left, right) => {
        this.samples.push(Math.max(-1, Math.min(1, left)), Math.max(-1, Math.min(1, right)));
      },
    });
    try {
      this.nes.loadROM(rom);
    } catch (error) {
      const mapper = /mapper:?\s*(\d+)/i.exec(String((error as Error)?.message))?.[1];
      throw new Error(mapper ? `JSNES 不支持这个 ROM 的 mapper ${mapper}。` : `JSNES 无法载入这个 ROM：${(error as Error)?.message ?? error}`);
    }
  }
  frame() {
    this.samples = [];
    this.nes.frame();
    const audio = Float32Array.from(this.samples);
    if (audio.length === 0 || audio.some(value => !Number.isFinite(value)))
      throw new Error('模拟器音频数据无效。');
    return { video: this.video, audio };
  }
}
