// 实验音频证据工具：独立生成 PCM 和测量结果，不代表最终 API。
import { writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";

export const sampleRate = 48000;

export async function writeWave(
  path: string,
  samples: Float32Array,
): Promise<void> {
  const bytes = Buffer.alloc(44 + samples.length * 2);
  bytes.write("RIFF");
  bytes.writeUInt32LE(bytes.length - 8, 4);
  bytes.write("WAVE", 8);
  bytes.write("fmt ", 12);
  bytes.writeUInt32LE(16, 16);
  bytes.writeUInt16LE(1, 20);
  bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(sampleRate, 24);
  bytes.writeUInt32LE(sampleRate * 2, 28);
  bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34);
  bytes.write("data", 36);
  bytes.writeUInt32LE(samples.length * 2, 40);
  samples.forEach((sample, index) =>
    bytes.writeInt16LE(
      Math.round(Math.max(-1, Math.min(1, sample)) * 32767),
      44 + index * 2,
    ),
  );
  await writeFile(path, bytes);
}

export function decode(path: string): Float32Array {
  const result = spawnSync(
    "ffmpeg",
    [
      "-v",
      "error",
      "-i",
      path,
      "-ar",
      String(sampleRate),
      "-ac",
      "1",
      "-f",
      "f32le",
      "pipe:1",
    ],
    { maxBuffer: 16 * 1024 * 1024 },
  );
  if (result.status !== 0) throw new Error(result.stderr.toString());
  const samples = new Float32Array(result.stdout.length / 4);
  samples.forEach((_, index) => {
    samples[index] = result.stdout.readFloatLE(index * 4);
  });
  return samples;
}

export function tones(
  frequencies: readonly number[],
  amplitude = 0.015,
  seconds = 4,
): Float32Array {
  return Float32Array.from({ length: sampleRate * seconds }, (_, index) =>
    frequencies.reduce(
      (sum, frequency) =>
        sum +
        amplitude * Math.sin((2 * Math.PI * frequency * index) / sampleRate),
      0,
    ),
  );
}

export function magnitude(samples: Float32Array, frequency: number): number {
  const from = sampleRate;
  const to = Math.min(samples.length, sampleRate * 3);
  let real = 0;
  let imaginary = 0;
  for (let index = from; index < to; index++) {
    const phase = (2 * Math.PI * frequency * index) / sampleRate;
    real += (samples[index] ?? 0) * Math.cos(phase);
    imaginary += (samples[index] ?? 0) * Math.sin(phase);
  }
  return (2 * Math.hypot(real, imaginary)) / (to - from);
}

export function rms(samples: Float32Array): number {
  const end = Math.min(samples.length, sampleRate * 3);
  let squareSum = 0;
  for (let index = sampleRate; index < end; index++)
    squareSum += (samples[index] ?? 0) ** 2;
  return Math.sqrt(squareSum / (end - sampleRate));
}

export function impulse(): Float32Array {
  const samples = new Float32Array(sampleRate * 3);
  // A short positive pulse allows delay measurement without phase sensitivity.
  samples.fill(0.5, sampleRate, sampleRate + 48);
  return samples;
}

export function firstSignalSeconds(samples: Float32Array): number {
  const index = samples.findIndex((value) => Math.abs(value) > 0.1);
  if (index < 0) throw new Error("输出音频没有预期脉冲");
  return index / sampleRate;
}
