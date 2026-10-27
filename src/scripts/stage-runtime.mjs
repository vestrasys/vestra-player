#!/usr/bin/env node
/**
 * Lays out the plugin runtime the way a packaged player ships it
 * (`Vestra Player.app/Contents/Resources/runtime/`, `<install dir>\resources\runtime\`
 * on Windows; found by host/src/runtime_paths.rs):
 *
 *   runtime/
 *     runner.js                              the own-process plugin runner
 *     shared-runner.js                       the runner for trusted plugins sharing one process
 *     boot.js                                one plugin's life, used by both runners
 *     package.json                           {"type":"module"}
 *     node_modules/@vestra-player/sdk/
 *       package.json                         exports map (runner and workshop resolve through it)
 *       LICENSE                              the SDK's license (Apache-2.0), shipped with its code
 *       dist/**.js, dist/**.d.ts             SDK implementation; the .d.ts serve the workshop
 *
 * The runners are `src/runtime/*.ts` with their types stripped here, so a
 * plugin process does not load Node's type stripper (Amaro, a WebAssembly
 * module: about 15 MB of memory per process, doc/process/rounds/2026-10-plugin-memory.md).
 * Stripping replaces the types with spaces, so line and column numbers in
 * stack traces are those of the source; relative imports of `./x.ts` become
 * `./x.js`. A development build runs the `.ts` files directly.
 *
 * The SDK files are copied from the installed package
 * (node_modules/@vestra-player/sdk, the version src/package.json pins). It
 * has no run-time dependencies (Node built-ins only), so nothing is installed.
 *
 *   node scripts/stage-runtime.mjs <destination>/runtime
 */
import { existsSync } from "node:fs";
import { cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire, stripTypeScriptTypes } from "node:module";
import { dirname, join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
const sdk = dirname(
  createRequire(join(source, "package.json")).resolve(
    "@vestra-player/sdk/package.json",
  ),
);

/** A relative module specifier ending in `.ts`, as written in an import. */
const RELATIVE_TS =
  /(\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)(["'])(\.\.?\/[^"']+)\.ts\2/g;

/**
 * `source` (TypeScript with erasable syntax only, as the tsconfig requires)
 * as JavaScript: Node's own stripper, the one that runs `.ts` at run time.
 */
export function stripTypes(text, file = "<source>") {
  // The API is marked experimental in Node 26; its warning says nothing a
  // packaging log needs.
  const emitWarning = process.emitWarning;
  process.emitWarning = (warning, ...rest) => {
    if (!String(warning).includes("stripTypeScriptTypes"))
      emitWarning.call(process, warning, ...rest);
  };
  let javascript;
  try {
    javascript = stripTypeScriptTypes(text, { mode: "strip" });
  } finally {
    process.emitWarning = emitWarning;
  }
  javascript = javascript.replace(
    RELATIVE_TS,
    (_, keyword, quote, path) => `${keyword}${quote}${path}.js${quote}`,
  );
  if (/(["'])\.\.?\/[^"']+\.ts\1/.test(javascript))
    throw new Error(`${file}: a relative .ts import was not rewritten`);
  return javascript;
}

export async function stageRuntime(destination) {
  if (!existsSync(join(sdk, "dist/player-runtime.js")))
    throw new Error(`SDK package without dist/ at ${sdk}: run npm ci`);
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  const runtime = join(source, "runtime");
  for (const name of await readdir(runtime)) {
    if (!name.endsWith(".ts") || name.endsWith(".d.ts")) continue;
    const file = join(runtime, name);
    await writeFile(
      join(destination, name.replace(/\.ts$/, ".js")),
      stripTypes(await readFile(file, "utf8"), file),
    );
  }
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
