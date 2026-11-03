/**
 * Helpers shared by the packaging scripts (doc/process/ci.md, "Packaging").
 * Node built-ins only: the scripts run on CI runners before any npm install
 * and on Windows, macOS and Linux alike.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync } from "node:fs";
import {
  cp,
  mkdir,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { inflateRawSync } from "node:zlib";

export const packaging = import.meta.dirname;
export const source = resolve(packaging, "..");
export const repo = resolve(source, "..");

export function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

/** Parses `--name value` and `--flag` arguments; the rest are positional. */
export function parseArgs(argv, flags = []) {
  const options = { _: [] };
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    if (!arg.startsWith("--")) options._.push(arg);
    else if (flags.includes(arg.slice(2))) options[arg.slice(2)] = true;
    else {
      if (index + 1 >= argv.length) fail(`${arg} needs a value`);
      options[arg.slice(2)] = argv[++index];
    }
  }
  return options;
}

/** Runs a command, echoing it; throws on a non-zero exit. */
export function run(command, args, options = {}) {
  if (!options.quiet) console.log(`$ ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    stdio: options.capture ? ["ignore", "pipe", "inherit"] : "inherit",
    encoding: "utf8",
    maxBuffer: 1 << 28,
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !options.allowFailure)
    throw new Error(`${command} exited with ${result.status}`);
  return result;
}

export const capture = (command, args, options = {}) =>
  run(command, args, { quiet: true, capture: true, ...options }).stdout;

export async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

export const sha256 = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");

/** Downloads `url` to `path` (with retries); `headers` e.g. for a token. */
export async function download(url, path, headers = {}) {
  await mkdir(dirname(path), { recursive: true });
  let last;
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const response = await fetch(url, { headers, redirect: "follow" });
      if (!response.ok) throw new Error(`${response.status} ${url}`);
      await writeFile(path, Buffer.from(await response.arrayBuffer()));
      return path;
    } catch (error) {
      last = error;
      console.warn(`download attempt ${attempt} failed: ${error.message}`);
      await new Promise((done) => setTimeout(done, 2000 * attempt));
    }
  }
  throw last;
}

export async function verifySha256(path, expected) {
  const actual = await sha256File(path);
  if (actual !== expected)
    throw new Error(
      `SHA-256 mismatch for ${path}: expected ${expected}, got ${actual}`,
    );
}

export async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

/**
 * The host binary's version: `version` from src/host/Cargo.toml ([package]).
 * It goes up when the host input hash changes (host.mjs check-bump;
 * doc/releases/README.md), not for src/runtime, which is not in the binary.
 */
export async function hostVersion() {
  const text = await readFile(join(source, "host/Cargo.toml"), "utf8");
  const version = /^version\s*=\s*"([^"]+)"/m.exec(text)?.[1];
  if (!version) throw new Error("no version in src/host/Cargo.toml");
  return version;
}

/**
 * A version as the host's `version::parse` accepts it (src/host/src/version.rs
 * with `packages::manifest::parse_version`): at most 64 ASCII characters; up
 * to the first `-` or `+`, MAJOR.MINOR.PATCH of u32 numbers without leading
 * zeros (at most 32 characters); anything after it is kept as is. Returns
 * { core: [major, minor, patch], pre, build } or undefined. `pre` is the part
 * after a leading `-` up to `+`, `build` the part after the first `+`.
 */
export function parseVersion(version) {
  if (
    typeof version !== "string" ||
    version.length > 64 ||
    !/^[\x00-\x7f]*$/.test(version)
  )
    return undefined;
  const cut = version.search(/[-+]/);
  const core = cut < 0 ? version : version.slice(0, cut);
  const parts = core.split(".");
  if (
    core.length > 32 ||
    parts.length !== 3 ||
    !parts.every((part) => /^(0|[1-9][0-9]*)$/.test(part)) ||
    parts.some((part) => Number(part) > 0xffffffff)
  )
    return undefined;
  const rest = cut < 0 ? "" : version.slice(cut);
  const plus = rest.indexOf("+");
  const build = plus < 0 ? undefined : rest.slice(plus + 1);
  const pre = rest.startsWith("-")
    ? rest.slice(1, plus < 0 ? undefined : plus)
    : undefined;
  return { core: parts.map(Number), pre, build };
}

/**
 * The player's release version, `version` in src/release.json: the tag,
 * release notes, installer and archive names, latest.json and the installed
 * version shown to users. Always ≥ hostVersion().
 */
export async function releaseVersion() {
  const path = join(source, "release.json");
  const version = JSON.parse(await readFile(path, "utf8")).version;
  if (!parseVersion(version))
    throw new Error(
      `${path}: version ${JSON.stringify(version)} is not MAJOR.MINOR.PATCH[-pre][+build]`,
    );
  // It names files, the tag and release assets: no spaces, slashes, quotes.
  if (!/^[0-9A-Za-z.+-]+$/.test(version))
    throw new Error(
      `${path}: version ${JSON.stringify(version)} has characters a file name cannot carry`,
    );
  return version;
}

/**
 * Semantic version precedence (semver.org §11): negative, 0 or positive.
 * Build metadata (`+…`) does not count; a pre-release is lower than its
 * release; pre-release identifiers compare numerically when both are
 * numbers, otherwise as ASCII, and a longer list wins a tie.
 */
export function compareVersions(a, b) {
  const parse = (version) => {
    const parsed = parseVersion(version);
    if (!parsed)
      throw new Error(`${version} is not MAJOR.MINOR.PATCH[-pre][+build]`);
    return parsed;
  };
  const [x, y] = [parse(a), parse(b)];
  for (let index = 0; index < 3; index++)
    if (x.core[index] !== y.core[index]) return x.core[index] - y.core[index];
  if (x.pre === y.pre) return 0;
  if (x.pre === undefined) return 1;
  if (y.pre === undefined) return -1;
  const [p, q] = [x.pre.split("."), y.pre.split(".")];
  for (let index = 0; index < Math.min(p.length, q.length); index++) {
    if (p[index] === q[index]) continue;
    const [m, n] = [p[index], q[index]].map((id) =>
      /^[0-9]+$/.test(id) ? Number(id) : undefined,
    );
    if (m !== undefined && n !== undefined) return m - n;
    if (m !== undefined) return -1;
    if (n !== undefined) return 1;
    return p[index] < q[index] ? -1 : 1;
  }
  return p.length - q.length;
}

/** { release, host }, after checking release ≥ host. */
export async function versions() {
  const [release, host] = [await releaseVersion(), await hostVersion()];
  if (compareVersions(release, host) < 0)
    throw new Error(
      `src/release.json version ${release} is lower than the host's ${host} (src/host/Cargo.toml)`,
    );
  return { release, host };
}

/** Every file under `root` (relative, `/`-separated); symlinks as files. */
export async function walk(root, base = root) {
  const out = [];
  for (const item of await readdir(root, { withFileTypes: true })) {
    const path = join(root, item.name);
    if (item.isDirectory()) out.push(...(await walk(path, base)));
    else out.push(relative(base, path).split(sep).join("/"));
  }
  return out.sort();
}

export async function directorySize(root) {
  let total = 0;
  for (const file of await walk(root))
    total += (await stat(join(root, file))).size;
  return total;
}

export const megabytes = (bytes) => `${(bytes / 1e6).toFixed(1)} MB`;

export async function clean(path) {
  await rm(path, { recursive: true, force: true });
  await mkdir(path, { recursive: true });
}

/**
 * Glob → RegExp for `/`-separated relative paths: `**` any depth
 * (including none), `*` within one segment, `?` one character.
 */
export function globToRegExp(glob) {
  let pattern = "";
  for (let index = 0; index < glob.length; index++) {
    const char = glob[index];
    if (char === "*" && glob[index + 1] === "*") {
      index++;
      if (glob[index + 1] === "/") {
        index++;
        pattern += "(?:.*/)?";
      } else pattern += ".*";
    } else if (char === "*") pattern += "[^/]*";
    else if (char === "?") pattern += "[^/]";
    else pattern += char.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`^${pattern}$`);
}

export const matchesAny = (path, globs) =>
  globs.some((glob) => globToRegExp(glob).test(path));

/** Central directory of a zip archive (no zip64): name, method, sizes, offset. */
export async function zipDirectory(path) {
  const bytes = await readFile(path);
  let end = bytes.length - 22;
  while (end >= 0 && bytes.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error(`${path}: not a zip archive`);
  const count = bytes.readUInt16LE(end + 10);
  let offset = bytes.readUInt32LE(end + 16);
  const entries = [];
  for (let index = 0; index < count; index++) {
    if (bytes.readUInt32LE(offset) !== 0x02014b50)
      throw new Error(`${path}: bad central directory`);
    const nameLength = bytes.readUInt16LE(offset + 28);
    const extra = bytes.readUInt16LE(offset + 30);
    const comment = bytes.readUInt16LE(offset + 32);
    entries.push({
      name: bytes.toString("utf8", offset + 46, offset + 46 + nameLength),
      method: bytes.readUInt16LE(offset + 10),
      compressed: bytes.readUInt32LE(offset + 20),
      header: bytes.readUInt32LE(offset + 42),
    });
    offset += 46 + nameLength + extra + comment;
  }
  return { bytes, entries };
}

export async function zipEntries(path) {
  return (await zipDirectory(path)).entries.map((entry) => entry.name);
}

/** One entry's bytes from a zip archive (stored or deflate), else undefined. */
export async function zipRead(path, name) {
  const { bytes, entries } = await zipDirectory(path);
  const entry = entries.find((item) => item.name === name);
  if (!entry) return undefined;
  const local = entry.header;
  const start =
    local +
    30 +
    bytes.readUInt16LE(local + 26) +
    bytes.readUInt16LE(local + 28);
  const data = bytes.subarray(start, start + entry.compressed);
  if (entry.method === 0) return data;
  if (entry.method === 8) return inflateRawSync(data);
  throw new Error(`${path}: ${name} uses compression method ${entry.method}`);
}

/**
 * A small TOML reader for the packaging configuration and deny.toml:
 * comments, `[table]`, `[[array]]`, dotted table names, and values that are
 * strings ("basic", 'literal'), integers, booleans, or (multi-line) arrays
 * of those. Anything else throws, so a config edit cannot be misread.
 */
export function parseToml(text, file = "toml") {
  const root = {};
  let table = root;
  const lines = text.split(/\r?\n/);
  const path = (name) =>
    name.split(".").map((part) => part.trim().replace(/^"|"$/g, ""));
  const descend = (keys, create) => {
    let node = root;
    for (const key of keys) {
      node[key] ??= {};
      node = Array.isArray(node[key]) ? node[key].at(-1) : node[key];
    }
    return create(node);
  };
  for (let index = 0; index < lines.length; index++) {
    let line = stripComment(lines[index]).trim();
    if (!line) continue;
    const where = `${file}:${index + 1}`;
    let match;
    if ((match = /^\[\[([^\]]+)\]\]$/.exec(line))) {
      const keys = path(match[1]);
      const last = keys.pop();
      table = descend(keys, (node) => {
        node[last] ??= [];
        if (!Array.isArray(node[last]))
          throw new Error(`${where}: ${last} is not an array`);
        const item = {};
        node[last].push(item);
        return item;
      });
      continue;
    }
    if ((match = /^\[([^\]]+)\]$/.exec(line))) {
      table = descend(path(match[1]), (node) => node);
      continue;
    }
    match = /^([A-Za-z0-9_\-."]+)\s*=\s*(.*)$/.exec(line);
    if (!match)
      throw new Error(`${where}: cannot parse ${JSON.stringify(line)}`);
    let value = match[2];
    if (value.startsWith("[")) {
      while (!balanced(value)) {
        if (++index >= lines.length)
          throw new Error(`${where}: unterminated array`);
        value += ` ${stripComment(lines[index]).trim()}`;
      }
    }
    const keys = path(match[1]);
    const last = keys.pop();
    let target = table;
    for (const key of keys) target = target[key] ??= {};
    if (last in target) throw new Error(`${where}: duplicate key ${last}`);
    target[last] = parseValue(value.trim(), where);
  }
  return root;
}

function stripComment(line) {
  let quote = null;
  for (let index = 0; index < line.length; index++) {
    const char = line[index];
    if (quote) {
      if (char === "\\" && quote === '"') index++;
      else if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "#") return line.slice(0, index);
  }
  return line;
}

function balanced(value) {
  let depth = 0;
  let quote = null;
  for (let index = 0; index < value.length; index++) {
    const char = value[index];
    if (quote) {
      if (char === "\\" && quote === '"') index++;
      else if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "[") depth++;
    else if (char === "]") depth--;
  }
  return depth === 0;
}

function parseValue(value, where) {
  if (value.startsWith('"') && value.endsWith('"') && value.length >= 2)
    return JSON.parse(value);
  if (value.startsWith("'") && value.endsWith("'") && value.length >= 2)
    return value.slice(1, -1);
  if (value === "true" || value === "false") return value === "true";
  if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value);
  if (value.startsWith("[") && value.endsWith("]")) {
    const items = [];
    let rest = value.slice(1, -1).trim();
    while (rest) {
      const match =
        /^("(?:[^"\\]|\\.)*"|'[^']*'|-?\d+|true|false)\s*(,\s*)?/.exec(rest);
      if (!match)
        throw new Error(`${where}: unsupported array item in ${value}`);
      items.push(parseValue(match[1], where));
      rest = rest.slice(match[0].length).trim();
    }
    return items;
  }
  throw new Error(`${where}: unsupported value ${value}`);
}

/** Copies a file or directory (cp -R semantics, keeps symlinks). */
export async function copy(from, to) {
  await mkdir(dirname(to), { recursive: true });
  await cp(from, to, { recursive: true, verbatimSymlinks: true });
}

export const exists = existsSync;
export const toPosix = (path) => path.split(sep).join(posix.sep);
