// English source texts, the status table shared by the OSD and the windows, and the Chinese catalog.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { statusText } from "../messages.ts";
import type { Status } from "../messages.ts";
import { SYNC_MENU, SYNC_STEPS } from "../model.ts";
import { shownStatus, words, zh } from "./status.ts";
import type { Text } from "@vestra-player/sdk";

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
  assert.equal(shownStatus(window.props), "Subtitle style updated");
  const sent = [...host.sent("ui.open"), ...host.sent("ui.set")].map((m) =>
    m.property === "status" ? m.value : (m.properties as { status?: unknown } | undefined)?.status);
  assert.deepEqual(sent.findLast(Boolean), { $vestraText: "Subtitle style updated", args: [] }, "a text value");
  assert.deepEqual(window.props["track-labels"], ["Subtitles Off", "English · en"]);
  assert.deepEqual(window.props["secondary-labels"], ["Secondary Off", "English · en"]);
  assert.equal(host.command("dialogue-next").state.reason, "No text subtitles available");
  await stop();
});

test("fallback track names are translated in the menu, the tune lists and the OSD; the menu keys stay English", async () => {
  for (const [locale, names] of [["en", ["Embedded 3 · ja", "Embedded 4", "Embedded 4 (4)", "Off (5)"]], ["zh-Hans", ["内嵌 3 · ja", "内嵌 4", "内嵌 4 (4)", "Off (5)"]]] as const) {
    const host = sourceHost();
    for (const [id, title, language] of [[3, "", "ja"], [4, "", null], [4, "", null], [5, "Off", null]] as const)
      host.subtitles.tracks.push({ target: { kind: "embedded", id }, title, language, styleSupported: true, styleReason: null });
    const { stop } = await startPlugin(activate, { directory, host, locale });
    assert.deepEqual([2, 3, 4, 5].map((n) => host.command(`track-${n}`).display.path.at(-1)), names, locale);
    await host.command("track-2").run();
    assert.equal(host.osd.at(-1), locale === "en" ? "Subtitles: Embedded 3 · ja" : "字幕：内嵌 3 · ja");
    await stop();
  }
});

test("Simplified Chinese: a window joins a status and its notes with 。", () => {
  const opened: Status = { id: "opened", args: ["b.ass"], notes: ["noteEncoding", "noteFonts"] };
  assert.equal(zh(opened), "打开字幕：b.ass。字幕编码可能不正确，可在字幕设置中更改默认编码。已忽略字幕内嵌字体");
  assert.equal(
    new TestHost().translate(statusText(words, opened) as Text),
    "Opened subtitle: b.ass. The subtitle encoding may be wrong; change the default encoding in Subtitle Settings. Fonts embedded in the subtitle were ignored",
  );
  assert.equal(zh({ id: "cycle.waiting", args: ["中文 · zh", 2, 3] }), "字幕：中文 · zh（2/3）（等待生效）");
  assert.equal(zh({ id: "raw", args: ["图形字幕"] }), "图形字幕", "the player's own words stay");
});

test("fixed lists stay in step with their Slint names; every text is translated", async () => {
  assert.deepEqual(SYNC_MENU.map((row) => row.step), [...SYNC_STEPS]);
  const settings = await read("ui/settings.slint");
  assert.equal([...settings.matchAll(/@tr\("\{\} s", "[\d.]+"\)/g)].length, SYNC_STEPS.length);
  const pot = await read("locales/messages.pot");
  const po = await read("locales/zh-Hans.po");
  assert.doesNotMatch(po, /msgstr ""\n\n/);
  assert.match(po, /msgid "Subtitle Tuning…"\nmsgstr "字幕微调…"/);
  assert.equal([...pot.matchAll(/^msgid /gm)].length, [...po.matchAll(/^msgid /gm)].length, "the catalog has the template's texts");
  for (const source of ["main.ts", "model.ts", "formats.ts", "messages.ts", "ui/tune.slint", "ui/settings.slint", "ui/sync.slint"]) {
    const code = (await read(source)).split("\n").filter((line) => !/^\s*(\/\/|\*|\/\*\*)/.test(line));
    assert.deepEqual(code.filter((line) => /\p{Script=Han}/u.test(line)), [], source);
  }
});

test("every command has a category for the command palette, translated, and controls in a window name it as home", async () => {
  const expected: Record<string, [string, string | null]> = {
    "load": ["Subtitles", "tune"],
    "load-secondary": ["Subtitles", null],
    "show": ["Subtitles", null],
    "track-off": ["Subtitle Track", "tune"],
    "track-1": ["Subtitle Track", "tune"],
    "track-cycle": ["Subtitle Track", null],
    "second-off": ["Secondary Subtitle", "tune"],
    "second-1": ["Secondary Subtitle", "tune"],
    "second-position-top": ["Secondary Subtitle Position", null],
    "second-position-aboveMain": ["Secondary Subtitle Position", null],
    "size-up": ["Subtitle Style", "tune"],
    "size-down": ["Subtitle Style", "tune"],
    "move-up": ["Subtitle Style", "tune"],
    "move-down": ["Subtitle Style", "tune"],
    "bold": ["Subtitle Style", "settings"],
    "shadow": ["Subtitle Style", "settings"],
    "box": ["Subtitle Style", "settings"],
    "ass-original": ["Subtitles", "settings"],
    "move-reset": ["Reset", "tune"],
    "sync-reset": ["Reset", "tune"],
    "dialogue-previous": ["Seek", null],
    "dialogue-next": ["Seek", null],
    "dialogue-current": ["Seek", null],
    "sync-earlier": ["Subtitle Sync", "tune"],
    "sync-later": ["Subtitle Sync", "tune"],
    "sync-input": ["Subtitle Sync", null],
    "reload": ["Subtitles", null],
    "refresh": ["Subtitles", null],
    "tune": ["Subtitles", null],
    "settings": ["Subtitles", null],
  };
  for (const locale of ["en", "zh-Hans"] as const) {
    const host = sourceHost();
    Object.assign(host.subtitles.tracks[0]!, { codec: "subrip", text: true }); // so Secondary Subtitle ▸ lists it
    const { stop } = await startPlugin(activate, { directory, host, locale });
    const commands = [...host.commands.entries()];
    for (const [id, { display }] of commands) assert.ok(display.category, `${id} has a category (${locale})`);
    const shown = Object.fromEntries(commands.map(([id, { declaration: d, display }]) => [id, [display.category, d.home ?? null]]));
    if (locale === "en") assert.deepEqual(shown, expected);
    else {
      assert.equal(shown["second-position-top"]![0], "次字幕位置");
      assert.equal(shown["sync-reset"]![0], "复位");
      assert.equal(shown["dialogue-next"]![0], "跳转");
      for (const [id, [category]] of Object.entries(expected)) assert.notEqual(shown[id]![0], category, `${id} is translated`);
    }
    // Every home is a command of this plugin that opens a window.
    for (const [, home] of Object.values(expected)) if (home) assert.ok(host.commands.has(home));
    await stop();
  }
});
