import type { SubtitleStyle } from "@vestra-player/sdk";
import type { Cue } from "./formats.ts";
import { DEFAULT_ENCODING, isEncoding } from "./formats.ts";
import type { Encoding } from "./formats.ts";
import { status } from "./messages.ts";
import type { Status } from "./messages.ts";

/** Sync step choices in ms (Subtitle Settings ▸ Sync step); 0.5 s is the default. */
export const SYNC_STEPS = [50, 100, 250, 500, 1000, 5000] as const;
export const DEFAULT_SYNC_STEP = 500;
/** The host's accepted `offsetMs` range (reference §9). */
export const SYNC_LIMIT = 600000;

/**
 * The Subtitles ▸ Sync menu titles of each step (the step is in the title,
 * so each is a whole text); same order as `SYNC_STEPS`.
 */
export const SYNC_MENU = [
  { step: 50, earlier: "Earlier 0.05 s", later: "Later 0.05 s" },
  { step: 100, earlier: "Earlier 0.1 s", later: "Later 0.1 s" },
  { step: 250, earlier: "Earlier 0.25 s", later: "Later 0.25 s" },
  { step: 500, earlier: "Earlier 0.5 s", later: "Later 0.5 s" },
  { step: 1000, earlier: "Earlier 1 s", later: "Later 1 s" },
  { step: 5000, earlier: "Earlier 5 s", later: "Later 5 s" },
] as const;

/** A08: where the secondary subtitle sits (`SubtitleStyle.secondaryPosition`), with its menu title. */
export type SecondaryPosition = "top" | "aboveMain";
export const SECONDARY_POSITIONS = [
  { id: "top", label: "Top" },
  { id: "aboveMain", label: "Above Main Subtitle" },
] as const;
export type Stored = {
  syncStepMs: number;
  autoReload: boolean;
  fallbackEncoding: Encoding;
  secondaryPosition: SecondaryPosition;
  extra: Record<string, unknown>;
};
const KNOWN = new Set(["version", "syncStepMs", "autoReload", "fallbackEncoding", "secondaryPosition"]);
export const isObject = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);

/** `settings.json` (version 1), read leniently: a broken field falls back to its default. */
export function parse(raw: unknown): Stored {
  const value = isObject(raw) ? raw : {};
  const syncStepMs = SYNC_STEPS.find((step) => step === value.syncStepMs) ?? DEFAULT_SYNC_STEP;
  const extra = Object.fromEntries(Object.entries(value).filter(([key]) => !KNOWN.has(key)));
  const fallbackEncoding = isEncoding(value.fallbackEncoding) ? value.fallbackEncoding : DEFAULT_ENCODING;
  const secondaryPosition: SecondaryPosition = value.secondaryPosition === "aboveMain" ? "aboveMain" : "top";
  return { syncStepMs, autoReload: value.autoReload === true, fallbackEncoding, secondaryPosition, extra };
}
export const serialize = ({ syncStepMs, autoReload, fallbackEncoding, secondaryPosition, extra }: Stored) => ({
  ...extra,
  version: 1,
  syncStepMs,
  autoReload,
  fallbackEncoding,
  secondaryPosition,
});

/** Seconds of a step: 0.05, 0.5, 5. */
export const stepSeconds = (ms: number) => `${ms / 1000}`;

/**
 * Adjust Sync input: whole milliseconds with an optional sign (ASCII or U+2212).
 * Returns the value, or the status saying why it was refused.
 */
export function parseSync(text: string): { value: number } | { error: Status } {
  const trimmed = String(text).trim().replace(/^−/, "-");
  if (!/^[+-]?\d{1,9}$/.test(trimmed)) return { error: status("syncInteger") };
  const value = Number(trimmed);
  if (Math.abs(value) > SYNC_LIMIT) return { error: status("syncRange", [SYNC_LIMIT, SYNC_LIMIT]) };
  return { value };
}

// ------------------------------------------------------------------ style.json (D09)
/** A remembered style: every field but `offsetMs` (the sync is not kept) and the secondary position (settings.json). */
export type SavedStyle = Omit<SubtitleStyle, "offsetMs" | "secondaryPosition">;
export type StyleFile = { style: SavedStyle | null; shadowLast: number; boxOpacity: number };
export const DEFAULT_SHADOW = 2;
export const DEFAULT_BOX = 50;
const COLORS = ["white", "yellow", "cyan"];
const int = (value: unknown, min: number, max: number, fallback: number) =>
  typeof value === "number" && Number.isInteger(value) && value >= min && value <= max ? value : fallback;

/** `style.json`, read leniently: an out-of-range field takes its default. */
export function parseStyle(raw: unknown, base: SavedStyle): StyleFile {
  const value = isObject(raw) ? raw : {};
  const s = isObject(value.style) ? value.style : null;
  const style: SavedStyle | null = s && {
    fontId: typeof s.fontId === "string" && s.fontId ? s.fontId : base.fontId,
    fontSize: int(s.fontSize, 16, 72, base.fontSize),
    color: COLORS.includes(s.color as string) ? (s.color as SavedStyle["color"]) : base.color,
    outline: int(s.outline, 0, 4, base.outline),
    marginPercent: int(s.marginPercent, 2, 15, base.marginPercent),
    bold: s.bold === true,
    shadow: int(s.shadow, 0, 4, 0),
    box: s.box === null || s.box === undefined ? null : int(s.box, 0, 100, DEFAULT_BOX),
    assOverride: s.assOverride === "force" ? "force" : "keep",
  };
  return {
    style,
    shadowLast: int(value.shadowLast, 1, 4, DEFAULT_SHADOW),
    boxOpacity: int(value.boxOpacity, 0, 100, DEFAULT_BOX),
  };
}
export const saveable = (style: SubtitleStyle): SavedStyle => {
  const { offsetMs: _sync, secondaryPosition: _position, ...rest } = style;
  return { ...rest, bold: rest.bold === true, shadow: rest.shadow ?? 0, box: rest.box ?? null, assOverride: rest.assOverride ?? "keep" };
};

// ------------------------------------------------------------------ B11 previous / next dialogue
export type Jump = "previous" | "next" | "current";
/**
 * The cue to jump to from `position` (seconds), with cue times shifted by the
 * sync offset (positive = later). Previous: the last cue starting more than
 * 0.5 s before; next: the first starting more than 0.05 s after; current: the
 * cue being shown, else the last one that ended. Returns its displayed start.
 */
export function dialogueTarget(cues: Cue[], position: number, offsetMs: number, jump: Jump): number | null {
  const shift = offsetMs / 1000;
  const shown = cues.map((c) => ({ start: c.start + shift, end: c.end + shift })).sort((a, b) => a.start - b.start);
  if (jump === "next") return shown.find((c) => c.start > position + 0.05)?.start ?? null;
  if (jump === "previous") return shown.filter((c) => c.start < position - 0.5).at(-1)?.start ?? null;
  const inside = shown.filter((c) => c.start <= position && position < c.end).at(-1);
  return (inside ?? shown.filter((c) => c.end <= position).at(-1))?.start ?? null;
}
/** `00:12:03.450` */
export function clock(seconds: number) {
  const ms = Math.max(0, Math.round(seconds * 1000));
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(Math.floor(ms / 3600000))}:${pad(Math.floor(ms / 60000) % 60)}:${pad(Math.floor(ms / 1000) % 60)}.${pad(ms % 1000, 3)}`;
}
