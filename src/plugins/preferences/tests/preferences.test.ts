import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { Locale, ThemeId } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate, { LANGUAGE_PAGE_ID, PAGE_ID } from "../main.ts";
import { LANGUAGES } from "../languages.ts";
import { THEMES } from "../themes.ts";

const directory = new URL("..", import.meta.url);
const running = new Set<() => Promise<void>>();
/** Start in English (the source texts) unless a locale is given. */
async function start(host = new TestHost(), locale: Locale = "en") {
  const started = await startPlugin(activate, { directory, host, locale });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  return { ...started, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

const page = (host: TestHost) => host.settingsPage(PAGE_ID);
const languagePage = (host: TestHost) => host.settingsPage(LANGUAGE_PAGE_ID);
const checked = (host: TestHost) => host.command("theme.vestra").state.checked;
/** A palette id this plugin does not list (one a newer or older player could report). */
const OTHER = "light" as ThemeId;
/** A host whose current palette is not Vestra, so Restore Defaults and the radio switch. */
const other = () => {
  const host = new TestHost();
  host.appearance = OTHER;
  return host;
};

test("menu: Options… (F5, 830), one radio per palette from 840, the languages from 850", async () => {
  const { host } = await start();
  const options = host.command("options").declaration;
  assert.deepEqual([options.path, options.order, options.group, options.bindings], [["@settings", "Options…"], 830, "settings-main", ["F5"]]);
  const radios = THEMES.map((row) => host.command(`theme.${row.id}`).declaration);
  assert.deepEqual(
    radios.map((d) => [d.path, d.order, d.group, d.radio]),
    [
      [["@settings", "Appearance", "Vestra Dark"], 840, "settings-look", true],
      [["@settings", "Appearance", "Vestra Light"], 841, "settings-look", true],
    ],
  );
  const system = host.command("language.system").declaration;
  assert.deepEqual([system.path, system.order, system.group, system.radio], [["@settings", "Language", "Follow System"], 850, "settings-look", true]);
  assert.deepEqual(
    LANGUAGES.map((row) => {
      const d = host.command(`language.${row.id}`).declaration;
      return [d.path.at(-1), d.order, d.group];
    }),
    [
      ["English", 851, "settings-language"],
      ["简体中文", 852, "settings-language"],
      ["日本語", 853, "settings-language"],
      ["한국어", 854, "settings-language"],
      ["Deutsch", 855, "settings-language"],
      ["Français", 856, "settings-language"],
      ["Español", 857, "settings-language"],
    ],
  );
  assert.equal(checked(host), true);
  assert.equal(host.command("language.system").state.checked, true);
  assert.deepEqual(host.conflicts, []);
});

test("Simplified Chinese shows the texts of before", async () => {
  const { host } = await start(new TestHost(), "zh-Hans");
  assert.deepEqual(host.command("options").display.path, ["设置", "选项…"]);
  assert.deepEqual(host.command("theme.vestra").display.path, ["设置", "外观", "Vestra 深色"]);
  assert.deepEqual(host.command("theme.vestra-light").display.path, ["设置", "外观", "Vestra 浅色"]);
  assert.deepEqual(host.command("language.system").display.path, ["设置", "语言", "跟随系统"]);
  // Each language keeps its own name.
  assert.deepEqual(host.command("language.ja").display.path, ["设置", "语言", "日本語"]);
  assert.equal(page(host).title, "外观");
  assert.equal(languagePage(host).title, "语言");
  await host.command("theme.vestra-light").run();
  await host.settle();
  assert.deepEqual(host.osd, ["配色：Vestra 浅色"]);
  await host.command("language.de").run();
  await host.command("language.system").run();
  await host.settle();
  assert.deepEqual(host.osd.slice(1), ["界面语言：Deutsch，重启后生效", "界面语言：跟随系统，重启后生效"]);
});

test("every Slint and TypeScript text of the plugin is translated into Simplified Chinese", () => {
  const read = (file: string) => readFileSync(new URL(`../locales/${file}`, import.meta.url), "utf8");
  const ids = [...read("messages.pot").matchAll(/^msgid "(.+)"$/gm)].map((m) => m[1]);
  assert.ok(ids.includes("Restore Defaults") && ids.includes("Takes effect after restart."));
  // No language name in the template: they are never translated.
  for (const row of LANGUAGES) assert.ok(!ids.includes(row.name), row.name);
  const zh = read("zh-Hans.po");
  assert.doesNotMatch(zh, /msgstr ""\n\n/);
  assert.match(zh, /msgid "Restore Defaults"\nmsgstr "恢复默认"/);
});

test("the light palette: menu radio and page pick switch to it, Restore Defaults returns", async () => {
  const { host } = await start();
  const light = () => host.command("theme.vestra-light").state.checked;
  assert.equal(light(), false);
  await host.command("theme.vestra-light").run();
  await host.settle();
  assert.equal(host.appearance, "vestra-light");
  assert.deepEqual(host.osd, ["Palette: Vestra Light"]);
  assert.deepEqual([checked(host), light()], [false, true]);
  assert.equal(page(host).props.theme, "vestra-light");
  assert.equal(page(host).props["can-reset"], true);
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual([checked(host), light()], [true, false]);
  page(host).emit("pick", "vestra-light");
  await host.settle();
  assert.equal(host.appearance, "vestra-light");
  assert.equal(host.sent("appearance.set").length, 3);
});

test("F5 opens the options window on the last page", async () => {
  const { host } = await start();
  await host.press("F5");
  await host.command("options").run();
  assert.deepEqual(host.settings.shows, [null, null]);
  assert.equal(host.settings.open, true);
});

test("Appearance page: registered in Appearance with the palettes and their previews", async () => {
  const { host } = await start();
  const p = page(host);
  assert.deepEqual([p.category, p.title, p.order, p.file, p.component, p.global], [
    "appearance",
    "Appearance",
    10,
    "ui/appearance.slint",
    "AppearancePage",
    "AppearanceBridge",
  ]);
  assert.equal(p.props.theme, "vestra");
  assert.equal(p.props["can-reset"], false);
  const themes = p.props.themes as { id: string; swatch: Record<string, { r: number; g: number; b: number }> }[];
  // Ids and colours only: the names and notes are in the page's Slint.
  assert.deepEqual(
    themes.map((t) => Object.keys(t)),
    [
      ["id", "swatch"],
      ["id", "swatch"],
    ],
  );
  assert.deepEqual(
    themes.map((t) => t.id),
    ["vestra", "vestra-light"],
  );
  // The preview draws the palette's own colours, whatever is current.
  assert.deepEqual(themes[0]!.swatch.accent, { r: 0xf7, g: 0x2a, b: 0xa6 });
  assert.deepEqual(themes[0]!.swatch.bar, { r: 0x16, g: 0x14, b: 0x1c });
  assert.deepEqual(themes[0]!.swatch.f0, { r: 0xfc, g: 0xc4, b: 0x02 });
  assert.deepEqual(themes[1]!.swatch.bar, { r: 0xf7, g: 0xf6, b: 0xfa });
  assert.deepEqual(themes[1]!.swatch.accent, { r: 0xd4, g: 0x12, b: 0x8a });
  // The light palette keeps a dark video area.
  assert.deepEqual(themes[1]!.swatch.video, { r: 0x0e, g: 0x0d, b: 0x12 });
});

test("menu radio: choosing the current palette does nothing", async () => {
  const { host } = await start();
  await host.command("theme.vestra").run();
  page(host).emit("pick", "vestra");
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 0);
  assert.deepEqual(host.osd, []);
});

test("menu radio switches to Vestra from another palette, with the OSD", async () => {
  const { host } = await start(other());
  assert.equal(checked(host), false);
  assert.equal(page(host).props["can-reset"], true);
  await host.command("theme.vestra").run();
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual(host.osd, ["Palette: Vestra Dark"]);
  assert.equal(checked(host), true);
  assert.equal(page(host).props.theme, "vestra");
  assert.equal(page(host).props["can-reset"], false);
});

test("page: Restore Defaults, and picks that are not a listed palette", async () => {
  const { host } = await start(other());
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual(host.osd, ["Palette: Vestra Dark"]);
  assert.equal(page(host).props["can-reset"], false);
  // Already the default, or not a palette this plugin lists: nothing is sent.
  page(host).emit("reset");
  page(host).emit("pick", "dark");
  page(host).emit("pick", OTHER);
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 1);
  assert.equal(host.osd.length, 1);
});

test("quick repeated clicks switch once", async () => {
  const { host } = await start(other());
  page(host).emit("pick", "vestra");
  page(host).emit("reset");
  await host.command("theme.vestra").run();
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 1);
  assert.deepEqual(host.osd, ["Palette: Vestra Dark"]);
});

test("the saved palette shows at start; another plugin's switch is followed silently", async () => {
  const { host } = await start(other());
  assert.equal(checked(host), false);
  assert.equal(page(host).props.theme, OTHER);
  host.setAppearance("vestra");
  await host.settle();
  assert.equal(checked(host), true);
  assert.equal(page(host).props.theme, "vestra");
  assert.deepEqual(host.osd, []);
});

test("a refused switch is reported and changes nothing", async () => {
  const { host } = await start(other());
  host.handle("appearance.set", () => {
    throw new HostError("unavailable", "no user interface");
  });
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, OTHER);
  assert.equal(checked(host), false);
  assert.deepEqual(host.osd, ["no user interface"]);
  // The next attempt is not blocked by the failed one.
  host.reset("appearance.set");
  await host.command("theme.vestra").run();
  await host.settle();
  assert.equal(host.appearance, "vestra");
});

test("Language: the menu and the page save the choice for the next start", async () => {
  // Run in a translated language other than the one picked, so a switch at
  // runtime would show. The texts come from this run's catalog (whatever
  // its wording), never from the machine's locale.
  const { host, plugin } = await start(new TestHost(), "fr");
  const shown = (text: string, args?: string[]) => host.translate(text, args);
  assert.notEqual(shown("Language"), "Language", "fr has a catalog");
  const p = languagePage(host);
  assert.deepEqual([p.category, p.title, p.order, p.file, p.component, p.global], [
    "general",
    shown("Language"),
    10,
    "ui/language.slint",
    "LanguagePage",
    "LanguageBridge",
  ]);
  assert.equal(p.props.language, "system");
  assert.deepEqual(p.props.languages, [...LANGUAGES]);
  await host.command("language.ja").run();
  await host.settle();
  assert.equal(host.language, "ja");
  assert.equal(host.command("language.ja").state.checked, true);
  assert.equal(host.command("language.system").state.checked, false);
  assert.equal(p.props.language, "ja");
  assert.deepEqual(host.osd, [shown("Language: {}. Takes effect after restart", ["日本語"])]);
  // The language of this run stays (restart-only): same locale, same texts.
  assert.equal(plugin.i18n.locale, "fr");
  assert.equal(host.locale, "fr");
  assert.equal(p.title, shown("Language"));
  assert.equal(host.command("language.system").display.path.at(-1), shown("Follow System"));
  // The page, the same choice again, and an unknown one.
  p.emit("pick", "system");
  p.emit("pick", "system");
  p.emit("pick", "tlh");
  await host.settle();
  assert.equal(host.language, "system");
  assert.equal(host.sent("appearance.setLanguage").length, 2);
  assert.deepEqual(host.osd.at(-1), shown("Language: Follow System. Takes effect after restart"));
  // Another plugin's change is followed silently.
  host.setLanguage("ko");
  await host.settle();
  assert.equal(host.command("language.ko").state.checked, true);
  assert.equal(p.props.language, "ko");
  assert.equal(host.osd.length, 2);
});

test("stopping removes the pages and the menu, the palette and the language stay", async () => {
  const { host, stop } = await start(other());
  await host.command("theme.vestra").run();
  await host.command("language.de").run();
  await host.settle();
  await stop();
  assert.equal(host.settings.pages.size, 0);
  assert.equal(host.commands.size, 0);
  assert.equal(host.appearance, "vestra");
  assert.equal(host.language, "de");
});

test("the pages' Slint has no colour literals", () => {
  assert.equal(THEMES.length, 2);
  for (const file of ["appearance", "language"]) {
    const slint = readFileSync(new URL(`../ui/${file}.slint`, import.meta.url), "utf8");
    assert.doesNotMatch(slint, /#[0-9a-fA-F]{3,8}\b/);
  }
});
