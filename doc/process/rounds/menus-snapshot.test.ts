// Menu snapshot: load each plugin into the test host and record its commands.
//   OUT=<file> [PLUGINS=playback,playlist] node --test doc/process/rounds/menus-snapshot.test.ts
// `shortcut` keeps the P0 field for a direct diff: the first binding, or null.
// `bindings` and `order` are the new declaration fields (absent before migration).
// `host` lists the player's core actions (`host.coreCommands`, round 2 P3 on).
import { startPlugin, TestHost } from "/Users/ziyang/Develop/vestra-player/src/sdk/plugin/testing.ts";
import { writeFileSync } from "node:fs";
import { test } from "node:test";
const all = ["info", "playback","playlist","video","audio","subtitles","capture","nes","youtube","bilibili","workshop"];
const names = process.env.PLUGINS ? process.env.PLUGINS.split(",") : all;
type Entry = [string, { declaration: unknown }];
const row = ([id, c]: Entry) => {
  const d = c.declaration as { title: string; path: string[]; shortcut?: string | null; bindings?: string[]; order?: number };
  const bindings = d.bindings ?? (d.shortcut ? [d.shortcut] : []);
  return { id, path: d.path, title: d.title ?? null, shortcut: bindings[0] ?? null, bindings, order: d.order ?? null };
};
test("menus", async () => {
  const out: Record<string, unknown[]> = {};
  // The player's own commands (core actions, `host/<id>`): the same for every plugin, listed once.
  out.host = [...(new TestHost().coreCommands?.entries() ?? [])].map(row);
  for (const name of names) {
    const dir = new URL(`file:///Users/ziyang/Develop/vestra-player/src/plugins/${name}/`);
    const { default: activate } = await import(new URL("main.ts", dir).href);
    const host = new TestHost();
    const { stop } = await startPlugin(activate, { directory: dir, host });
    out[name] = [...host.commands.entries()].map(row);
    await stop();
  }
  writeFileSync(process.env.OUT!, JSON.stringify(out, null, 2) + "\n");
});
