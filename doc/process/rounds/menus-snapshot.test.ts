import { startPlugin, TestHost } from "/Users/ziyang/Develop/vestra-player/src/sdk/plugin/testing.ts";
import { writeFileSync } from "node:fs";
import { test } from "node:test";
const names = ["playback","playlist","video","audio","subtitles","capture","nes","youtube","bilibili","workshop"];
test("menus", async () => {
  const out: Record<string, unknown[]> = {};
  for (const name of names) {
    const dir = new URL(`file:///Users/ziyang/Develop/vestra-player/src/plugins/${name}/`);
    const { default: activate } = await import(new URL("main.ts", dir).href);
    const host = new TestHost();
    const { stop } = await startPlugin(activate, { directory: dir, host });
    out[name] = [...host.commands.entries()].map(([id, c]) => ({ id, path: c.declaration.path, title: c.declaration.title ?? null, shortcut: c.declaration.shortcut ?? null }));
    await stop();
  }
  writeFileSync(process.env.OUT!, JSON.stringify(out, null, 2) + "\n");
});
