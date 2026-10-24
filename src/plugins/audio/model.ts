import type { AudioNode, AudioTrack } from "@vestra-player/sdk";

export const bands = [60, 170, 350, 1000, 3500, 10000] as const;
/** 晶化 (crystalizer, a 3 kHz treble shelf): a switch plus a remembered strength. */
export type Clarity = { on: boolean; db: number };
/** G05 声道: routing on a stereo mix; `stereo` leaves the track as it is. */
export const channelModes = [
  { id: "stereo", label: "立体声" },
  { id: "left", label: "左声道" },
  { id: "right", label: "右声道" },
  { id: "mono", label: "单声道混合" },
  { id: "swap", label: "左右互换" },
] as const;
export type ChannelMode = (typeof channelModes)[number]["id"];
/** `equalizer: false` bypasses the bands but keeps their gains (均衡器, Shift+E). */
export type Settings = {
  equalizer: boolean;
  gains: number[];
  delayMs: number;
  normalize: boolean;
  clarity: Clarity;
  channels: ChannelMode;
};
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
  channels: "stereo",
});

/** 声音同步 ▶ keep mode: reset per source, keep for this run, or keep across restarts. */
export const syncKeeps = [
  { id: "none", label: "不保持" },
  { id: "session", label: "本次运行内保持" },
  { id: "always", label: "始终保持" },
] as const;
export type SyncKeep = (typeof syncKeeps)[number]["id"];
export const DELAY_LIMIT = 10000;
export const SYNC_STEP_MS = 50;

/** Rejects non-finite input (the edit is ignored), otherwise clamps. */
export function clamp(value: unknown, min: number, max: number) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("Expected finite numeric value");
  return Math.min(max, Math.max(min, value));
}
/** The host takes delays in 10 ms steps within ±10 000 ms. */
export const delay = (value: unknown) => Math.round(clamp(value, -DELAY_LIMIT, DELAY_LIMIT) / 10) * 10;
export const tenth = (value: unknown, min: number, max: number) => Math.round(clamp(value, min, max) * 10) / 10;

/** OSD wording: "音频同步: -0.05秒". */
export const syncText = (ms: number) => `音频同步: ${ms > 0 ? "+" : ms < 0 ? "-" : ""}${Math.abs(ms) / 1000}秒`;

/** True when the equalizer gains equal the preset's (menu check mark). */
export const matches = (settings: Settings, index: number) =>
  presets[index]!.gains.every((gain, i) => gain === settings.gains[i]);

/** Index of the preset the gains equal, or -1 for custom gains. */
export const presetOf = (settings: Settings) => presets.findIndex((_, index) => matches(settings, index));

/**
 * Neutral or bypassed bands and effects are left out, so neutral settings are an empty chain.
 * Order: channels → equalizer × 6 → treble (晶化) → dynaudnorm (规格化). A mono track keeps
 * the channel mode but gets no channels node.
 */
export function nodes(value: Settings, track: { mono: boolean } = { mono: false }): AudioNode[] {
  return [
    ...(value.channels === "stereo" || track.mono ? [] : [{ kind: "channels", mode: value.channels } as const]),
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
  /** A11: load same-named audio files next to a local source (not selected). */
  autoLoad: boolean;
  tab: number;
  /** G16: the chosen output device id (`null` = 系统默认) and its last known name. */
  device: string | null;
  deviceName: string | null;
  extra: Record<string, unknown>;
};
export const TABS = 3;
const KNOWN = new Set(["version", "equalizer", "gains", "normalize", "clarity", "channels", "autoLoadExternal", "syncKeep", "syncMs", "tab", "device", "deviceName"]);

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
  const channels = value.channels && typeof value.channels === "object" ? (value.channels as Record<string, unknown>).mode : undefined;
  const settings: Settings = {
    equalizer: value.equalizer !== false,
    gains,
    normalize: value.normalize === true,
    clarity,
    delayMs: syncKeep === "always" ? number(value.syncMs, 0, delay) : 0,
    channels: channelModes.find((m) => m.id === channels)?.id ?? "stereo",
  };
  const tab = typeof value.tab === "number" && Number.isInteger(value.tab) && value.tab >= 0 && value.tab < TABS ? value.tab : 0;
  const extra = Object.fromEntries(Object.entries(value).filter(([key]) => !KNOWN.has(key)));
  const device = deviceId(value.device);
  const deviceName = device !== null && typeof value.deviceName === "string" && value.deviceName.trim() ? value.deviceName.slice(0, 200) : null;
  return { settings, syncKeep, autoLoad: value.autoLoadExternal === true, tab, device, deviceName, extra };
}

/** The JSON written back; `syncMs` only while 始终保持. */
export function serialize({ settings, syncKeep, autoLoad, tab, device, deviceName, extra }: Stored) {
  return {
    ...extra,
    version: 1,
    equalizer: settings.equalizer,
    gains: settings.gains,
    normalize: settings.normalize,
    clarity: { ...settings.clarity },
    channels: { mode: settings.channels },
    autoLoadExternal: autoLoad,
    syncKeep,
    ...(syncKeep === "always" ? { syncMs: settings.delayMs } : {}),
    tab,
    device,
    ...(device !== null && deviceName ? { deviceName } : {}),
  };
}

// ---------------------------------------------------------------- output device (G16)

export const SYSTEM_DEFAULT = "系统默认";
export const NO_DEVICES = "没有可用的输出设备";
export const DEVICE_POLL_MS = 5000;
/** A saved id the player would accept: 1–512 characters, no control characters. */
export function deviceId(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && value.length <= 512 && !/[\u0000-\u001f\u007f]/.test(value) ? value : null;
}
const utf8 = new TextEncoder();
/** At most `bytes` UTF-8 bytes, cut on a character with "…". */
function fit(text: string, bytes: number) {
  if (utf8.encode(text).length <= bytes) return text;
  let out = "";
  for (const char of text) {
    if (utf8.encode(`${out}${char}…`).length > bytes) break;
    out += char;
  }
  return `${out}…`;
}
/** A device name as a menu item: no control characters or edge spaces, ≤ 72 bytes, never empty. */
export const deviceLabel = (name: string) => fit(name.replace(/[\u0000-\u001f\u007f]/g, " ").trim(), 72) || "未命名设备";
/**
 * Menu titles for `[系统默认, ...devices, missing?]`: distinct (a repeated name gets " (2)"),
 * never one of the fixed titles. `missing` is the chosen device while it is not connected.
 */
export function deviceTitles(devices: readonly { id: string; name: string }[], missing: string | null = null) {
  const used = new Set([SYSTEM_DEFAULT, NO_DEVICES]);
  return [...devices.map((d) => d.name), ...(missing === null ? [] : [missing])].map((name) => {
    const base = deviceLabel(name);
    let title = base;
    for (let n = 2; used.has(title); n++) title = `${base} (${n})`;
    used.add(title);
    return title;
  });
}

// ---------------------------------------------------------------- tracks (G03, G04) and external audio (A11)

/** "1. 日语 [jpn] (aac)", "3. movie.ac3 (ac3)（外部）", "音轨 2 (opus)"; at most 60 characters. */
export function trackLabel(track: AudioTrack, index: number) {
  const n = index + 1;
  const title = track.title?.trim() || null;
  const language = track.language ? `[${track.language}]` : null;
  const codec = track.codec ? `(${track.codec})` : null;
  const head = title || language ? [`${n}.`, title, language] : [`音轨 ${n}`];
  const text = [...head, codec].filter(Boolean).join(" ") + (track.external ? "（外部）" : "");
  return [...text].length > 60 ? `${[...text].slice(0, 59).join("")}…` : text;
}

/** Audio file extensions A11 offers and the same-name scan picks up. */
export const AUDIO_EXTENSIONS = ["mka", "m4a", "aac", "ac3", "eac3", "dts", "flac", "mp3", "ogg", "opus", "wav", "mp2", "wma", "aiff", "aif", "thd", "mlp"];
export const AUTO_LOAD_LIMIT = 8;
const extension = (name: string) => (name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "");

/**
 * Same-folder files that start with the source's name and have an audio extension
 * (`movie.mkv` → `movie.ac3`, `movie.jpn.mka`), sorted, at most 8. An audio source gets none.
 */
export function companions(sourceName: string, names: readonly string[]) {
  const dot = sourceName.lastIndexOf(".");
  if (dot <= 0 || AUDIO_EXTENSIONS.includes(extension(sourceName))) return [];
  const stem = sourceName.slice(0, dot).toLowerCase();
  return names
    .filter((name) => name !== sourceName && name.toLowerCase().startsWith(`${stem}.`) && AUDIO_EXTENSIONS.includes(extension(name)))
    .sort()
    .slice(0, AUTO_LOAD_LIMIT);
}
