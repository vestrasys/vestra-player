#!/usr/bin/env node
/**
 * The SDK package (@vestra-player/sdk, repository vestra-player-sdk) carries
 * a read-only copy of the player's Slint library (`@vestra/theme.slint`,
 * `@vestra/widgets.slint`, `@vestra/markdown.slint`, `@vestra/icons.slint`,
 * `@vestra/bar-buttons.slint` and the palette
 * they import) for plugin authors' IDEs. The player's host/ui/ is the source of the content; the copy lives in
 * the SDK repository's slint/ and ships in its releases.
 *
 *   node scripts/sync-sdk-slint.mjs                  check the installed SDK
 *                                                    (node_modules) against host/ui
 *   node scripts/sync-sdk-slint.mjs --write <dir>    copy host/ui into a checkout
 *                                                    of the SDK repository (<dir>/slint)
 *
 * After changing the library: --write into the SDK repository, release it,
 * then upgrade the dependency here; tests/sdk/host-agreement.test.ts runs the
 * check.
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
/** Path under host/ui/ = path under the SDK's slint/. */
const SLINT_FILES = [
  "palette.slint",
  "vestra/theme.slint",
  "vestra/widgets.slint",
  "vestra/markdown.slint",
  "vestra/icons.slint",
  "vestra/bar-buttons.slint",
];
const from = join(source, "host/ui");

const args = process.argv.slice(2);
const writeAt = args.indexOf("--write");
const write = writeAt >= 0;
if (write && !args[writeAt + 1]) {
  console.error("usage: node scripts/sync-sdk-slint.mjs [--write <sdk-repo>]");
  process.exit(2);
}
const to = write
  ? join(resolve(args[writeAt + 1]), "slint")
  : join(
      dirname(
        createRequire(join(source, "package.json")).resolve(
          "@vestra-player/sdk/package.json",
        ),
      ),
      "slint",
    );

const drift = [];
for (const file of SLINT_FILES) {
  const text = await readFile(join(from, file), "utf8");
  const target = join(to, file);
  const current = await readFile(target, "utf8").catch(() => null);
  if (current === text) continue;
  drift.push(file);
  if (write) {
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, text);
  }
}
if (!write && drift.length) {
  console.error(
    `The installed SDK's Slint copy (${to}) differs from host/ui: ${drift.join(", ")}. ` +
      "Copy host/ui into the SDK repository (node scripts/sync-sdk-slint.mjs --write <sdk-repo>), " +
      "release it and upgrade @vestra-player/sdk here.",
  );
  process.exit(1);
}
console.log(
  !write
    ? "The installed SDK's Slint copy matches host/ui."
    : drift.length
      ? `Copied ${drift.join(", ")} into ${to}.`
      : `${to} already matches host/ui.`,
);
