/**
 * "How to open" (Store v3; owner ruling 2026-11-15: a convention in the
 * description, no manifest field): a last line starting with "→ ", at most
 * 48 characters after it, goes to the card and leaves the description; an
 * installed plugin without one gets its menu entry or palette row from
 * `commands.list`. And the Category drop-down's fixed goals.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { TestHost } from "@vestra-player/sdk/testing";
import { commandLine, goalsOf, OPEN_LIMIT, splitDescription } from "../open-line.ts";
import { msg } from "../text.ts";
import { buildIndex, BUILTIN, choose, FakeNet, info, OFFICIAL, openStore, props, rows, showBuiltins, signWith, start, until } from "./helpers.ts";

test("the description's last line with the marker is the line; anything else leaves it", () => {
  assert.deepEqual(splitDescription("Saves the frame.\n→ Menu: Tools ▸ Snapshot"), {
    description: "Saves the frame.",
    open: "Menu: Tools ▸ Snapshot",
  });
  // Trailing blank lines and spaces around the text do not count; a description that is only the line is none.
  assert.deepEqual(splitDescription("Text.\n→   Sidebar: Playlist tab (F6)  \n\n"), { description: "Text.", open: "Sidebar: Playlist tab (F6)" });
  assert.deepEqual(splitDescription("→ 菜单：工具 ▸ 截图"), { description: null, open: "菜单：工具 ▸ 截图" });
  // Not the last line, no space after the arrow, an arrow inside a sentence, nothing after it, too long.
  for (const text of [
    "→ Menu: Tools\nMore text.",
    "Text.\n→Menu: Tools",
    "Text. → Menu: Tools",
    "Text.\n→ ",
    `Text.\n→ ${"x".repeat(OPEN_LIMIT + 1)}`,
    "Right-click menu: Tools ▸ Snapshot ▸.",
  ])
    assert.deepEqual(splitDescription(text), { description: text, open: null }, text);
  assert.equal(splitDescription(`Text.\n→ ${"界".repeat(OPEN_LIMIT)}`).open, "界".repeat(OPEN_LIMIT), "characters, not bytes");
  assert.deepEqual(splitDescription(null), { description: null, open: null });
});

test("the fallback: the plugin's own shortest menu path, else its palette row, else nothing", () => {
  const rows = [
    { id: "x.cap/burst", path: ["工具", "截图", "连拍"], label: "截图：连拍" },
    { id: "x.cap/save", path: ["工具", "截图…"], label: "截图：保存…" },
    { id: "x.capture/other", path: ["打开"], label: "打开：别的" },
    { id: "x.win/top", path: [], label: "窗口：总在最前" },
    { id: "x.win/size", path: [], label: "窗口大小：一倍…" },
  ];
  assert.deepEqual(commandLine(rows, "x.cap"), msg("Menu: {}", "工具 ▸ 截图"));
  assert.deepEqual(commandLine(rows, "x.win"), msg("Command palette: {}", "窗口：总在最前"));
  assert.equal(commandLine(rows, "x.none"), null);
});

test("categories: a fixed list matched by tags in any case; nothing known is Other", () => {
  assert.deepEqual(goalsOf(["Video", "online"]), ["Picture", "Online video"]);
  assert.deepEqual(goalsOf(["subtitles", "download"]), ["Subtitles"]);
  assert.deepEqual(goalsOf(["AI", "Plugin development"]), ["Tools"]);
  assert.deepEqual(goalsOf([]), ["Other"]);
  assert.deepEqual(goalsOf(["core"]), ["Other"]);
});

test("cards and pages show the line: from the index's description before install, from the commands after", async () => {
  const net = new FakeNet();
  // The repository copies each package's description (and its translations) into the index.
  const index = JSON.parse(buildIndex(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]).toString("utf8"));
  index.plugins[0].description = "Does things.\n→ Menu: Tools ▸ Alpha";
  index.plugins[0].translations = { "zh-Hans": { description: "做事。\n→ 菜单：工具 ▸ Alpha" } };
  const body = Buffer.from(JSON.stringify(index));
  net.route(OFFICIAL, { body });
  net.route(`${OFFICIAL}.sig`, { body: `${signWith(body)}\n` });
  const host = new TestHost();
  // A built-in plugin: the player answers commands.list.
  host.builtin = true;
  const store = info("vestra.store", { version: "0.3.0", builtin: true, source: { kind: "bundled" }, directory: "" });
  const started = await start({ net, host, packages: [BUILTIN, store] });
  try {
    await openStore(started.host);
    await showBuiltins(started.host);
    await until(started.host, () => rows(started.host).some((r) => r.name === "Alpha" && r.open !== ""), "line");
    assert.equal(rows(started.host).find((r) => r.name === "Alpha")!.open, "菜单：工具 ▸ Alpha");
    // No description at all and no commands: the line stays empty.
    assert.equal(rows(started.host).find((r) => r.name === "vestra.video")!.open, "");
    // The store itself (no manifest here): its own menu entry, Tools ▸ Plugin Store…, without the "…".
    await until(started.host, () => rows(started.host).find((r) => r.key === "\nvestra.store")?.open !== "", "fallback");
    assert.equal(rows(started.host).find((r) => r.key === "\nvestra.store")!.open, "菜单：工具 ▸ 插件商城");
    await choose(started.host, "Alpha");
    assert.equal(props(started.host)["d-open"], "菜单：工具 ▸ Alpha");
  } finally {
    await started.stop();
  }
});
