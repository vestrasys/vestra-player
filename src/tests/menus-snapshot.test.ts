// Menu snapshot: load each plugin into the test host and record its commands.
// Not part of `npm run test:sdk`; run it by hand:
//   OUT=<file> [PLUGINS=playback,playlist] node --test tests/menus-snapshot.test.ts
// Default: every built-in plugin in src/plugins (a directory with `main.ts`).
// `shortcut` is the first binding, or null, for a direct diff with old snapshots.
// `bindings` and `order` are the declaration fields.
// `host` lists the player's core actions (`host.coreCommands`).
// The test host is the SDK package installed in src/node_modules (`npm ci` in src/).
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

const src = new URL("../", import.meta.url);
const testing = createRequire(new URL("package.json", src)).resolve(
  "@vestra-player/sdk/testing",
);
const { startPlugin, TestHost } = await import(pathToFileURL(testing).href);
const plugins = new URL("plugins/", src);
const all = readdirSync(plugins, { withFileTypes: true })
  .filter(
    (entry) =>
      entry.isDirectory() &&
      existsSync(new URL(`${entry.name}/main.ts`, plugins)),
  )
  .map((entry) => entry.name)
  .sort();
const names = process.env.PLUGINS ? process.env.PLUGINS.split(",") : all;

type Entry = [string, { declaration: unknown }];
const row = ([id, c]: Entry) => {
  const d = c.declaration as {
    title: string;
    path: string[];
    shortcut?: string | null;
    bindings?: string[];
    order?: number;
  };
  const bindings = d.bindings ?? (d.shortcut ? [d.shortcut] : []);
  return {
    id,
    path: d.path,
    title: d.title ?? null,
    shortcut: bindings[0] ?? null,
    bindings,
    order: d.order ?? null,
  };
};

test("menus", async () => {
  const out = process.env.OUT;
  if (!out) throw new Error("set OUT=<file> for the snapshot");
  const snapshot: Record<string, unknown[]> = {};
  // The player's own commands (core actions, `host/<id>`): the same for every plugin, listed once.
  snapshot.host = [...(new TestHost().coreCommands?.entries() ?? [])].map(row);
  for (const name of names) {
    const dir = new URL(`${name}/`, plugins);
    const { default: activate } = await import(new URL("main.ts", dir).href);
    const host = new TestHost();
    const { stop } = await startPlugin(activate, { directory: dir, host });
    snapshot[name] = [...host.commands.entries()].map(row);
    await stop();
  }
  writeFileSync(out, JSON.stringify(snapshot, null, 2) + "\n");
});
