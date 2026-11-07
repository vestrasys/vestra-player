// English source texts, the status table shared by the OSD and the windows, and the Chinese catalog.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { line, list, locationText, raw, sentences, serviceResult, statusText } from "../messages.ts";
import { trackTitle } from "../model.ts";
import type { Result } from "../services.ts";
import type { ListStyle, TemplateArgs, Text, TextArg, TextList } from "@vestra-player/sdk";

const directory = new URL("..", import.meta.url);
const read = (file: string) => readFile(new URL(`../${file}`, import.meta.url), "utf8");
/** `plugin.i18n.text` / `list`: plain text values, as the plugin makes them. */
const words = {
  text: (source: string, args?: TemplateArgs): Text => ({ $vestraText: source, args }) as Text,
  list: (items: readonly TextArg[], style?: ListStyle): TextList => ({ $vestraList: items, style }),
};
function zhHost() {
  const host = new TestHost();
  host.locale = "zh-Hans";
  host.loadCatalog(fileURLToPath(directory));
  return host;
}

test("Simplified Chinese: the menus show the old words", async () => {
  const { host, stop } = await startPlugin(activate, { directory, locale: "zh-Hans" });
  assert.deepEqual(host.command("search").display.path, ["字幕", "在线字幕", "搜索与下载字幕…"]);
  assert.deepEqual(host.command("settings").display.path, ["字幕", "在线字幕", "在线字幕设置…"]);
  await stop();
});

test("palette: every command has a category in every language, the submenu's words (player 0.7.6)", async () => {
  const expected = { en: "Online Subtitles", "zh-Hans": "在线字幕", ja: "オンライン字幕", ko: "온라인 자막", de: "Online-Untertitel", fr: "Sous-titres en ligne", es: "Subtítulos en línea" } as const;
  for (const [locale, category] of Object.entries(expected) as [keyof typeof expected, string][]) {
    const { host, stop } = await startPlugin(activate, { directory, locale });
    assert.equal(host.commands.size, 2);
    for (const [id, { display }] of host.commands) {
      assert.ok(typeof display.category === "string" && display.category.trim() !== "", `${locale}: ${id} has no category`);
      assert.equal(display.category, category, `${locale}: ${id}`);
    }
    await stop();
  }
});

test("a track title: the language name translated, the release as it is", () => {
  const { text } = words;
  const result = (language: string) => ({ language, release: "Movie.CN\u0001x" }) as Result;
  const zh = zhHost(), en = new TestHost();
  assert.equal(zh.translate(trackTitle(text, result("zh-CN"))), "简体中文 · Movie.CN x");
  assert.equal(en.translate(trackTitle(text, result("zh-CN"))), "Simplified Chinese · Movie.CN x");
  assert.equal(zh.translate(trackTitle(text, result("xx"))), "xx · Movie.CN x", "an unknown code stays as given");
});

test("a window joins lines with 。 or ；, as before; the player's own words stay", () => {
  const host = zhHost();
  const shown = (status: Parameters<typeof statusText>[1], on = host) => on.translate(statusText(words, status) as Text);
  const opened = sentences(line("opened", ["英语 · X"]), line("noteEncoding"), line("savedTo", ["/v/x.srt"]));
  assert.equal(shown(opened), "打开字幕：英语 · X。字幕编码可能不正确，可在字幕设置中更改默认编码。已保存到 /v/x.srt");
  assert.equal(shown(list(line("found", [3]), line("connect.assrt"))), "找到 3 条字幕；无法连接 射手网(伪)");
  assert.equal(shown(serviceResult("assrt", line("tokenRemaining", [9]))), "射手网(伪)：token 有效，剩余调用 9 次");
  assert.equal(statusText(words, sentences(raw("disk full"))), "disk full");
  assert.equal(statusText(words, sentences()), "", "nothing");
  const memory = locationText(words, { kind: "memory", value: "" });
  assert.equal(shown(sentences(line("saved", [memory]))), "已保存（仅本次运行）");
  assert.equal(host.translate(locationText(words, { kind: "macos", value: "x" })), "macOS 钥匙串（x）");
  assert.equal(shown(opened, new TestHost()), "Opened subtitle: 英语 · X. The subtitle encoding may be wrong; change the default encoding in Subtitle Settings. Saved to /v/x.srt");
});

test("every text is translated; no Chinese left in the sources", async () => {
  const pot = await read("locales/messages.pot");
  const po = await read("locales/zh-Hans.po");
  assert.doesNotMatch(po, /msgstr ""\n\n/);
  assert.equal([...pot.matchAll(/^msgid /gm)].length, [...po.matchAll(/^msgid /gm)].length, "the catalog has the template's texts");
  assert.ok(po.includes('msgid "{0}: {1}"\nmsgstr "{0}：{1}"'), "a service's result");
  for (const source of ["main.ts", "model.ts", "media.ts", "services.ts", "secrets.ts", "formats.ts", "messages.ts", "ui/download.slint", "ui/settings.slint", "ui/service.slint", "ui/choose.slint", "ui/messages.slint"]) {
    const code = (await read(source)).split("\n").filter((text) => !/^\s*(\/\/|\*|\/\*\*)/.test(text));
    assert.deepEqual(code.filter((text) => /\p{Script=Han}/u.test(text)), [], source);
  }
});
