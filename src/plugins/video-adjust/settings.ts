// 实验参数模型；字段、范围和 API 与最终产品可能相差很大。
export interface Adjustments {
  brightness: number;
  contrast: number;
  saturation: number;
  hue: number;
}

export type AdjustmentName = keyof Adjustments;
export type PresetName = "neutral" | "bright" | "cinema";

const ranges: Record<AdjustmentName, readonly [number, number]> = {
  brightness: [-1, 1],
  contrast: [0, 2],
  saturation: [0, 3],
  hue: [-180, 180],
};

export function preset(name: PresetName): Adjustments {
  switch (name) {
    case "neutral":
      return { brightness: 0, contrast: 1, saturation: 1, hue: 0 };
    case "bright":
      return { brightness: 0.12, contrast: 1.08, saturation: 1.05, hue: 0 };
    case "cinema":
      return { brightness: -0.04, contrast: 1.18, saturation: 0.85, hue: -3 };
  }
}

export function change(
  settings: Adjustments,
  name: AdjustmentName,
  value: number,
): Adjustments {
  const [minimum, maximum] = ranges[name];
  if (!Number.isFinite(value) || value < minimum || value > maximum) {
    throw new RangeError(
      `${name} 必须在 ${minimum} 到 ${maximum} 之间，收到 ${value}`,
    );
  }
  return { ...settings, [name]: value };
}

export function filter(settings: Adjustments): string {
  // 有限数字已校验，不能经字符串参数注入其他 FFmpeg filter。
  for (const name of Object.keys(ranges) as AdjustmentName[])
    change(settings, name, settings[name]);
  return `eq=brightness=${settings.brightness}:contrast=${settings.contrast}:saturation=${settings.saturation},hue=h=${settings.hue}`;
}

export function isNeutral(settings: Adjustments): boolean {
  return (
    settings.brightness === 0 &&
    settings.contrast === 1 &&
    settings.saturation === 1 &&
    settings.hue === 0
  );
}
