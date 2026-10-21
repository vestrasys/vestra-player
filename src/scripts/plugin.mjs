#!/usr/bin/env node
/**
 * Shared plugin tool: `node scripts/plugin.mjs <check|build|test> <name...|--all>`.
 * A name is a directory under plugins/; a name with a slash is a path relative
 * to src/ (for example `examples/bookmark`). `--all` covers plugins/ only.
 *
 *   check   strict type check of the plugin's .ts files (and its tests) with
 *           the repository compiler and tsconfig
 *   build   check, then write dist/: .ts → .js with types stripped, other
 *           package files copied, dist/manifest.json with `version`
 *   test    node --test on tests/*.test.{ts,mjs}
 *
 * Per-plugin options live in the plugin's package.json under "vestra":
 *   { "bundle": true | { "external": [...] }, "assets": ["vendor"] }
 * `bundle` bundles the entry with the plugin's own esbuild dependency;
 * `assets` copies extra files or directories verbatim. The version comes from
 * --version, else manifest.json, else package.json.
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import {
  cp,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import {
  basename,
  dirname,
  extname,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { pathToFileURL } from "node:url";
import ts from "typescript";

const source = resolve(import.meta.dirname, "..");
const pluginsRoot = join(source, "plugins");
const tsc = join(source, "node_modules/typescript/lib/tsc.js");

/** Manifest capabilities; the same list as the host's `plugins.rs`. */
const CAPABILITIES = new Set([
  "producer.av",
  "playback.read",
  "playback.control",
  "source.resolve",
  "events",
  "commands",
  "ui.window",
  "ui.mount",
  "ui.dialog",
  "plugins.manage",
  "processing.video",
  "processing.audio",
  "subtitles.read",
  "subtitles.control",
  "media.read",
  "media.control",
  "ui.osd",
  "clipboard",
  "window.control",
  "bindings.manage",
  "timeline.marks",
  "ui.drop",
  "ui.settings",
  "ui.theme",
]);

/** Never part of a production package. */
const SKIP_DIRS = new Set([
  "dist",
  "node_modules",
  "tests",
  "vendor",
  "evidence",
  ".build",
  ".typecheck",
]);
const SKIP_FILES = new Set([
  "manifest.json",
  "package.json",
  "package-lock.json",
]);
const SKIP_EXTENSIONS = new Set([".md", ".log"]);
const isTypeScript = (file) => file.endsWith(".ts") && !file.endsWith(".d.ts");
const isDeclaration = (file) => /\.d\.(m|c)?ts$/.test(file);

function fail(message) {
  throw Object.assign(new Error(message), { expected: true });
}

async function json(path, fallback) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT" && fallback !== undefined) return fallback;
    throw error;
  }
}

async function plugin(name) {
  const directory = name.includes("/")
    ? resolve(source, name)
    : join(pluginsRoot, name);
  if (!existsSync(join(directory, "manifest.json")))
    fail(`${name}: no manifest.json in ${directory}`);
  const manifest = await json(join(directory, "manifest.json"));
  const unknown = (manifest.capabilities ?? []).filter(
    (capability) => !CAPABILITIES.has(capability),
  );
  if (unknown.length)
    fail(`${name}: unknown capabilities ${unknown.join(", ")}`);
  const packageJson = await json(join(directory, "package.json"), {});
  return {
    name,
    directory,
    manifest,
    packageJson,
    options: packageJson.vestra ?? {},
  };
}

async function files(root, directory = root, keep = () => false) {
  const result = [];
  for (const item of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, item.name);
    const rel = relative(root, path);
    // By name, so a symlinked node_modules is skipped too.
    if ((item.name.startsWith(".") || SKIP_DIRS.has(item.name)) && !keep(rel))
      continue;
    if (item.isDirectory()) result.push(...(await files(root, path, keep)));
    else result.push(rel);
  }
  return result;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

/** npm dependencies are installed from the lock file when missing. */
function dependencies(target) {
  const deps = {
    ...target.packageJson.dependencies,
    ...target.packageJson.devDependencies,
  };
  if (
    !Object.keys(deps).length ||
    existsSync(join(target.directory, "node_modules"))
  )
    return;
  console.log(`${target.name}: installing dependencies (npm ci)`);
  if (
    run("npm", ["ci", "--no-audit", "--no-fund"], { cwd: target.directory }) !==
    0
  )
    fail(
      `${target.name}: npm ci failed; fix package-lock.json or install the dependencies`,
    );
}

async function check(target) {
  dependencies(target);
  const typescript = (await files(target.directory)).filter(
    (file) => isTypeScript(file) || file.endsWith(".mts"),
  );
  const tests = existsSync(join(target.directory, "tests"))
    ? (await readdir(join(target.directory, "tests")))
        .filter(isTypeScript)
        .map((file) => join("tests", file))
    : [];
  const sources = [...typescript, ...tests].map((file) =>
    join(target.directory, file),
  );
  if (!sources.length) {
    console.log(`${target.name}: no TypeScript to check`);
    return;
  }
  // A throwaway project extending the repository config, limited to this plugin.
  const scratch = await mkdtemp(join(tmpdir(), "vestra-plugin-check-"));
  try {
    const config = join(scratch, "tsconfig.json");
    await writeFile(
      config,
      JSON.stringify({
        extends: join(source, "tsconfig.json"),
        // Type roots resolve from the project directory, which is scratch here.
        compilerOptions: {
          noEmit: true,
          typeRoots: [join(source, "node_modules/@types")],
        },
        include: [],
        files: sources,
      }),
    );
    if (run(process.execPath, [tsc, "-p", config]) !== 0)
      fail(`${target.name}: type check failed`);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
  console.log(`${target.name}: type check passed (${sources.length} files)`);
}

function version(target, flag) {
  const value = flag ?? target.manifest.version ?? target.packageJson.version;
  if (!value)
    fail(`${target.name}: add "version" to manifest.json or pass --version`);
  if (
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value) ||
    value.split(".").some((part) => BigInt(part) > 4294967295n)
  )
    fail(
      `${target.name}: version ${value} needs three u32 components without leading zeros`,
    );
  return value;
}

/** The source of the entry: a dev manifest may point at main.ts or dist/main.js. */
function entries(target) {
  const entry = target.manifest.entry.replace(/^dist\//, "");
  const candidates = [entry.replace(/\.js$/, ".ts"), entry];
  const found = candidates.find((file) =>
    existsSync(join(target.directory, file)),
  );
  if (!found)
    fail(`${target.name}: entry ${target.manifest.entry} has no source file`);
  return { source: found, built: found.replace(/\.ts$/, ".js") };
}

function transpile(text, file) {
  const output = ts.transpileModule(text, {
    fileName: file,
    reportDiagnostics: true,
    compilerOptions: {
      target: ts.ScriptTarget.ES2023,
      module: ts.ModuleKind.ESNext,
      verbatimModuleSyntax: true,
      rewriteRelativeImportExtensions: true,
    },
  });
  if (output.diagnostics?.length)
    fail(
      `${file}: ${ts.flattenDiagnosticMessageText(output.diagnostics[0].messageText, "\n")}`,
    );
  return output.outputText;
}

/** Production code receives the SDK from the runner; it must not import it. */
function noRuntimeSdk(file, text) {
  const imports = [
    ...text.matchAll(/(?:from\s*|import\s*\(\s*|import\s+)["']([^"']+)["']/g),
  ].map((m) => m[1]);
  const offending = imports.find(
    (spec) => spec.startsWith(".") && spec.split("/").includes("sdk"),
  );
  if (offending)
    fail(
      `${file}: runtime import of the repository SDK (${offending}); use "import type"`,
    );
}

async function bundle(target, entry, out) {
  let esbuild;
  try {
    const require = createRequire(join(target.directory, "package.json"));
    esbuild = await import(pathToFileURL(require.resolve("esbuild")).href);
  } catch {
    fail(
      `${target.name}: "bundle" needs esbuild in the plugin's own dependencies`,
    );
  }
  const options =
    typeof target.options.bundle === "object" ? target.options.bundle : {};
  await esbuild.build({
    entryPoints: [join(target.directory, entry.source)],
    bundle: true,
    platform: "node",
    format: "esm",
    target: "node22",
    external: options.external ?? [],
    // Bundled CommonJS dependencies still call require().
    banner: {
      js: 'import { createRequire as __createRequire } from "node:module"; const require = __createRequire(import.meta.url);',
    },
    outfile: join(out, entry.built),
    logLevel: "warning",
  });
}

async function build(target, flag) {
  const release = version(target, flag);
  await check(target);
  const out = join(target.directory, "dist");
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });
  const entry = entries(target);
  const assets = target.options.assets ?? [];
  const listed = (rel) =>
    assets.some((asset) => rel === asset || rel.startsWith(asset + sep));
  const bundled = !!target.options.bundle;
  if (bundled) await bundle(target, entry, out);
  for (const file of await files(target.directory, target.directory, listed)) {
    const name = basename(file);
    const destination = join(out, file);
    if (!listed(file)) {
      if (
        SKIP_FILES.has(name) ||
        SKIP_EXTENSIONS.has(extname(name)) ||
        /^tsconfig.*\.json$/.test(name)
      )
        continue;
      if (isDeclaration(name)) continue;
      // Code reaches the bundle through the entry's imports.
      if (bundled && /\.(m|c)?[jt]s$/.test(name)) continue;
    }
    await mkdir(dirname(destination), { recursive: true });
    if (isTypeScript(file) && !listed(file)) {
      const text = transpile(
        await readFile(join(target.directory, file), "utf8"),
        file,
      );
      noRuntimeSdk(file, text);
      await writeFile(destination.replace(/\.ts$/, ".js"), text);
    } else await cp(join(target.directory, file), destination);
  }
  if (bundled)
    noRuntimeSdk(entry.built, await readFile(join(out, entry.built), "utf8"));
  await writeFile(join(out, "package.json"), '{"type":"module"}\n');
  await writeFile(
    join(out, "manifest.json"),
    `${JSON.stringify({ ...target.manifest, entry: entry.built, version: release }, null, 2)}\n`,
  );
  console.log(
    `${target.name}: built ${relative(source, out)} version ${release}`,
  );
}

async function test(target) {
  const directory = join(target.directory, "tests");
  const tests = existsSync(directory)
    ? (await readdir(directory))
        .filter((file) => /\.test\.(ts|mjs)$/.test(file))
        .sort()
    : [];
  if (!tests.length)
    fail(`${target.name}: no tests/*.test.ts or tests/*.test.mjs`);
  dependencies(target);
  if (
    run(
      process.execPath,
      ["--test", ...tests.map((file) => join(directory, file))],
      { cwd: target.directory },
    ) !== 0
  )
    fail(`${target.name}: tests failed`);
}

async function main() {
  const args = process.argv.slice(2);
  const command = args.shift();
  const flagIndex = args.indexOf("--version");
  const flag = flagIndex >= 0 ? args.splice(flagIndex, 2)[1] : undefined;
  if (!["check", "build", "test"].includes(command) || !args.length) {
    console.error(
      "usage: node scripts/plugin.mjs <check|build|test> <name...|--all> [--version x.y.z]",
    );
    process.exit(2);
  }
  const names = args.includes("--all")
    ? (await readdir(pluginsRoot, { withFileTypes: true }))
        .filter(
          (item) =>
            item.isDirectory() &&
            existsSync(join(pluginsRoot, item.name, "manifest.json")),
        )
        .map((item) => item.name)
        .sort()
    : args;
  if (flag && names.length > 1) fail("--version applies to a single plugin");
  const failed = [];
  for (const name of names) {
    try {
      const target = await plugin(name);
      if (command === "check") await check(target);
      else if (command === "build") await build(target, flag);
      else await test(target);
    } catch (error) {
      if (!error.expected) throw error;
      console.error(`✖ ${error.message}`);
      failed.push(name);
    }
  }
  if (failed.length) {
    console.error(`${command} failed for: ${failed.join(", ")}`);
    process.exit(1);
  }
}

await main();
