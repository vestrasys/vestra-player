#!/usr/bin/env node
/**
 * Third-party license checks (packaging-proposal §5.3).
 *
 *   node packaging/check-licenses.mjs --static
 *       Repository only (runs in _static): about.toml `accepted` equals
 *       deny.toml [licenses].allow; third-party.toml parses and every
 *       `license_from` file exists; every plugin with run-time npm
 *       dependencies has a licenses/ directory; node.json's major equals the
 *       Node major CI tests with (.github/actions/setup-node).
 *   node packaging/check-licenses.mjs --bundle <root> --platform macos|windows
 *       A staged package (root = Vestra Player.app/Contents or the install
 *       directory): every file is ours or belongs to a component of
 *       third-party.toml / the shipped deps components.json; every license
 *       file exists; every license expression is allowed; every plugin
 *       package holds the licenses of what it bundles; the shipped node is
 *       the pinned version (when it can run here).
 *
 * Exit status 1 with every problem listed.
 */
import { spawnSync } from "node:child_process";
import { readdir, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  exists,
  fail,
  matchesAny,
  packaging,
  parseArgs,
  parseToml,
  readJson,
  repo,
  source,
  walk,
  zipEntries,
  zipRead,
} from "./lib.mjs";

const problems = [];
const problem = (message) => problems.push(message);

export async function loadThirdParty() {
  const text = await readFile(join(packaging, "third-party.toml"), "utf8");
  return parseToml(text, "third-party.toml");
}

/** Whether an SPDX expression is satisfied by `allowed` (OR: one side, AND: all). */
export function spdxAllowed(expression, allowed) {
  const tokens = expression.match(/\(|\)|[^\s()]+/g) ?? [];
  let position = 0;
  const peek = () => tokens[position];
  const factor = () => {
    if (peek() === "(") {
      position++;
      const value = or();
      if (tokens[position++] !== ")")
        throw new Error(`unbalanced: ${expression}`);
      return value;
    }
    let atom = tokens[position++];
    if (!atom) throw new Error(`incomplete: ${expression}`);
    if (peek() === "WITH") {
      position++;
      atom = `${atom} WITH ${tokens[position++]}`;
    }
    return allowed.has(atom);
  };
  const and = () => {
    let value = factor();
    while (peek() === "AND") {
      position++;
      value = factor() && value;
    }
    return value;
  };
  const or = () => {
    let value = and();
    while (peek() === "OR") {
      position++;
      value = and() || value;
    }
    return value;
  };
  const value = or();
  if (position !== tokens.length)
    throw new Error(`trailing tokens: ${expression}`);
  return value;
}

// ------------------------------------------------------------ static

async function checkStatic() {
  const deny = parseToml(
    await readFile(join(source, "host/deny.toml"), "utf8"),
    "deny.toml",
  );
  const about = parseToml(
    await readFile(join(packaging, "about.toml"), "utf8"),
    "about.toml",
  );
  const denyAllow = new Set(deny.licenses?.allow ?? []);
  const accepted = new Set(about.accepted ?? []);
  for (const license of denyAllow)
    if (!accepted.has(license))
      problem(`about.toml accepted lacks ${license} (deny.toml allows it)`);
  for (const license of accepted)
    if (!denyAllow.has(license))
      problem(`about.toml accepts ${license}, deny.toml does not allow it`);

  const config = await loadThirdParty();
  const allowed = new Set(config.policy?.allowed ?? []);
  for (const license of denyAllow)
    if (!allowed.has(license))
      problem(
        `third-party.toml [policy].allowed lacks ${license} (deny.toml allows it)`,
      );
  for (const component of config.component ?? []) {
    if (!component.name) problem("third-party.toml: a component without name");
    if (!component.deps && !component.license)
      problem(`${component.name}: no license`);
    if (component.license && !spdxAllowed(component.license, allowed))
      problem(`${component.name}: ${component.license} is not allowed`);
    if (
      !component.license_files?.length &&
      !component.vxp_license_files?.length
    )
      problem(`${component.name}: no license_files`);
    if (
      component.license_from &&
      component.license_from.length !== component.license_files?.length
    )
      problem(
        `${component.name}: license_from and license_files differ in length`,
      );
    for (const from of component.license_from ?? [])
      if (!exists(join(source, from)))
        problem(`${component.name}: ${from} does not exist (from src/)`);
    if (component.vxp) {
      const dir = await pluginDirectory(component.vxp);
      if (!dir)
        problem(`${component.name}: no plugin with id ${component.vxp}`);
      else
        for (const file of component.vxp_license_files ?? [])
          if (!exists(join(source, "plugins", dir, file)))
            problem(`${component.name}: plugins/${dir}/${file} does not exist`);
    }
  }

  // Every plugin that bundles npm packages ships their licenses.
  for (const dir of await readdir(join(source, "plugins"))) {
    const file = join(source, "plugins", dir, "package.json");
    if (!exists(file)) continue;
    const manifest = await readJson(file);
    if (
      Object.keys(manifest.dependencies ?? {}).length &&
      !exists(join(source, "plugins", dir, "licenses"))
    )
      problem(`plugins/${dir} has npm dependencies but no licenses/ directory`);
  }

  const node = await readJson(join(packaging, "node.json"));
  const action = await readFile(
    join(repo, ".github/actions/setup-node/action.yml"),
    "utf8",
  );
  const ciMajor = /node-version:\s*"?(\d+)/.exec(action)?.[1];
  if (node.version.split(".")[0] !== ciMajor)
    problem(
      `node.json pins Node ${node.version} but CI tests with Node ${ciMajor}`,
    );
}

let pluginIds;
async function pluginDirectory(id) {
  if (!pluginIds) {
    pluginIds = new Map();
    for (const dir of await readdir(join(source, "plugins"))) {
      const file = join(source, "plugins", dir, "manifest.json");
      if (exists(file)) pluginIds.set((await readJson(file)).id, dir);
    }
  }
  return pluginIds.get(id);
}

// ------------------------------------------------------------ bundle

async function checkBundle(root, platform) {
  const config = await loadThirdParty();
  const allowed = new Set(config.policy.allowed);
  const licensesDir = platform === "macos" ? "Resources/licenses" : "licenses";
  const pluginsDir =
    platform === "macos" ? "Resources/plugins" : "resources/plugins";
  const files = await walk(root);
  const owners = new Map(files.map((file) => [file, []]));
  const claim = (globs, owner) => {
    let matched = 0;
    for (const file of files)
      if (matchesAny(file, globs)) {
        owners.get(file).push(owner);
        matched++;
      }
    return matched;
  };
  claim(config.own[platform], "own");

  for (const component of config.component) {
    const globs = component[platform] ?? [];
    if (globs.length && !claim(globs, component.name) && !component.deps)
      problem(`${component.name}: no file matches ${globs.join(", ")}`);
    for (const file of component.license_files ?? [])
      if (!exists(join(root, licensesDir, file)))
        problem(`${component.name}: missing ${licensesDir}/${file}`);
    if (component.vxp) {
      const vxp = files.find((file) =>
        new RegExp(
          `^${pluginsDir}/${component.vxp.replace(".", "\\.")}-[^/]+\\.vxp$`,
        ).test(file),
      );
      if (!vxp)
        problem(`${component.name}: ${component.vxp} is not in ${pluginsDir}`);
      else {
        const entries = new Set(await zipEntries(join(root, vxp)));
        for (const file of component.vxp_license_files)
          if (!entries.has(file))
            problem(`${component.name}: ${vxp} has no ${file}`);
      }
    }
    if (component.deps) await checkDeps(root, platform, licensesDir, allowed);
    if (component.version === "node.json") await checkNode(root, platform);
  }

  // Every plugin package that bundles npm packages carries licenses/.
  for (const vxp of files.filter(
    (file) => file.startsWith(`${pluginsDir}/`) && file.endsWith(".vxp"),
  )) {
    const entries = await zipEntries(join(root, vxp));
    const manifest = await zipRead(join(root, vxp), "package.json");
    const dependencies = manifest
      ? Object.keys(JSON.parse(manifest).dependencies ?? {})
      : [];
    const id = vxp.slice(pluginsDir.length + 1).replace(/-[^-]+\.vxp$/, "");
    const dir = await pluginDirectory(id);
    const sourceDependencies = dir
      ? Object.keys(
          (await readJsonIfExists(join(source, "plugins", dir, "package.json")))
            ?.dependencies ?? {},
        )
      : [];
    if (
      (dependencies.length || sourceDependencies.length) &&
      !entries.some((entry) => entry.startsWith("licenses/"))
    )
      problem(
        `${vxp} bundles npm packages (${[...dependencies, ...sourceDependencies].join(", ")}) but has no licenses/`,
      );
  }

  for (const [file, list] of owners)
    if (!list.length)
      problem(`${file}: not ours and in no component of third-party.toml`);
}

async function readJsonIfExists(path) {
  return exists(path) ? readJson(path) : undefined;
}

async function checkDeps(root, platform, licensesDir, allowed) {
  const manifestPath = join(
    root,
    licensesDir,
    "third-party/deps/components.json",
  );
  if (!exists(manifestPath))
    return problem(`missing ${licensesDir}/third-party/deps/components.json`);
  const manifest = await readJson(manifestPath);
  const shipped = (file) => {
    if (platform === "windows") return file;
    if (file.startsWith("lib/")) return `Frameworks/${file.slice(4)}`;
    if (file.startsWith("bin/")) return `MacOS/${file.slice(4)}`;
    return file;
  };
  for (const component of manifest.components) {
    const label = `deps ${component.name} ${component.version}`;
    if (!component.license) problem(`${label}: no license`);
    else {
      try {
        if (!spdxAllowed(component.license, allowed))
          problem(`${label}: ${component.license} is not allowed`);
      } catch (error) {
        problem(`${label}: ${error.message}`);
      }
    }
    if (!component.license_files?.length) problem(`${label}: no license file`);
    for (const file of component.license_files ?? [])
      if (!exists(join(root, licensesDir, "third-party/deps", file)))
        problem(`${label}: missing ${licensesDir}/third-party/deps/${file}`);
    for (const file of component.files)
      if (!exists(join(root, shipped(file))) && !file.endsWith("libmpv.dylib"))
        problem(`${label}: ${shipped(file)} is listed but not shipped`);
  }
  const listed = new Set(
    manifest.components.flatMap((component) => component.files.map(shipped)),
  );
  const globs =
    platform === "macos"
      ? ["Frameworks/**", "MacOS/ffmpeg", "MacOS/ffprobe"]
      : ["*.dll", "ffmpeg/**"];
  for (const file of await walk(root))
    if (matchesAny(file, globs) && !listed.has(file))
      problem(`${file}: shipped but not in deps components.json`);
}

async function checkNode(root, platform) {
  const pinned = (await readJson(join(packaging, "node.json"))).version;
  const binary = join(
    root,
    platform === "macos" ? "MacOS/node" : "node/node.exe",
  );
  if (!exists(binary)) return problem(`${binary} is missing`);
  const runsHere =
    (platform === "macos") === (process.platform === "darwin") &&
    (platform === "windows") === (process.platform === "win32");
  if (!runsHere) return;
  const result = spawnSync(binary, ["--version"], { encoding: "utf8" });
  const version = result.stdout?.trim();
  if (version !== `v${pinned}`)
    problem(
      `shipped node is ${version || result.error?.message}, node.json pins v${pinned}`,
    );
}

// ------------------------------------------------------------ main

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const options = parseArgs(process.argv.slice(2), ["static"]);
  if (options.static) await checkStatic();
  else if (options.bundle && ["macos", "windows"].includes(options.platform))
    await checkBundle(resolve(options.bundle), options.platform);
  else
    fail(
      "usage: check-licenses.mjs --static | --bundle <root> --platform macos|windows",
    );
  if (problems.length) {
    for (const message of problems) console.error(`license check: ${message}`);
    console.error(`${problems.length} license problem(s)`);
    process.exit(1);
  }
  console.log("license check passed");
}
