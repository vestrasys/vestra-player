#!/usr/bin/env node
/**
 * Copies the player's Slint library (`@vestra/theme.slint`, `@vestra/widgets.slint`
 * and the palette they import) into the SDK package, read-only for plugin
 * authors' IDEs. The player is the source; at run time it supplies its own
 * copy. `--check` only reports drift (tests/sdk/slint-copy.test.ts).
 *
 *   node scripts/sync-sdk-slint.mjs [--check]
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
/** Path under host/ui/ = path under packages/sdk/slint/. */
const SLINT_FILES = [
  "palette.slint",
  "vestra/theme.slint",
  "vestra/widgets.slint",
];
const from = join(source, "host/ui");
const to = join(source, "packages/sdk/slint");

const check = process.argv.includes("--check");
const drift = [];
for (const file of SLINT_FILES) {
  const text = await readFile(join(from, file), "utf8");
  const target = join(to, file);
  const current = await readFile(target, "utf8").catch(() => null);
  if (current === text) continue;
  drift.push(file);
  if (!check) {
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, text);
  }
}
if (check && drift.length) {
  console.error(
    `SDK Slint copy differs from host/ui: ${drift.join(", ")}; run node scripts/sync-sdk-slint.mjs`,
  );
  process.exit(1);
}
console.log(
  check
    ? "SDK Slint copy matches host/ui."
    : drift.length
      ? `Copied ${drift.join(", ")} into packages/sdk/slint.`
      : "SDK Slint copy already up to date.",
);
