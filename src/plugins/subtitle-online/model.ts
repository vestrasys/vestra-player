// settings.json and result ordering. Secrets are not here (see secrets.ts).
import type { I18n, Text } from "@vestra-player/sdk";
import type { Result, ServiceId } from "./services.ts";
import { SERVICE_IDS, languageLabel } from "./services.ts";

export type Settings = {
  version: 1;
  /** Services in search order; OpenSubtitles on, assrt off by default. */
  services: { id: ServiceId; enabled: boolean }[];
  /** Preferred languages, at most 5, e.g. ["zh-CN", "zh-TW", "en"]. */
  languages: string[];
  autoSearch: "off" | "no-subtitles";
  afterSearch: "show" | "load-best";
  exactOnly: boolean;
  exclude: string;
  saveLocation: "video" | "data";
  /** OpenSubtitles user name (the password is a secret). */
  username: string;
  privacyAccepted: boolean;
};

export const DEFAULTS: Settings = {
  version: 1,
  services: [
    { id: "opensubtitles", enabled: true },
    { id: "assrt", enabled: false },
  ],
  languages: ["zh-CN", "zh-TW", "en"],
  autoSearch: "off",
  afterSearch: "show",
  exactOnly: false,
  exclude: "",
  saveLocation: "video",
  username: "",
  privacyAccepted: false,
};

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
export const isSettingsFile = (v: unknown): v is Record<string, unknown> => isObject(v);

/** Read leniently: a broken field takes its default; services missing from the file are appended. */
export function parseSettings(raw: unknown): Settings {
  const v = isObject(raw) ? raw : {};
  const listed = Array.isArray(v.services) ? v.services.filter(isObject) : [];
  const services: Settings["services"] = [];
  for (const item of listed)
    if (SERVICE_IDS.includes(item.id as ServiceId) && !services.some((s) => s.id === item.id))
      services.push({ id: item.id as ServiceId, enabled: item.enabled === true });
  for (const fallback of DEFAULTS.services) if (!services.some((s) => s.id === fallback.id)) services.push({ ...fallback });
  return {
    version: 1,
    services,
    languages: parseLanguages(Array.isArray(v.languages) ? v.languages.join(",") : DEFAULTS.languages.join(",")) ?? DEFAULTS.languages,
    autoSearch: v.autoSearch === "no-subtitles" ? "no-subtitles" : "off",
    afterSearch: v.afterSearch === "load-best" ? "load-best" : "show",
    exactOnly: v.exactOnly === true,
    exclude: typeof v.exclude === "string" ? v.exclude.slice(0, 200) : "",
    saveLocation: v.saveLocation === "data" ? "data" : "video",
    username: typeof v.username === "string" ? v.username.slice(0, 100) : "",
    privacyAccepted: v.privacyAccepted === true,
  };
}

/** "zh-CN, en" → ["zh-CN", "en"]: 1–5 codes like `xx` or `xx-YY`; `null` when invalid. */
export function parseLanguages(text: string): string[] | null {
  const codes = text.split(/[,，\s]+/).map((c) => c.trim()).filter(Boolean);
  if (!codes.length || codes.length > 5) return null;
  if (!codes.every((c) => /^[a-z]{2,3}(-[a-z]{2,4})?$/i.test(c))) return null;
  return [...new Set(codes.map((c) => (c.includes("-") ? `${c.split("-")[0]!.toLowerCase()}-${c.split("-")[1]!.toUpperCase()}` : c.toLowerCase())))];
}

/**
 * Results in display order: hash matches first, then human translations before
 * machine ones, then the preferred language order, then downloads. `exclude`
 * drops releases containing any of its words; `exactOnly` keeps hash matches only.
 */
export function rank(results: Result[], settings: Pick<Settings, "languages" | "exclude" | "exactOnly">) {
  const words = settings.exclude.split(/[,，\s]+/).map((w) => w.trim().toLowerCase()).filter(Boolean);
  const order = settings.languages.map((l) => l.toLowerCase());
  const place = (r: Result) => {
    const code = r.language.toLowerCase();
    const i = order.findIndex((l) => l === code || (!l.includes("-") && code.startsWith(`${l}-`)));
    return i < 0 ? order.length : i;
  };
  const seen = new Set<string>();
  return results
    .filter((r) => !seen.has(r.key) && seen.add(r.key))
    .filter((r) => !settings.exactOnly || r.hashMatch)
    .filter((r) => !words.some((w) => r.release.toLowerCase().includes(w)))
    .sort(
      (a, b) =>
        Number(b.hashMatch) - Number(a.hashMatch) ||
        Number(a.machine) - Number(b.machine) ||
        place(a) - place(b) ||
        (b.downloads ?? -1) - (a.downloads ?? -1),
    );
}

/** A language's name by its code (any case), else the code as given. */
function languageName(text: I18n["text"], code: string): Text | string {
  const name = languageLabel(code);
  return name ? text(name) : code;
}

/**
 * One row of the result list (ui/download.slint `ResultRow`): data, and the
 * language's name (translated by the player; an unknown code as given); the
 * window names the match and the service, and marks hearing-impaired and
 * machine-translated releases.
 */
export const row = (text: I18n["text"], r: Result, checked: boolean) => ({
  checked,
  language: languageName(text, r.language),
  release: r.release,
  hearing: r.hearingImpaired,
  machine: r.machine,
  format: r.format,
  fps: r.fps ? String(Math.round(r.fps * 1000) / 1000) : "",
  downloads: r.downloads === null ? "" : String(r.downloads),
  hash: r.hashMatch,
  service: r.service,
});

/**
 * Track title: "<language> · <release>" (the language name translated, the
 * release data), at most 200 characters with a SAMI language after it.
 */
export function trackTitle(text: I18n["text"], r: Result): Text {
  const release = r.release.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 150);
  return text("{} · {}", [languageName(text, r.language.slice(0, 20)), release]);
}

/** The language part of a saved file name: "zh-CN" → "zh". */
export const fileLanguage = (code: string) => code.split("-")[0]!.toLowerCase() || "und";
