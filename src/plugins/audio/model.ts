import type { AudioNode, AudioTrack } from "@vestra-player/sdk";

export const bands = [60, 170, 350, 1000, 3500, 10000] as const;
/** Crystalizer (a 3 kHz treble shelf): a switch plus a remembered strength. */
export type Clarity = { on: boolean; db: number };
/**
 * G05 channels: routing on a stereo mix; `stereo` leaves the track as it is.
 * `osd` is the message after a menu choice, `mixed` the one for a track with
 * more than two channels (mixed to stereo first).
 */
export const channelModes = [
  { id: "stereo", label: "Stereo", osd: "Channels: Stereo", mixed: "Channels: Stereo" },
  { id: "left", label: "Left Channel", osd: "Channels: Left Channel", mixed: "Channels: Left Channel (mixed to stereo)" },
  { id: "right", label: "Right Channel", osd: "Channels: Right Channel", mixed: "Channels: Right Channel (mixed to stereo)" },
  { id: "mono", label: "Mono Mix", osd: "Channels: Mono Mix", mixed: "Channels: Mono Mix (mixed to stereo)" },
  { id: "swap", label: "Swap Left and Right", osd: "Channels: Swap Left and Right", mixed: "Channels: Swap Left and Right (mixed to stereo)" },
] as const;
export type ChannelMode = (typeof channelModes)[number]["id"];
/** `equalizer: false` bypasses the bands but keeps their gains (Equalizer, Shift+E). */
export type Settings = {
  equalizer: boolean;
  gains: number[];
  delayMs: number;
  normalize: boolean;
  clarity: Clarity;
  channels: ChannelMode;
};
/** The window's preset list (ui/workbench.slint) has the same names in the same order. */
export const presets = [
  { label: "Flat", osd: "Equalizer preset: Flat", gains: [0, 0, 0, 0, 0, 0] },
  { label: "Voice", osd: "Equalizer preset: Voice", gains: [-3, -2, 1, 3, 2, -1] },
  { label: "Music", osd: "Equalizer preset: Music", gains: [3, 2, -1, 0, 2, 3] },
  { label: "Cinema", osd: "Equalizer preset: Cinema", gains: [4, 2, -1, 1, 2, 2] },
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

/** Audio Sync ▸ keep mode: reset per source, keep for this run, or keep across restarts. */
export const syncKeeps = [
  { id: "none", label: "Don't Keep" },
  { id: "session", label: "Keep for This Run" },
  { id: "always", label: "Always Keep" },
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

/** The delay in seconds with its sign, for the OSD template "Audio sync: {} s": "-0.05", "+0.3", "0". */
export const syncSeconds = (ms: number) => `${ms > 0 ? "+" : ms < 0 ? "-" : ""}${Math.abs(ms) / 1000}`;

/** True when the equalizer gains equal the preset's (menu check mark). */
export const matches = (settings: Settings, index: number) =>
  presets[index]!.gains.every((gain, i) => gain === settings.gains[i]);

/** Index of the preset the gains equal, or -1 for custom gains. */
export const presetOf = (settings: Settings) => presets.findIndex((_, index) => matches(settings, index));

/**
 * Neutral or bypassed bands and effects are left out, so neutral settings are an empty chain.
 * Order: channels → equalizer × 6 → treble (crystalizer) → dynaudnorm (normalize). A mono track keeps
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
  /** G16: the chosen output device id (`null` = System Default) and its last known name. */
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
 * A legacy numeric `clarity` (the old clarity slider) migrates to the crystalizer switch:
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

/** The JSON written back; `syncMs` only while Always Keep. */
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

/**
 * Fixed device titles. The menu titles are translated by the player from the
 * catalog; "System Default", "Unnamed device" and "Selected device" are also
 * texts of the window, so their translations are there.
 */
export const SYSTEM_DEFAULT = "System Default";
export const NO_DEVICES = "No Output Devices";
export const UNNAMED_DEVICE = "Unnamed device";
export const SELECTED_DEVICE = "Selected device";
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
/** A device name without control characters or edge spaces, ≤ 72 bytes; empty for a blank name. */
export const cleanName = (name: string) => fit(name.replace(/[\u0000-\u001f\u007f]/g, " ").trim(), 72);
/** A device name as a menu item: `cleanName`, never empty. */
export const deviceLabel = (name: string) => cleanName(name) || UNNAMED_DEVICE;
/**
 * Menu titles for `[System Default, ...devices, missing?]`: distinct (a repeated name gets " (2)"),
 * never one of the fixed titles. `missing` is the chosen device while it is not connected.
 * `name` is the `cleanName`, empty for an unnamed device; `n` is the number after it, or `null`.
 * `key` is the English title, which keeps them distinct.
 */
export function deviceTitles(devices: readonly { id: string; name: string }[], missing: string | null = null) {
  const used = new Set<string>([SYSTEM_DEFAULT, NO_DEVICES]);
  return [...devices.map((d) => d.name), ...(missing === null ? [] : [missing])].map((raw) => {
    const name = cleanName(raw);
    const base = deviceLabel(raw);
    let n: number | null = null;
    let key = base;
    for (let next = 2; used.has(key); next++) [n, key] = [next, `${base} (${next})`];
    used.add(key);
    return { name, n, key };
  });
}

// ---------------------------------------------------------------- tracks (G03, G04) and external audio (A11)

/**
 * A track as the window's list, the menu and the OSD show it. `text` is data,
 * "1. 日语 [jpn] (aac)" (at most 60 characters), or empty for a track without
 * title and language: it is then shown as the words "Track {n}" ("Track 2
 * (opus)"). An external track gets " (external)". main.ts puts the words into
 * text values (`trackItem` for the window's list, `trackLabel` for the menu and the OSD).
 */
export type TrackRow = { kind: "track"; text: string; n: number; codec: string; external: boolean; disconnected: false };
export function trackRow(track: AudioTrack, index: number): TrackRow {
  const n = index + 1;
  const title = track.title?.trim() || null;
  const language = track.language ? `[${track.language}]` : null;
  const codec = track.codec ? `(${track.codec})` : null;
  const text = title || language ? [`${n}.`, title, language, codec].filter(Boolean).join(" ") : "";
  return { kind: "track", text: clip(text, 60), n, codec: track.codec ?? "", external: track.external, disconnected: false };
}
/** At most `length` characters, cut with "…". */
export const clip = (text: string, length: number) => ([...text].length > length ? `${[...text].slice(0, length - 1).join("")}…` : text);

/** Audio file extensions A11 offers and the same-name scan picks up. */
export const AUDIO_EXTENSIONS = ["mka", "m4a", "aac", "ac3", "eac3", "dts", "flac", "mp3", "ogg", "opus", "wav", "mp2", "wma", "aiff", "aif", "thd", "mlp"];
export const AUTO_LOAD_LIMIT = 8;
const extension = (name: string) => (name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "");

/**
 * Video extensions, only to give an audio file to the video it names best:
 * `Movie.Extended.ac3` belongs to `Movie.Extended.mkv`, not to `Movie.mkv`, when both are in the folder.
 */
const VIDEO_EXTENSIONS = [
  "mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "flv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "ogv", "3gp", "rm", "rmvb", "asf", "divx",
];
/** File name without its last extension, compared case-insensitively and in NFC (macOS lists NFD names). */
const stemOf = (name: string) => (name.lastIndexOf(".") > 0 ? name.slice(0, name.lastIndexOf(".")) : name).normalize("NFC").toLowerCase();
/** `name`'s stem is `base`, or `base` + "." + a tag (`movie.jpn`, `movie.en.5ch`); not `movie.` (`movie..ac3`). */
const named = (base: string, name: string) => name === base || (name.startsWith(`${base}.`) && name.length > base.length + 1);

/**
 * Same-folder files whose name is the source's name without its extension, optionally followed by
 * "." and a tag, then an audio extension (`movie.mkv` → `movie.ac3`, `movie.jpn.mka`; case-insensitive,
 * NFC), sorted, at most 8. An audio source gets none.
 * A file that names another video of the folder more closely (a longer name) belongs to that one:
 * with `Movie.mkv` and `Movie.Extended.mkv` side by side, `Movie.Extended.ac3` goes with the second.
 */
export function companions(sourceName: string, names: readonly string[]) {
  const dot = sourceName.lastIndexOf(".");
  if (dot <= 0 || AUDIO_EXTENSIONS.includes(extension(sourceName))) return [];
  const stem = stemOf(sourceName);
  const others = names
    .filter((name) => VIDEO_EXTENSIONS.includes(extension(name)))
    .map(stemOf)
    .filter((other) => other.length > stem.length && named(stem, other));
  return names
    .filter((name) => name !== sourceName && AUDIO_EXTENSIONS.includes(extension(name)))
    .filter((name) => {
      const key = stemOf(name);
      return named(stem, key) && !others.some((other) => named(other, key));
    })
    .sort()
    .slice(0, AUTO_LOAD_LIMIT);
}
