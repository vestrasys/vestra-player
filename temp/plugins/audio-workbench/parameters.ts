// 实验探针：参数与接口可能与最终成品 API 相差很大。
export interface AudioParameters {
  bass: number;
  mid: number;
  treble: number;
  normalize: boolean;
  voice: boolean;
  delayMs: number;
}

export const neutral = (): AudioParameters => ({
  bass: 0,
  mid: 0,
  treble: 0,
  normalize: false,
  voice: false,
  delayMs: 0,
});

export function preset(index: number): AudioParameters {
  switch (index) {
    case 0:
      return neutral();
    case 1:
      return {
        ...neutral(),
        bass: -3,
        mid: 4,
        treble: 1,
        normalize: true,
        voice: true,
      };
    case 2:
      return { ...neutral(), bass: 5, mid: -1, treble: 3, normalize: true };
    default:
      throw new Error("未知音频预设");
  }
}

export function update(
  parameters: AudioParameters,
  action: string,
  value: number,
): AudioParameters {
  if (!Number.isFinite(value)) throw new Error("音频参数必须为有限数值");
  switch (action) {
    case "bass":
    case "mid":
    case "treble":
      return { ...parameters, [action]: Math.max(-12, Math.min(12, value)) };
    case "delay-ms":
      return {
        ...parameters,
        delayMs: Math.round(Math.max(-2000, Math.min(2000, value))),
      };
    case "normalize":
      return { ...parameters, normalize: value !== 0 };
    case "voice":
      return { ...parameters, voice: value !== 0 };
    default:
      throw new Error(`未知音频操作：${action}`);
  }
}
