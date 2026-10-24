// English source texts, the status table shared by the OSD and the windows, and the Chinese catalog.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { STATUS, forSlint, line, list, raw, sentences, serviceResult } from "../messages.ts";
import { LANGUAGE_CODES } from "../services.ts";
import { LOCATIONS, statusText } from "./status.ts";

const directory = new URL("..", import.meta.url);
const read = (file: string) => readFile(new URL(`../${file}`, import.meta.url), "utf8");
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

test("a window joins lines with 。 or ；, as before; the player's own words stay", () => {
  const host = zhHost();
  const opened = sentences(line("opened", ["英语 · X"]), line("noteEncoding"), line("savedTo", ["/v/x.srt"]));
  assert.equal(statusText(host, forSlint(opened)), "打开字幕：英语 · X。字幕编码可能不正确，可在字幕设置中更改默认编码。已保存到 /v/x.srt");
  const found = list(line("found", [3]), line("connect.assrt"));
  assert.equal(statusText(host, forSlint(found)), "找到 3 条字幕；无法连接 射手网(伪)");
  assert.equal(statusText(host, forSlint(serviceResult("assrt", line("tokenRemaining", [9])))), "射手网(伪)：token 有效，剩余调用 9 次");
  assert.equal(statusText(host, forSlint(sentences(raw("disk full")))), "disk full");
  assert.equal(statusText(host, forSlint(sentences(line("saved", ["memory", ""])))), "已保存（仅本次运行）");
  assert.equal(statusText(new TestHost(), forSlint(opened)), "Opened subtitle: 英语 · X. The subtitle encoding may be wrong; change the default encoding in Subtitle Settings. Saved to /v/x.srt");
});

test("ui/messages.slint has every status of messages.ts with the same English text, and the same names", async () => {
  const slint = await read("ui/messages.slint");
  const found = new Map([...slint.matchAll(/m\.id == "([\w.]+)" \{ return @tr\("((?:[^"\\]|\\.)*)"/g)].map((m) => [m[1]!, m[2]!]));
  assert.deepEqual(Object.fromEntries(found), { ...STATUS });
  // Each status passes exactly as many arguments as its text has placeholders.
  for (const [, id, text, args] of slint.matchAll(/m\.id == "([\w.]+)" \{ return @tr\("((?:[^"\\]|\\.)*)"(.*)\); \}$/gm))
    assert.equal(args ? args.split(/, (?![^(]*\))/).length - 1 : 0, text!.split("{}").length - 1, id);
  const codes = [...slint.matchAll(/if code == "([\w-]+)"/g)].map((m) => m[1]!);
  assert.deepEqual(codes, LANGUAGE_CODES);
  const locations = Object.fromEntries([...slint.matchAll(/if kind == "(\w+)" \{ return @tr\("([^"]*)"/g)].map((m) => [m[1]!, m[2]!]));
  assert.deepEqual({ ...locations, memory: "This run only" }, LOCATIONS);
});

test("every text is translated; no Chinese left in the sources", async () => {
  const pot = await read("locales/messages.pot");
  const po = await read("locales/zh-Hans.po");
  assert.doesNotMatch(po, /msgstr ""\n\n/);
  assert.equal([...pot.matchAll(/^msgid /gm)].length, [...po.matchAll(/^msgid /gm)].length);
  for (const source of ["main.ts", "model.ts", "media.ts", "services.ts", "secrets.ts", "formats.ts", "messages.ts", "ui/download.slint", "ui/settings.slint", "ui/service.slint", "ui/choose.slint", "ui/messages.slint"]) {
    const code = (await read(source)).split("\n").filter((text) => !/^\s*(\/\/|\*|\/\*\*)/.test(text));
    assert.deepEqual(code.filter((text) => /\p{Script=Han}/u.test(text)), [], source);
  }
});
