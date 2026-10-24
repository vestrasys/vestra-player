import type { LanguagePreference } from "@vestra-player/sdk";

/**
 * The interface languages in menu order (`plugin.appearance.setLanguage`).
 * Each is named in its own language, whatever the interface language, so
 * the names are not translated (i18n-ignore); "Follow System" is.
 */
// i18n-ignore: each language in its own words
export const LANGUAGES = [
  { id: "en", name: "English" },
  { id: "zh-Hans", name: "简体中文" },
  { id: "ja", name: "日本語" },
  { id: "ko", name: "한국어" },
  { id: "de", name: "Deutsch" },
  { id: "fr", name: "Français" },
  { id: "es", name: "Español" },
] as const satisfies readonly { id: LanguagePreference; name: string }[];

export const isLanguage = (value: unknown): value is LanguagePreference =>
  value === "system" || LANGUAGES.some((row) => row.id === value);
