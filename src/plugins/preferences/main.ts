import type { LanguagePreference, Plugin, ThemeId } from "@vestra-player/sdk";
import { isLanguage, LANGUAGES } from "./languages.ts";
import { DEFAULT_THEME, isTheme, THEMES, themeRows } from "./themes.ts";

/**
 * Options… (830, top level, settings group; menu sweep B5). Palette only:
 * Appearance (840–849, one radio per palette from 840) and Language (850–857,
 * Follow System then one radio per language).
 */
export const PAGE_ID = "appearance";
export const LANGUAGE_PAGE_ID = "language";

/**
 * Preferences: Options… (J04, F5) opens the player's options
 * window; this plugin's own pages there are Appearance (H10: palette radio
 * with previews and Restore Defaults) and Language (Follow System or one
 * language, used from the next start), also reachable in the command palette
 * as Appearance: <palette> and Language: <language>. The palette
 * and the language are the player's (`plugin.appearance`): the player saves
 * them, and they stay when this plugin stops.
 */
export default async function activate(plugin: Plugin) {
  plugin.player.subscribe(); // B3: removed in SDK 1.11; migrate (events-and-triggers.md §7)
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
  /**
   * Command palette categories ("Category: Title", player 0.7.6): "Settings"
   * for Options…, the submenu names for the radios (reusing the menu labels
   * and so their translations). Text values, so `i18n extract` finds them.
   * The radios' home is Options…: the options window has the Appearance and
   * Language pages with the same choices.
   */
  const { text } = plugin.i18n;
  const category = {
    settings: text("Settings"),
    appearance: text("Appearance"),
    language: text("Language"),
  };
  const HOME = "options";
  plugin.commands.add({
    id: "options",
    path: ["Options…"],
    category: category.settings,
    order: 830,
    group: "settings-main",
    bindings: ["F5"],
    run: () => plugin.settings.show(),
  });
  THEMES.forEach((row, index) =>
    plugin.commands.add({
      id: `theme.${row.id}`,
      title: row.title,
      category: category.appearance,
      home: HOME,
      order: 840 + index,
      group: "settings-look",
      radio: true,
      checked: when.theme(row.id),
      run: () => choose(row.id),
    }),
  );
  plugin.commands.add({
    id: "language.system",
    title: "Follow System",
    category: category.language,
    home: HOME,
    order: 850,
    group: "settings-look",
    radio: true,
    checked: when.language("system"),
    run: () => chooseLanguage("system"),
  });
  LANGUAGES.forEach((row, index) =>
    plugin.commands.add({
      id: `language.${row.id}`,
      // i18n-ignore: a language's own name is not translated
      title: row.name,
      category: category.language,
      home: HOME,
      order: 851 + index,
      group: "settings-language",
      radio: true,
      checked: when.language(row.id),
      run: () => chooseLanguage(row.id),
    }),
  );
}
