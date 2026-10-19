// 实验探针：FFmpeg 后端表达式不是最终产品 API，可能发生重大变化。
import type { AudioParameters } from "./parameters.ts";

/** Only numeric plugin parameters become filter arguments; never interpolate external text. */
export function audioFilter(parameters: AudioParameters): string {
  const gains = [parameters.bass, parameters.mid, parameters.treble];
  if (
    gains.some((gain) => !Number.isFinite(gain) || gain < -12 || gain > 12) ||
    !Number.isFinite(parameters.delayMs) ||
    Math.abs(parameters.delayMs) > 2000
  ) {
    throw new Error("音频参数越界");
  }
  const frequencies = [100, 1000, 10000];
  const filters = gains.map(
    (gain, index) =>
      `equalizer=f=${frequencies[index]}:t=q:w=1:g=${gain.toFixed(3)}`,
  );
  if (parameters.voice)
    filters.push(
      "highpass=f=120",
      "equalizer=f=2500:t=q:w=1:g=4",
      "lowpass=f=8000",
    );
  if (parameters.normalize) filters.push("loudnorm=I=-16:TP=-1.5:LRA=11");
  if (parameters.delayMs > 0)
    filters.push(`adelay=${Math.round(parameters.delayMs)}:all=1`);
  if (parameters.delayMs < 0)
    filters.push(
      `atrim=start=${(-parameters.delayMs / 1000).toFixed(3)}`,
      "asetpts=PTS-STARTPTS",
    );
  return filters.join(",");
}
