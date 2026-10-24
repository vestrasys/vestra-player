// English source texts, the status table shared by the OSD and the windows, and the Chinese catalog.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { STATUS } from "../messages.ts";
import { SYNC_MENU, SYNC_STEPS } from "../model.ts";
import { shownStatus, statusText, zh } from "./status.ts";

const directory = new URL("..", import.meta.url);
const embedded = { kind: "embedded" as const, id: 1 };
function sourceHost() {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.subtitles.tracks.push({ target: embedded, title: "English", language: "en", styleSupported: true, styleReason: null });
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: embedded };
  return host;
}
const read = (file: string) => readFile(new URL(`../${file}`, import.meta.url), "utf8");

test("English by default: menus, on-screen messages and window status", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host });
  assert.deepEqual(host.command("load").declaration.path, ["@open", "Open Subtitle…"]);
  assert.deepEqual(host.command("dialogue-next").declaration.path, ["@playback", "Seek", "Next Dialogue"]);
  assert.deepEqual(host.command("track-off").declaration.path, ["@subtitles", "Subtitle Track", "Off"]);
  assert.deepEqual(host.command("sync-earlier").declaration.path, ["@subtitles", "Sync", "Earlier 0.5 s"]);
  assert.deepEqual(host.command("box").display.path.slice(1), ["Subtitle Style", "Opaque Box"]);
  assert.equal(host.command("track-1").declaration.path.at(-1), "English · en", "track names are data");
  await host.press("Alt+PageUp");
  await host.press(".");
  assert.deepEqual(host.osd, ["Subtitle size: larger (38)", "Subtitle sync: 0.5 s earlier"]);
  assert.deepEqual(host.sent("osd.show").at(0), { text: "Subtitle size: larger ({})", args: [38] });
  await host.settle(170);
  await host.command("tune").run();
  const window = host.window("subtitle-tune");
  assert.equal(shownStatus(host, window.props), "Subtitle style updated");
  assert.deepEqual(window.props.status, { id: "styleUpdated", a: "", b: "", c: "", n1: "", n2: "" });
  assert.equal(host.command("dialogue-next").state.reason, "No text subtitles available");
  await stop();
});

test("Simplified Chinese: a window joins a status and its notes with 。", () => {
  const host = new TestHost();
  host.locale = "zh-Hans";
  host.loadCatalog(new URL("..", import.meta.url).pathname);
  const status = { id: "opened", a: "b.ass", b: "1", c: "", n1: "noteEncoding", n2: "noteFonts" };
  assert.equal(statusText(host, status), "打开字幕：b.ass。字幕编码可能不正确，可在字幕设置中更改默认编码。已忽略字幕内嵌字体");
  assert.equal(zh({ id: "cycle.waiting", args: ["中文 · zh", 2, 3] }), "字幕：中文 · zh（2/3）（等待生效）");
  assert.equal(zh({ id: "raw", args: ["图形字幕"] }), "图形字幕", "the player's own words stay");
});

test("ui/messages.slint has every status of messages.ts with the same English text", async () => {
  const slint = await read("ui/messages.slint");
  const found = new Map(
    [...slint.matchAll(/(?:m\.id|id) == "([\w.]+)" \{ return @tr\("((?:[^"\\]|\\.)*)"/g)].map((m) => [m[1]!, m[2]!]),
  );
  assert.deepEqual(Object.fromEntries(found), { ...STATUS });
  // Each status passes exactly as many arguments as its text has placeholders.
  for (const [, id, text, args] of slint.matchAll(/m\.id == "([\w.]+)" \{ return @tr\("((?:[^"\\]|\\.)*)"((?:, m\.[abc])*)\)/g))
    assert.equal(args!.split(",").length - 1, text!.split("{}").length - 1, id);
});

test("fixed lists stay in step with their Slint names; every text is translated", async () => {
  assert.deepEqual(SYNC_MENU.map((row) => row.step), [...SYNC_STEPS]);
  const settings = await read("ui/settings.slint");
  assert.equal([...settings.matchAll(/@tr\("\{\} s", "[\d.]+"\)/g)].length, SYNC_STEPS.length);
  const pot = await read("locales/messages.pot");
  const po = await read("locales/zh-Hans.po");
  assert.doesNotMatch(po, /msgstr ""\n\n/);
  assert.match(po, /msgid "Subtitle Tuning…"\nmsgstr "字幕微调…"/);
  assert.equal([...pot.matchAll(/^msgid /gm)].length, [...po.matchAll(/^msgid /gm)].length);
  for (const source of ["main.ts", "model.ts", "formats.ts", "messages.ts", "ui/tune.slint", "ui/settings.slint", "ui/sync.slint"]) {
    const code = (await read(source)).split("\n").filter((line) => !/^\s*(\/\/|\*|\/\*\*)/.test(line) && !line.includes("i18n-ignore"));
    const chinese = code.filter((line) => /\p{Script=Han}/u.test(line) && !/i18n-ignore|R2-subtitles-1/.test(line));
    // The two [string] off-entries wait for R2-subtitles-1 (marked i18n-ignore inline).
    assert.deepEqual(chinese.filter((line) => !/关闭(次)?字幕/.test(line)), [], source);
  }
});
