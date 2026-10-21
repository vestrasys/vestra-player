import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
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
const checked = (host: TestHost) => [host.command("theme.vestra").state.checked, host.command("theme.classic").state.checked];

test("menu: 选项… (F5, 830) and the two palette radios (840, 841)", async () => {
  const { host } = await start();
  const options = host.command("options").declaration;
  assert.deepEqual([options.path, options.order, options.group, options.bindings], [["设置", "选项…"], 830, "settings-main", ["F5"]]);
  const radios = ["theme.vestra", "theme.classic"].map((id) => host.command(id).declaration);
  assert.deepEqual(
    radios.map((d) => [d.path, d.order, d.group, d.radio]),
    [
      [["设置", "外观", "Vestra"], 840, "settings-look", true],
      [["设置", "外观", "经典"], 841, "settings-look", true],
    ],
  );
  assert.deepEqual(checked(host), [true, false]);
  assert.deepEqual(host.conflicts, []);
});

test("F5 opens the options window on the last page", async () => {
  const { host } = await start();
  await host.press("F5");
  await host.command("options").run();
  assert.deepEqual(host.settings.shows, [null, null]);
  assert.equal(host.settings.open, true);
});

test("外观 page: registered in 外观 with both palettes and their previews", async () => {
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
  assert.deepEqual(themes.map((t) => [t.id, t.title]), [["vestra", "Vestra"], ["classic", "经典"]]);
  // The previews draw each palette's own colours, whatever is current.
  assert.deepEqual(themes[0]!.swatch.accent, { r: 0xfa, g: 0x13, b: 0x9c });
  assert.deepEqual(themes[1]!.swatch.accent, { r: 0xfa, g: 0xe1, b: 0x00 });
  assert.deepEqual(themes[1]!.swatch.bar, { r: 0x1e, g: 0x1e, b: 0x1e });
  assert.deepEqual(themes[0]!.swatch.f0, { r: 0xfd, g: 0xb5, b: 0x06 });
  for (const stop of ["f0", "f1", "f2", "f3"]) assert.deepEqual(themes[1]!.swatch[stop], themes[1]!.swatch.accent, "solid fill");
});

test("menu radio switches the palette, with the OSD", async () => {
  const { host } = await start();
  await host.command("theme.classic").run();
  await host.settle();
  assert.equal(host.appearance, "classic");
  assert.deepEqual(host.osd, ["配色：经典"]);
  assert.deepEqual(checked(host), [false, true]);
  assert.equal(page(host).props.theme, "classic");
  assert.equal(page(host).props["can-reset"], true);
  // Choosing the current one again does nothing.
  await host.command("theme.classic").run();
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 1);
  assert.equal(host.osd.length, 1);
});

test("page: picking a palette and 恢复默认", async () => {
  const { host } = await start();
  page(host).emit("pick", "classic");
  await host.settle();
  assert.equal(host.appearance, "classic");
  assert.deepEqual(checked(host), [false, true]);
  page(host).emit("reset");
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual(host.osd, ["配色：经典", "配色：Vestra"]);
  assert.equal(page(host).props["can-reset"], false);
  // Already the default, or not a palette: nothing is sent.
  page(host).emit("reset");
  page(host).emit("pick", "dark");
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 2);
  assert.equal(host.osd.length, 2);
});

test("quick repeated clicks switch once", async () => {
  const { host } = await start();
  page(host).emit("pick", "classic");
  page(host).emit("pick", "classic");
  await host.command("theme.classic").run();
  await host.settle();
  assert.equal(host.sent("appearance.set").length, 1);
  assert.deepEqual(host.osd, ["配色：经典"]);
});

test("the saved palette shows at start; another plugin's switch is followed silently", async () => {
  const host = new TestHost();
  host.appearance = "classic";
  await start(host);
  assert.deepEqual(checked(host), [false, true]);
  assert.equal(page(host).props.theme, "classic");
  host.setAppearance("vestra");
  await host.settle();
  assert.deepEqual(checked(host), [true, false]);
  assert.equal(page(host).props.theme, "vestra");
  assert.deepEqual(host.osd, []);
});

test("a refused switch is reported and changes nothing", async () => {
  const { host } = await start();
  host.handle("appearance.set", () => {
    throw new HostError("unavailable", "没有界面");
  });
  page(host).emit("pick", "classic");
  await host.settle();
  assert.equal(host.appearance, "vestra");
  assert.deepEqual(checked(host), [true, false]);
  assert.deepEqual(host.osd, ["没有界面"]);
  // The next attempt is not blocked by the failed one.
  host.reset("appearance.set");
  await host.command("theme.classic").run();
  await host.settle();
  assert.equal(host.appearance, "classic");
});

test("stopping removes the page and the menu, the palette stays", async () => {
  const { host, stop } = await start();
  await host.command("theme.classic").run();
  await host.settle();
  await stop();
  assert.equal(host.settings.pages.size, 0);
  assert.equal(host.commands.size, 0);
  assert.equal(host.appearance, "classic");
});

test("titles and the page's Slint has no colour literals", () => {
  assert.equal(titleOf("classic"), "经典");
  assert.equal(THEMES.length, 2);
  const slint = readFileSync(new URL("../ui/appearance.slint", import.meta.url), "utf8");
  assert.doesNotMatch(slint, /#[0-9a-fA-F]{3,8}\b/);
});
