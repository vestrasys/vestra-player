/** 同步单位 choices in ms (reference player dialogs 442「同步调整单位」); 0.5 s is the default. */
export const SYNC_STEPS = [50, 100, 250, 500, 1000, 5000] as const;
export const DEFAULT_SYNC_STEP = 500;
/** The host's accepted `offsetMs` range (reference §9). */
export const SYNC_LIMIT = 10000;

export type Stored = { syncStepMs: number; autoReload: boolean; extra: Record<string, unknown> };
const KNOWN = new Set(["version", "syncStepMs", "autoReload"]);

/** `settings.json` (version 1), read leniently: a broken field falls back to its default. */
export function parse(raw: unknown): Stored {
  const value = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const syncStepMs = SYNC_STEPS.find((step) => step === value.syncStepMs) ?? DEFAULT_SYNC_STEP;
  const extra = Object.fromEntries(Object.entries(value).filter(([key]) => !KNOWN.has(key)));
  return { syncStepMs, autoReload: value.autoReload === true, extra };
}
export const serialize = ({ syncStepMs, autoReload, extra }: Stored) => ({ ...extra, version: 1, syncStepMs, autoReload });

/** Seconds for menu titles and the step list: 0.05, 0.5, 5. */
export const stepSeconds = (ms: number) => `${ms / 1000}`;

/**
 * 手动调整同步 input: whole milliseconds with an optional sign (ASCII or U+2212).
 * Returns the value, or the Chinese reason it was refused.
 */
export function parseSync(text: string): { value: number } | { error: string } {
  const trimmed = String(text).trim().replace(/^−/, "-");
  if (!/^[+-]?\d{1,9}$/.test(trimmed)) return { error: "请输入整数毫秒，例如 1250 或 -500。" };
  const value = Number(trimmed);
  if (Math.abs(value) > SYNC_LIMIT) return { error: `同步值须在 −${SYNC_LIMIT} 至 ${SYNC_LIMIT} 毫秒之间` };
  return { value };
}
