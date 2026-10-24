#!/usr/bin/env node
/**
 * Third-party license checks (packaging-proposal §5.3).
 *
 *   node packaging/check-licenses.mjs --static
 *       Repository only (runs in _static): about.toml `accepted` equals
 *       deny.toml [licenses].allow; third-party.toml parses and every
 *       `license_from` file exists; every plugin with run-time npm
 *       dependencies has a licenses/ directory; a plugin with a
 *       dependency-inventory.json or vendor/ has inventory / vendor components
 *       and every package they list has an allowed license and its license
 *       files; node.json's major equals the Node major CI tests with
 *       (.github/actions/setup-node).
 *   node packaging/check-licenses.mjs --bundle <root> --platform macos|windows
 *       A staged package (root = Vestra Player.app/Contents or the install
 *       directory): every file is ours or belongs to a component of
 *       third-party.toml / the shipped deps components.json; every license
 *       file exists; every license expression is allowed; every plugin
 *       package holds the licenses of what it bundles (inventory / vendor
 *       components are checked inside the .vxp); the shipped node is
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

// ------------------------------------------------------------ plugin packages

/** A plugin's files, from its source directory: { files, read(rel) }. */
export async function sourcePlugin(dir) {
  const files = await walk(dir);
  return {
    files,
    read: async (rel) =>
      exists(join(dir, rel)) ? readFile(join(dir, rel)) : undefined,
  };
}

/** A plugin's files, from its .vxp. */
export async function vxpPlugin(path) {
  const files = (await zipEntries(path)).filter((name) => !name.endsWith("/"));
  return { files, read: (rel) => zipRead(path, rel) };
}

/** A vendored package's license text, and the notices shipped beside it. */
const LICENSE_FILE = /^(licen[cs]e|copying)([.-].*)?$/i;
const NOTICE_FILE = /^(notice|third-?party-?notice)/i;

/**
 * The npm packages a component's `inventory` / `vendor` describe in a plugin
 * (see third-party.toml): [{ name, version, license, files }] with `files`
 * relative to the plugin, and the problems found while reading them. A
 * vendored package named in `bundled` (the plugin's inventories) is that
 * bundle and is not listed again.
 */
export async function bundledPackages(component, plugin, bundled = new Set()) {
  const packages = [];
  const problems = [];
  const label = component.name;
  const inventoryNames = new Set(bundled);
  if (component.inventory) {
    const bytes = await plugin.read(component.inventory);
    let inventory;
    try {
      inventory = bytes && JSON.parse(bytes.toString("utf8"));
    } catch (error) {
      problems.push(`${label}: ${component.inventory}: ${error.message}`);
    }
    if (!bytes) problems.push(`${label}: no ${component.inventory}`);
    else if (inventory && !Array.isArray(inventory))
      problems.push(`${label}: ${component.inventory} is not an array`);
    const base = component.inventory_licenses ?? "licenses";
    for (const entry of Array.isArray(inventory) ? inventory : []) {
      inventoryNames.add(entry.name);
      packages.push({
        name: entry.name,
        version: entry.version,
        license: entry.license,
        files: (entry.licenseFiles ?? []).map(
          (file) => `${base}/${String(entry.name).replace("/", "__")}/${file}`,
        ),
      });
    }
  }
  if (component.vendor) {
    const root = `${component.vendor}/`;
    const under = plugin.files.filter((file) => file.startsWith(root));
    const directories = new Set();
    for (const file of under) {
      const parts = file.slice(root.length).split("/");
      const depth = parts[0].startsWith("@") ? 2 : 1;
      if (parts.length === depth + 1 && parts[depth] === "package.json")
        directories.add(parts.slice(0, depth).join("/"));
    }
    if (!directories.size)
      problems.push(`${label}: no package under ${component.vendor}/`);
    for (const directory of [...directories].sort()) {
      const at = `${root}${directory}/`;
      let meta = {};
      try {
        meta = JSON.parse((await plugin.read(`${at}package.json`)).toString());
      } catch (error) {
        problems.push(`${label}: ${at}package.json: ${error.message}`);
      }
      // The bundle the inventory describes (vendor/pi is pi-coding-agent).
      if (inventoryNames.has(meta.name)) continue;
      if (meta.name !== directory)
        problems.push(
          `${label}: ${at}package.json names ${meta.name}, not ${directory}`,
        );
      const own = under
        .filter((file) => file.startsWith(at))
        .map((file) => file.slice(at.length))
        .filter((name) => !name.includes("/"));
      const licenses = own.filter((name) => LICENSE_FILE.test(name));
      packages.push({
        name: meta.name ?? directory,
        version: meta.version,
        license: typeof meta.license === "string" ? meta.license : undefined,
        // A notice alone is not a license text: such a package has no files.
        files: licenses.length
          ? [...licenses, ...own.filter((name) => NOTICE_FILE.test(name))].map(
              (name) => `${at}${name}`,
            )
          : [],
      });
    }
    for (const file of under)
      if (![...directories].some((dir) => file.startsWith(`${root}${dir}/`)))
        problems.push(`${label}: ${file} belongs to no vendored package`);
  }
  return { packages, problems };
}

/**
 * The packages of one plugin's `inventory` / `vendor` components:
 * [{ component, packages }] in config order, and the problems found reading
 * them. Inventories are read first: a vendor/ directory may hold the bundle
 * they describe.
 */
export async function pluginPackages(components, plugin) {
  const bundled = new Set();
  const found = new Map();
  const problems = [];
  const listed = components.filter((c) => c.inventory || c.vendor);
  for (const component of [...listed].sort(
    (a, b) => Boolean(b.inventory) - Boolean(a.inventory),
  )) {
    const result = await bundledPackages(component, plugin, bundled);
    if (component.inventory)
      for (const pkg of result.packages) bundled.add(pkg.name);
    found.set(component, result.packages);
    problems.push(...result.problems);
  }
  return {
    lists: listed.map((component) => ({
      component,
      packages: found.get(component),
    })),
    problems,
  };
}

/**
 * Checks the `inventory` / `vendor` components of one plugin against its
 * files: every package has an allowed license and its license files; nothing
 * under the license or vendor directories is unaccounted for.
 */
export async function checkPluginPackages(components, plugin, allowed, where) {
  const files = new Set(plugin.files);
  const claimed = new Set(components.flatMap((c) => c.vxp_license_files ?? []));
  const licenseRoots = new Set(
    components
      .filter((c) => c.inventory)
      .map((c) => `${c.inventory_licenses ?? "licenses"}/`),
  );
  const { lists, problems: found } = await pluginPackages(components, plugin);
  const problems = found.map((text) => `${where}: ${text}`);
  for (const { packages } of lists) {
    for (const pkg of packages) {
      const label = `${where}: ${pkg.name}@${pkg.version}`;
      if (!pkg.license) problems.push(`${label}: no license`);
      else
        try {
          if (!spdxAllowed(pkg.license, allowed))
            problems.push(`${label}: ${pkg.license} is not allowed`);
        } catch (error) {
          problems.push(`${label}: ${error.message}`);
        }
      if (!pkg.files.length) problems.push(`${label}: no license file`);
      for (const file of pkg.files) {
        claimed.add(file);
        if (!files.has(file)) problems.push(`${label}: missing ${file}`);
      }
    }
  }
  // License texts nobody claims are stale or undeclared (the source tree's
  // licenses/<package>/package.json are vendor-pi.mjs's own records).
  for (const file of plugin.files)
    if (
      [...licenseRoots].some((root) => file.startsWith(root)) &&
      !claimed.has(file) &&
      !file.endsWith("/package.json")
    )
      problems.push(`${where}: ${file} is in no component of third-party.toml`);
  return problems;
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
    const packages = component.inventory || component.vendor;
    if (packages && !component.vxp)
      problem(`${component.name}: inventory/vendor need vxp`);
    if (!component.deps && !packages && !component.license)
      problem(`${component.name}: no license`);
    if (component.license && !spdxAllowed(component.license, allowed))
      problem(`${component.name}: ${component.license} is not allowed`);
    if (
      !packages &&
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

  // A plugin with an inventory or a vendor/ directory is described by
  // inventory / vendor components, which its source satisfies.
  for (const [id, dir] of await pluginIdMap()) {
    const components = config.component.filter((c) => c.vxp === id);
    for (const [key, rel] of [
      ["inventory", "dependency-inventory.json"],
      ["vendor", "vendor"],
    ])
      if (
        exists(join(source, "plugins", dir, rel)) &&
        !components.some((c) => c[key] === rel)
      )
        problem(
          `plugins/${dir}/${rel}: no component with vxp = "${id}" and ${key} = "${rel}"`,
        );
    if (components.some((c) => c.inventory || c.vendor))
      problems.push(
        ...(await checkPluginPackages(
          components,
          await sourcePlugin(join(source, "plugins", dir)),
          allowed,
          `plugins/${dir}`,
        )),
      );
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
async function pluginIdMap() {
  await pluginDirectory();
  return pluginIds;
}
export async function pluginDirectory(id) {
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

  const packaged = new Map(); // .vxp → its components
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
        for (const file of component.vxp_license_files ?? [])
          if (!entries.has(file))
            problem(`${component.name}: ${vxp} has no ${file}`);
        packaged.set(vxp, [...(packaged.get(vxp) ?? []), component]);
      }
    }
    if (component.deps) await checkDeps(root, platform, licensesDir, allowed);
    if (component.version === "node.json") await checkNode(root, platform);
  }

  for (const [vxp, components] of packaged)
    if (components.some((c) => c.inventory || c.vendor))
      problems.push(
        ...(await checkPluginPackages(
          components,
          await vxpPlugin(join(root, vxp)),
          allowed,
          vxp,
        )),
      );

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
