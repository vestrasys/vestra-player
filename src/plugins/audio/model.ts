import type { AudioNode } from "../../sdk/plugin/index.ts";

export const bands = [60, 170, 350, 1000, 3500, 10000] as const;
/** 晶化 (reference player crystalizer, our 3 kHz treble shelf): a switch plus a remembered strength. */
export type Clarity = { on: boolean; db: number };
/** `equalizer: false` bypasses the bands but keeps their gains (参考播放器均衡器 Shift+E). */
export type Settings = { equalizer: boolean; gains: number[]; delayMs: number; normalize: boolean; clarity: Clarity };
export const presets = [
  { label: "平直", gains: [0, 0, 0, 0, 0, 0] },
  { label: "人声", gains: [-3, -2, 1, 3, 2, -1] },
  { label: "音乐", gains: [3, 2, -1, 0, 2, 3] },
  { label: "影院", gains: [4, 2, -1, 1, 2, 2] },
] as const;
export const CLARITY_DB = 3;
export const neutralClarity = (): Clarity => ({ on: false, db: CLARITY_DB });
export const neutral = (): Settings => ({
  equalizer: true,
  gains: bands.map(() => 0),
  delayMs: 0,
  normalize: false,
  clarity: neutralClarity(),
});

/** 声音同步 ▶ keep mode: reset per source, keep for this run, or keep across restarts. */
export const syncKeeps = [
  { id: "none", label: "不保持" },
  { id: "session", label: "本次运行内保持" },
  { id: "always", label: "始终保持" },
] as const;
export type SyncKeep = (typeof syncKeeps)[number]["id"];
export const DELAY_LIMIT = 2000;
export const SYNC_STEP_MS = 50;

/** Rejects non-finite input (the edit is ignored), otherwise clamps. */
export function clamp(value: unknown, min: number, max: number) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("Expected finite numeric value");
  return Math.min(max, Math.max(min, value));
}
/** The host takes delays in 10 ms steps within ±2000 ms. */
export const delay = (value: unknown) => Math.round(clamp(value, -DELAY_LIMIT, DELAY_LIMIT) / 10) * 10;
export const tenth = (value: unknown, min: number, max: number) => Math.round(clamp(value, min, max) * 10) / 10;

/** OSD wording (reference player strings 1724「音频同步：%g秒」): "音频同步: -0.05秒". */
export const syncText = (ms: number) => `音频同步: ${ms > 0 ? "+" : ms < 0 ? "-" : ""}${Math.abs(ms) / 1000}秒`;

/** True when the equalizer gains equal the preset's (menu check mark). */
export const matches = (settings: Settings, index: number) =>
  presets[index]!.gains.every((gain, i) => gain === settings.gains[i]);

/** Index of the preset the gains equal, or -1 for custom gains. */
export const presetOf = (settings: Settings) => presets.findIndex((_, index) => matches(settings, index));

/** Neutral or bypassed bands and effects are left out, so neutral settings are an empty chain. */
export function nodes(value: Settings): AudioNode[] {
  return [
    ...bands.flatMap((frequency, i): AudioNode[] =>
      !value.equalizer || value.gains[i] === 0 ? [] : [{ kind: "equalizer", frequency, q: 1, gainDb: clamp(value.gains[i], -6, 6) }],
    ),
    ...(!value.clarity.on || value.clarity.db === 0
      ? []
      : [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: clamp(value.clarity.db, 0, 6) } as const]),
    ...(value.normalize ? [{ kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 } as const] : []),
  ];
}

/** What `settings.json` holds (version 1). Fields this version does not know are kept as they are. */
export type Stored = {
  settings: Settings;
  syncKeep: SyncKeep;
  tab: number;
  extra: Record<string, unknown>;
};
export const TABS = 3;
const KNOWN = new Set(["version", "equalizer", "gains", "normalize", "clarity", "syncKeep", "syncMs", "tab"]);

const number = (value: unknown, fallback: number, read: (value: unknown) => number) => {
  try {
    return read(value);
  } catch {
    return fallback;
  }
};

/**
 * Parse `settings.json` leniently: any broken field falls back to its default.
 * A legacy numeric `clarity` (the old 清晰度 slider) migrates to the 晶化 switch:
 * above 0 is on at that strength, 0 is off at the default strength.
 */
export function parse(raw: unknown): Stored {
  const value = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const base = neutral();
  const gains = Array.isArray(value.gains) && value.gains.length === bands.length
    ? value.gains.map((gain) => number(gain, 0, (g) => tenth(g, -6, 6)))
    : base.gains;
  let clarity = neutralClarity();
  if (typeof value.clarity === "number") {
    const db = number(value.clarity, 0, (v) => tenth(v, 0, 6));
    clarity = db > 0 ? { on: true, db } : neutralClarity();
  } else if (value.clarity && typeof value.clarity === "object") {
    const c = value.clarity as Record<string, unknown>;
    clarity = { on: c.on === true, db: number(c.db, CLARITY_DB, (v) => tenth(v, 0, 6)) };
  }
  const syncKeep = syncKeeps.find((k) => k.id === value.syncKeep)?.id ?? "none";
  const settings: Settings = {
    equalizer: value.equalizer !== false,
    gains,
    normalize: value.normalize === true,
    clarity,
    delayMs: syncKeep === "always" ? number(value.syncMs, 0, delay) : 0,
  };
  const tab = typeof value.tab === "number" && Number.isInteger(value.tab) && value.tab >= 0 && value.tab < TABS ? value.tab : 0;
  const extra = Object.fromEntries(Object.entries(value).filter(([key]) => !KNOWN.has(key)));
  return { settings, syncKeep, tab, extra };
}

/** The JSON written back; `syncMs` only while 始终保持. */
export function serialize({ settings, syncKeep, tab, extra }: Stored) {
  return {
    ...extra,
    version: 1,
    equalizer: settings.equalizer,
    gains: settings.gains,
    normalize: settings.normalize,
    clarity: { ...settings.clarity },
    syncKeep,
    ...(syncKeep === "always" ? { syncMs: settings.delayMs } : {}),
    tab,
  };
}
