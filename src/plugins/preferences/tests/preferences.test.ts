import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { ThemeId } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate, { PAGE_ID } from "../main.ts";
import { THEMES, titleOf } from "../themes.ts";

const directory = new URL("..", import.meta.url);
const running = new Set<() => Promise<void>>();
async function start(host = new TestHost()) {
  const started = await startPlugin(activate, { directory, host });
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
const checked = (host: TestHost) => host.command("theme.vestra").state.checked;
/** A palette id this plugin does not list (one a newer or older player could report). */
const OTHER = "light" as ThemeId;
/** A host whose current palette is not Vestra, so 恢复默认 and the radio switch. */
const other = () => {
  const host = new TestHost();
  host.appearance = OTHER;
  return host;
};

test("menu: 选项… (F5, 830) and one radio per palette from 840", async () => {
  const { host } = await start();
  const options = host.command("options").declaration;
  assert.deepEqual([options.path, options.order, options.group, options.bindings], [["设置", "选项…"], 830, "settings-main", ["F5"]]);
  const radios = THEMES.map((row) => host.command(`theme.${row.id}`).declaration);
  assert.deepEqual(
    radios.map((d) => [d.path, d.order, d.group, d.radio]),
    [[["设置", "外观", "Vestra"], 840, "settings-look", true]],
  );
  assert.equal(checked(host), true);
  assert.deepEqual(host.conflicts, []);
});

test("F5 opens the options window on the last page", async () => {
  const { host } = await start();
  await host.press("F5");
  await host.command("options").run();
  assert.deepEqual(host.settings.shows, [null, null]);
  assert.equal(host.settings.open, true);
});

test("外观 page: registered in 外观 with the palette and its preview", async () => {
  const { host } = await start();
  const p = page(host);
  assert.deepEqual([p.category, p.title, p.order, p.file, p.component, p.global], [
    "appearance",
    "外观",
    10,
    "ui/appearance.slint",
    "AppearancePage",
    "AppearanceBridge",
  ]);
  assert.equal(p.props.theme, "vestra");
  assert.equal(p.props["can-reset"], false);
  const themes = p.props.themes as { id: string; title: string; swatch: Record<string, { r: number; g: number; b: number }> }[];
  assert.deepEqual(themes.map((t) => [t.id, t.title]), [["vestra", "Vestra"]]);
  // The preview draws the palette's own colours, whatever is current.
  assert.deepEqual(themes[0]!.swatch.accent, { r: 0xfa, g: 0x13, b: 0x9c });
  assert.deepEqual(themes[0]!.swatch.bar, { r: 0x17, g: 0x14, b: 0x1c });
  assert.deepEqual(themes[0]!.swatch.f0, { r: 0xfd, g: 0xb5, b: 0x06 });
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
  assert.deepEqual(host.osd, ["配色：Vestra"]);
  assert.equal(checked(host), true);
  assert.equal(page(host).props.theme, "vestra");
  assert.equal(page(host).props["can-reset"], false);
});

test("page: 恢复默认, and picks that are not a listed palette", async () => {
  const { host } = await start(other());
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual(host.osd, ["配色：Vestra"]);
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
  assert.deepEqual(host.osd, ["配色：Vestra"]);
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
    throw new HostError("unavailable", "没有界面");
  });
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, OTHER);
  assert.equal(checked(host), false);
  assert.deepEqual(host.osd, ["没有界面"]);
  // The next attempt is not blocked by the failed one.
  host.reset("appearance.set");
  await host.command("theme.vestra").run();
  await host.settle();
  assert.equal(host.appearance, "vestra");
});

test("stopping removes the page and the menu, the palette stays", async () => {
  const { host, stop } = await start(other());
  await host.command("theme.vestra").run();
  await host.settle();
  await stop();
  assert.equal(host.settings.pages.size, 0);
  assert.equal(host.commands.size, 0);
  assert.equal(host.appearance, "vestra");
});

test("titles and the page's Slint has no colour literals", () => {
  assert.equal(titleOf("vestra"), "Vestra");
  assert.equal(titleOf(OTHER), OTHER);
  assert.equal(THEMES.length, 1);
  const slint = readFileSync(new URL("../ui/appearance.slint", import.meta.url), "utf8");
  assert.doesNotMatch(slint, /#[0-9a-fA-F]{3,8}\b/);
});
