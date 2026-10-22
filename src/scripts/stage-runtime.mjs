#!/usr/bin/env node
/**
 * Lays out the plugin runtime the way a packaged player ships it
 * (`Vestra Player.app/Contents/Resources/runtime/`, `<install dir>\resources\runtime\`
 * on Windows; found by host/src/runtime_paths.rs):
 *
 *   runtime/
 *     runner.ts                              the plugin runner (Node strips its types)
 *     package.json                           {"type":"module"}
 *     node_modules/@vestra-player/sdk/
 *       package.json                         exports map (runner and workshop resolve through it)
       LICENSE                              the SDK's license (Apache-2.0), shipped with its code
 *       dist/**.js, dist/**.d.ts             SDK implementation; the .d.ts serve the workshop
 *
 * The SDK files are copied from the installed package
 * (node_modules/@vestra-player/sdk, the version src/package.json pins). It
 * has no run-time dependencies (Node built-ins only), so nothing is installed.
 *
 *   node scripts/stage-runtime.mjs <destination>/runtime
 */
import { existsSync } from "node:fs";
import { cp, mkdir, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
const sdk = dirname(
  createRequire(join(source, "package.json")).resolve(
    "@vestra-player/sdk/package.json",
  ),
);

export async function stageRuntime(destination) {
  if (!existsSync(join(sdk, "dist/player-runtime.js")))
    throw new Error(`SDK package without dist/ at ${sdk}: run npm ci`);
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  await cp(join(source, "runtime/runner.ts"), join(destination, "runner.ts"));
  await writeFile(
    join(destination, "package.json"),
    '{"type":"module","private":true}\n',
  );
  const target = join(destination, "node_modules/@vestra-player/sdk");
  await mkdir(target, { recursive: true });
  await cp(join(sdk, "package.json"), join(target, "package.json"));
  await cp(join(sdk, "LICENSE"), join(target, "LICENSE"));
  await cp(join(sdk, "dist"), join(target, "dist"), { recursive: true });
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const destination = process.argv[2];
  if (!destination) {
    console.error("usage: node scripts/stage-runtime.mjs <destination>");
    process.exit(2);
  }
  await stageRuntime(resolve(destination));
  console.log(`Plugin runtime staged in ${resolve(destination)}`);
}
