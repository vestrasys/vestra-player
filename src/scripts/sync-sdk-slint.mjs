#!/usr/bin/env node
/**
 * The SDK package (@vestra-player/sdk, repository vestra-player-sdk) carries
 * a read-only copy of the player's Slint library (`@vestra/theme.slint`,
 * `@vestra/widgets.slint`, `@vestra/markdown.slint`, `@vestra/icons.slint`,
 * `@vestra/bar-buttons.slint`, `@vestra/focus.slint` and the palette
 * they import) for plugin authors' IDEs. The player's host/ui/ is the source of the content; the copy lives in
 * the SDK repository's slint/ and ships in its releases.
 *
 *   node scripts/sync-sdk-slint.mjs                  check the installed SDK
 *                                                    (node_modules) against host/ui;
 *                                                    a difference only warns
 *   node scripts/sync-sdk-slint.mjs --strict         the same, but a difference
 *                                                    fails (also VESTRA_SDK_STRICT=1)
 *   node scripts/sync-sdk-slint.mjs --write <dir>    copy host/ui into a checkout
 *                                                    of the SDK repository (<dir>/slint)
 *
 * After changing the library: --write into the SDK repository; the copy
 * ships with the SDK's next release, then the dependency is upgraded here.
 * tests/sdk/host-agreement.test.ts runs the --strict check (an advisory test).
 *
 * The copy is product data (agentic-workflow.md §1.6): between SDK releases
 * it may lag behind host/ui, so the plain check warns and exits 0. When the
 * SDK dependency is upgraded and before a player release it must match
 * (--strict, `npm run ci:local -- --strict-sdk`, release.yml). A warning is
 * also appended to the file named by VESTRA_SDK_ADVISORY_FILE (ci:local).
 */
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { reportErrors, stop } from "../packaging/lib.mjs";

reportErrors();

const source = resolve(import.meta.dirname, "..");
/** Path under host/ui/ = path under the SDK's slint/. */
const SLINT_FILES = [
  "palette.slint",
  "vestra/theme.slint",
  "vestra/widgets.slint",
  "vestra/markdown.slint",
  "vestra/icons.slint",
  "vestra/bar-buttons.slint",
  "vestra/focus.slint",
];
const from = join(source, "host/ui");

const args = process.argv.slice(2);
const writeAt = args.indexOf("--write");
const write = writeAt >= 0;
const strict =
  args.includes("--strict") || process.env.VESTRA_SDK_STRICT === "1";
if (write && !args[writeAt + 1]) {
  console.error(
    "usage: node scripts/sync-sdk-slint.mjs [--strict | --write <sdk-repo>]",
  );
  stop(2);
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
  const message =
    `The installed SDK's Slint copy (${to}) differs from host/ui: ${drift.join(", ")}. ` +
    "Copy host/ui into the SDK repository (node scripts/sync-sdk-slint.mjs --write <sdk-repo>); " +
    "it ships with the next SDK release, then upgrade @vestra-player/sdk here.";
  if (strict) {
    console.error(message);
    process.exitCode = 1;
  } else {
    console.warn(
      `warning (product data; --strict or VESTRA_SDK_STRICT=1 fails): ${message}`,
    );
    const file = process.env.VESTRA_SDK_ADVISORY_FILE;
    if (file)
      await appendFile(
        file,
        `sdk:sync-slint: the installed SDK's Slint copy differs from host/ui: ${drift.join(", ")}
`,
      );
  }
} else
  console.log(
    !write
      ? "The installed SDK's Slint copy matches host/ui."
      : drift.length
        ? `Copied ${drift.join(", ")} into ${to}.`
        : `${to} already matches host/ui.`,
  );
