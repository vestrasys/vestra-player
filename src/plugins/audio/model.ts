import type { AudioNode, AudioTrack } from "@vestra-player/sdk";

/** The ten equalizer bands (ISO octave centres, as the panel labels them). */
export const bands = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000] as const;
/** The six bands of settings.json up to 1.5.10 (version 1), read once and spread over the ten (`spread`). */
export const OLD_BANDS = [60, 170, 350, 1000, 3500, 10000] as const;
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
/**
 * The panel's preset list (ui/workbench.slint) has the same names in the same order. The gains are
 * the six-band presets of 1.5.10 spread over the ten bands (`spread`), so a saved preset stays that preset.
 */
export const presets = [
  { label: "Flat", osd: "Equalizer preset: Flat", gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { label: "Voice", osd: "Equalizer preset: Voice", gains: [-3, -3, -2.3, -0.4, 1.7, 3, 2.4, 1.6, -0.4, -1] },
  { label: "Music", osd: "Equalizer preset: Music", gains: [3, 3, 2.3, 0.4, -0.7, 0, 1.1, 2.1, 2.8, 3] },
  { label: "Cinema", osd: "Equalizer preset: Cinema", gains: [4, 3.9, 2.6, 0.4, -0.3, 1, 1.6, 2, 2, 2] },
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
/** Reset ▸ Audio Processing: the equalizer, Normalize and Crystalizer back to neutral; sync and channels kept. */
export const neutralProcessing = (value: Settings): Settings => {
  const { equalizer, gains, normalize, clarity } = neutral();
  return { ...value, equalizer, gains, normalize, clarity };
};

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
 * Order: channels → equalizer × 10 → treble (crystalizer) → dynaudnorm (normalize). A mono track keeps
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

/**
 * What `settings.json` holds. Version 2 keeps the ten equalizer gains in `gains10` and still writes
 * `gains` as the six-band curve of version 1 (`narrow`), so an older plugin (up to 1.5.10, which
 * reads only `gains`) plays a close curve instead of a flat one. Fields this version does not know
 * are kept as they are.
 */
export type Stored = {
  settings: Settings;
  syncKeep: SyncKeep;
  /** A11: load same-named audio files next to a local source (not selected). */
  autoLoad: boolean;
  /** G16: the chosen output device id (`null` = System Default) and its last known name. */
  device: string | null;
  deviceName: string | null;
  extra: Record<string, unknown>;
};
// "tab" was the Audio Workbench window's page (up to 1.5.10): read no more, and dropped at the next save.
const KNOWN = new Set(["version", "equalizer", "gains", "gains10", "normalize", "clarity", "channels", "autoLoadExternal", "syncKeep", "syncMs", "tab", "device", "deviceName"]);

/**
 * Six gains of version 1 (60 Hz–10 kHz) spread over the ten bands: each band takes the gain the old
 * curve had at its frequency, interpolated on a logarithmic frequency scale, the ends held, in 0.1 dB.
 */
export function spread(old: readonly number[]): number[] {
  const at = OLD_BANDS.map(Math.log2);
  return bands.map((frequency) => {
    const x = Math.log2(frequency);
    if (x <= at[0]!) return old[0]!;
    if (x >= at.at(-1)!) return old.at(-1)!;
    const i = at.findIndex((v, k) => x >= v && x <= at[k + 1]!);
    const t = (x - at[i]!) / (at[i + 1]! - at[i]!);
    return Math.round((old[i]! + (old[i + 1]! - old[i]!) * t) * 10) / 10;
  });
}

/**
 * The ten gains as six of version 1 (`spread`'s inverse): each old band takes the ten-band curve's
 * value at its frequency, interpolated on a logarithmic frequency scale, in 0.1 dB.
 */
export function narrow(gains: readonly number[]): number[] {
  const at = bands.map(Math.log2);
  return OLD_BANDS.map((frequency) => {
    const x = Math.log2(frequency);
    if (x <= at[0]!) return gains[0]!;
    if (x >= at.at(-1)!) return gains.at(-1)!;
    const i = at.findIndex((v, k) => x >= v && x <= at[k + 1]!);
    const t = (x - at[i]!) / (at[i + 1]! - at[i]!);
    return Math.round((gains[i]! + (gains[i + 1]! - gains[i]!) * t) * 10) / 10;
  });
}

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
 * above 0 is on at that strength, 0 is off at the default strength. Six gains (version 1) are spread
 * over the ten bands.
 */
export function parse(raw: unknown): Stored {
  const value = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const base = neutral();
  const read = (list: unknown[]) => list.map((gain) => number(gain, 0, (g) => tenth(g, -6, 6)));
  const six = Array.isArray(value.gains) && value.gains.length === OLD_BANDS.length ? read(value.gains) : null;
  const ten = Array.isArray(value.gains10) && value.gains10.length === bands.length ? read(value.gains10) : null;
  // `gains10` while `gains` is still its six-band curve; an older plugin that changed the curve
  // since wrote `gains` only (and kept `gains10` as it was), so its curve wins, spread.
  const sameCurve = ten !== null && (six === null || narrow(ten).every((gain, i) => gain === six[i]));
  const gains = sameCurve ? ten! : six !== null ? spread(six) : base.gains;
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
  const extra = Object.fromEntries(Object.entries(value).filter(([key]) => !KNOWN.has(key)));
  const device = deviceId(value.device);
  const deviceName = device !== null && typeof value.deviceName === "string" && value.deviceName.trim() ? value.deviceName.slice(0, 200) : null;
  return { settings, syncKeep, autoLoad: value.autoLoadExternal === true, device, deviceName, extra };
}

/** The JSON written back; `syncMs` only while Always Keep. */
export function serialize({ settings, syncKeep, autoLoad, device, deviceName, extra }: Stored) {
  return {
    ...extra,
    version: 2,
    equalizer: settings.equalizer,
    gains: narrow(settings.gains),
    gains10: settings.gains,
    normalize: settings.normalize,
    clarity: { ...settings.clarity },
    channels: { mode: settings.channels },
    autoLoadExternal: autoLoad,
    syncKeep,
    ...(syncKeep === "always" ? { syncMs: settings.delayMs } : {}),
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
const utf8 = new TextEncoder();
const bytes = (text: string) => utf8.encode(text).length;
/** A saved id the player would accept: 1–512 UTF-8 bytes, no control characters (C0, DEL, C1), as the player checks it. */
export function deviceId(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && bytes(value) <= 512 && !/\p{Cc}/u.test(value) ? value : null;
}
/** User-perceived characters; code points where the runtime has no `Intl.Segmenter`. */
const graphemes: (text: string) => Iterable<string> =
  typeof Intl.Segmenter === "function"
    ? ((segmenter) => (text) => Array.from(segmenter.segment(text), ({ segment }) => segment))(new Intl.Segmenter(undefined, { granularity: "grapheme" }))
    : (text) => text;
/** The player's limit for one menu path segment, in UTF-8 bytes. */
export const SEGMENT_BYTES = 80;
/**
 * Data made fit for a menu line: the player refuses control characters (C0, DEL and C1) and
 * surrounding whitespace in a menu path segment, so a control becomes a space and the edges
 * are trimmed.
 */
export const menuText = (text: string) => text.replace(/\p{Cc}/gu, " ").replace(/^\s+|\s+$/gu, "");
/**
 * At most `max` UTF-8 bytes, cut with "…" on whole user-perceived characters: a code point,
 * surrogate pair, emoji sequence or accented letter is kept or dropped as one. Spaces before
 * the "…" are dropped.
 */
function fit(text: string, max: number) {
  if (bytes(text) <= max) return text;
  let out = "";
  let room = max - bytes("…");
  for (const segment of graphemes(text)) {
    const size = bytes(segment);
    if (size > room) break;
    out += segment;
    room -= size;
  }
  return `${menuText(out)}…`;
}
/** A device name without control characters or edge spaces, ≤ 72 bytes; empty for a blank name. */
export const cleanName = (name: string) => fit(menuText(name), 72);
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
 * "1. 日语 [jpn] (aac)", or empty for a track without title and language: it is
 * then shown as the words "Track {n}" ("Track 2 (opus)"). An external track gets
 * " (external)". main.ts puts the words into text values (`trackItem` for the
 * window's list, `trackLabel` for the menu and the OSD).
 *
 * The menu label is a menu path segment, which the player limits to 80 UTF-8
 * bytes without controls or edge whitespace. Title, language and codec lose
 * their controls and edge spaces; language and codec are cut to 16 and 24 bytes,
 * then the title to what is left of `TRACK_TEXT_BYTES`, so the number, language
 * and codec stay visible. The number in front keeps the labels of a list distinct.
 */
export type TrackRow = { kind: "track"; text: string; n: number; codec: string; external: boolean; disconnected: false };
/** Room for `text`: 80 bytes less the widest " (external)" of the plugin's languages ("（外部）", 12 bytes). */
export const TRACK_TEXT_BYTES = SEGMENT_BYTES - 12;
export function trackRow(track: AudioTrack, index: number): TrackRow {
  const n = index + 1;
  const codec = fit(menuText(track.codec ?? ""), 24);
  const language = fit(menuText(track.language ?? ""), 16);
  const tail = [language && `[${language}]`, codec && `(${codec})`].filter(Boolean).join(" ");
  // The title gets what the number, the tail and the spaces between leave.
  const title = fit(menuText(track.title ?? ""), TRACK_TEXT_BYTES - bytes([`${n}.`, tail].filter(Boolean).join(" ")) - 1);
  const text = title || language ? fit([`${n}.`, title, tail].filter(Boolean).join(" "), TRACK_TEXT_BYTES) : "";
  return { kind: "track", text, n, codec, external: track.external, disconnected: false };
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
