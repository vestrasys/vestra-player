import type { LanguagePreference, Plugin, ThemeId } from "@vestra-player/sdk";
import { isLanguage, LANGUAGES } from "./languages.ts";
import { DEFAULT_THEME, isTheme, THEMES, themeRows } from "./themes.ts";

/**
 * Settings ▸ (Options… 830; Appearance ▸ 840–849, one radio per palette
 * from 840; Language ▸ 850–857, Follow System then one radio per language).
 * `@settings` is the player's Settings menu.
 */
const SETTINGS = "@settings";
export const PAGE_ID = "appearance";
export const LANGUAGE_PAGE_ID = "language";

/**
 * Preferences: Settings ▸ Options… (J04, F5) opens the player's options
 * window; this plugin's own pages there are Appearance (H10: palette radio
 * with previews and Restore Defaults) and Language (Follow System or one
 * language, used from the next start), also reachable as Settings ▸
 * Appearance ▸ <palette> and Settings ▸ Language ▸ <language>. The palette
 * and the language are the player's (`plugin.appearance`): the player saves
 * them, and they stay when this plugin stops.
 */
export default async function activate(plugin: Plugin) {
  const { appearance } = plugin;
  plugin.onError((error) => {
    // i18n-ignore: the player's (or the SDK's) own message
    void plugin.osd.show(error.message).catch(() => {});
  });

  // One switch at a time: a second click waits and then sees the new value.
  let queue: Promise<void> = Promise.resolve();
  const serial = (work: () => Promise<void>) => {
    const next = queue.then(work);
    queue = next.catch(() => {});
    return next;
  };
  // Headless hosts refuse the display; that never fails the action itself.
  const choose = (theme: ThemeId) =>
    serial(async () => {
      if (theme === appearance.theme) return;
      await appearance.set(theme);
      void plugin.osd.show(theme === "vestra-light" ? "Palette: Vestra Light" : "Palette: Vestra Dark").catch(() => {});
    });
  const chooseLanguage = (language: LanguagePreference) =>
    serial(async () => {
      if (language === appearance.language) return;
      await appearance.setLanguage(language);
      const name = LANGUAGES.find((row) => row.id === language)?.name;
      void (
        name === undefined
          ? plugin.osd.show("Language: Follow System. Takes effect after restart")
          : plugin.osd.show("Language: {}. Takes effect after restart", [name])
      ).catch(() => {});
    });

  const page = plugin.settings.page("ui/appearance.slint", {
    id: PAGE_ID,
    category: "appearance",
    title: "Appearance",
    order: 10,
  });
  page.bind(() => ({
    themes: themeRows(),
    theme: appearance.theme,
    canReset: appearance.theme !== DEFAULT_THEME,
  }));
  page.on("pick", (theme: unknown) => (isTheme(theme) ? choose(theme) : undefined));
  page.on("reset", () => choose(DEFAULT_THEME));

  const languages = plugin.settings.page("ui/language.slint", {
    id: LANGUAGE_PAGE_ID,
    category: "general",
    title: "Language",
    order: 10,
  });
  languages.bind(() => ({
    languages: LANGUAGES.map(({ id, name }) => ({ id, name })),
    language: appearance.language,
  }));
  languages.on("pick", (language: unknown) => (isLanguage(language) ? chooseLanguage(language) : undefined));

  // The check marks are conditions the player evaluates (`plugin.when`, player
  // 0.5.21): they follow a switch made anywhere without this plugin running.
  const { when } = plugin;
  plugin.commands.add({
    id: "options",
    path: [SETTINGS, "Options…"],
    order: 830,
    group: "settings-main",
    bindings: ["F5"],
    run: () => plugin.settings.show(),
  });
  THEMES.forEach((row, index) =>
    plugin.commands.add({
      id: `theme.${row.id}`,
      path: [SETTINGS, "Appearance", row.title],
      order: 840 + index,
      group: "settings-look",
      radio: true,
      checked: when.theme(row.id),
      run: () => choose(row.id),
    }),
  );
  plugin.commands.add({
    id: "language.system",
    path: [SETTINGS, "Language", "Follow System"],
    order: 850,
    group: "settings-look",
    radio: true,
    checked: when.language("system"),
    run: () => chooseLanguage("system"),
  });
  LANGUAGES.forEach((row, index) =>
    plugin.commands.add({
      id: `language.${row.id}`,
      path: [SETTINGS, "Language", /* i18n-ignore */ row.name],
      order: 851 + index,
      group: "settings-language",
      radio: true,
      checked: when.language(row.id),
      run: () => chooseLanguage(row.id),
    }),
  );
}
