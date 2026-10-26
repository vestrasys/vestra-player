#!/usr/bin/env node
/**
 * A built-in plugin whose shipped files changed must ship under a higher
 * version. The host upgrades a seeded built-in only when the installation
 * carries a newer version than it last seeded (host/src/package_manager.rs
 * `seed::decide`, doc/architecture/store-api-proposal.md §3.3), so a
 * change shipped under the same version never reaches existing users (#27:
 * workshop's ai.slint changed under 2.2.3, and players that had run 0.5.0
 * kept the broken window).
 *
 *   node scripts/check-builtin-versions.mjs [--base <ref>] [--head <ref>]
 *
 *   --base  what users have: by default the latest release tag (`v*`)
 *           reachable from HEAD; `origin/main` checks against the base
 *           branch instead
 *   --head  a commit to check; by default the working tree (tracked files,
 *           staged or not, and untracked files that are not ignored)
 *
 * Built in: every src/plugins/<name> with a manifest.json (what
 * stage-plugins.mjs packs). A plugin new since the base needs nothing.
 *
 * Shipped: the files `vestra-plugin pack` puts into the .vxp, per the SDK's
 * build (@vestra-player/sdk bin/vestra-plugin.mjs `build`), read from the
 * plugin's sources:
 * - not shipped: anything under a directory or with a name that starts with
 *   `.` or is `dist`, `node_modules`, `tests`, `vendor`, `evidence`,
 *   `.build`, `.typecheck`; `tsconfig*.json`; `.d.ts` / `.d.mts` /
 *   `.d.cts`; `.md`, `.log`, `.zip`, `.vxp`, `.pot` files;
 * - except what is listed, which ships whatever its name: the
 *   `vestra.assets` of package.json (workshop's `vendor`), and the icon,
 *   README and changelog (the manifest's `icon` / `readme` / `changelog`,
 *   else README.md / CHANGELOG.md when present);
 * - `locales/<lang>.po` ship compiled (dist/locales/<lang>.json), so they count;
 *   a `.po` anywhere else is not packed;
 * - manifest.json counts for every field but `version` (pack rewrites
 *   `entry` and `version`, the rest ships as written);
 * - package.json counts for its `vestra` options (bundle, assets) and, for
 *   a bundled plugin (`vestra.bundle`), its `dependencies`; then
 *   package-lock.json counts too (it pins what the bundle contains).
 *   Scripts, devDependencies and its own `version` do not.
 * A path listed on either side (base or head) counts.
 *
 * Exit 0 when every changed built-in has a higher version; 1 with the list
 * otherwise; 2 when git cannot answer (for example a shallow clone without
 * the base tag: fetch with full history, `fetch-depth: 0`).
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

const PLUGINS = "src/plugins";
/** The SDK's `SKIP_DIRS`: never part of a production package. */
const SKIP_DIRS = new Set([
  "dist",
  "node_modules",
  "tests",
  "vendor",
  "evidence",
  ".build",
  ".typecheck",
]);
/** The SDK's `SKIP_EXTENSIONS` without `.po` (compiled into the package). */
const SKIP_EXTENSIONS = [".md", ".log", ".zip", ".vxp", ".pot"];

/** `x.y.z` as the host reads it (manifest.rs `parse_version`), else null. */
export function parseVersion(value) {
  if (typeof value !== "string") return null;
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value);
  return match ? match.slice(1).map(Number) : null;
}

/** Whether `head` is a higher version than `base`; false when either is invalid. */
export function isHigher(head, base) {
  const a = parseVersion(head);
  const b = parseVersion(base);
  if (!a || !b) return false;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

/**
 * The paths (relative to the plugin, `/`-separated) the SDK copies whatever
 * their name, for one side: package.json `vestra.assets` and the display
 * files. `present(rel)` says whether a file exists on that side.
 */
export function listedOf(manifest, packageJson, present) {
  const assets = Array.isArray(packageJson?.vestra?.assets)
    ? packageJson.vestra.assets.filter((a) => typeof a === "string")
    : [];
  const display = new Set();
  for (const field of ["icon", "readme", "changelog"])
    if (typeof manifest?.[field] === "string") display.add(manifest[field]);
  for (const [name, field] of [
    ["README.md", "readme"],
    ["CHANGELOG.md", "changelog"],
  ])
    if (manifest?.[field] === undefined && present(name)) display.add(name);
  return (rel) =>
    display.has(rel) ||
    assets.some((asset) => rel === asset || rel.startsWith(`${asset}/`));
}

/**
 * Whether a plugin file (relative, `/`-separated) is shipped, by name only;
 * manifest.json, package.json and package-lock.json are compared by content
 * (`shippedChanges`) and are not shipped as files here.
 */
export function isShippedFile(rel, listed) {
  if (listed(rel)) return true;
  const parts = rel.split("/");
  for (let i = 0; i < parts.length; i++) {
    const name = parts[i];
    if (
      (name.startsWith(".") || SKIP_DIRS.has(name)) &&
      !listed(parts.slice(0, i + 1).join("/"))
    )
      return false;
  }
  const name = parts.at(-1);
  // The SDK's `SKIP_FILES`, at any depth.
  if (["manifest.json", "package.json", "package-lock.json"].includes(name))
    return false;
  if (/^tsconfig.*\.json$/.test(name)) return false;
  if (/\.d\.(m|c)?ts$/.test(name)) return false;
  if (SKIP_EXTENSIONS.some((extension) => name.endsWith(extension)))
    return false;
  // The SDK compiles locales/<lang>.po; any other .po is not packed.
  if (name.endsWith(".po")) return parts.length === 2 && parts[0] === "locales";
  return true;
}

const withoutVersion = (manifest) => {
  if (!manifest || typeof manifest !== "object") return manifest;
  const { version: _version, ...rest } = manifest;
  return rest;
};
const packOptions = (packageJson) => ({
  vestra: packageJson?.vestra ?? null,
  dependencies: packageJson?.vestra?.bundle
    ? (packageJson?.dependencies ?? null)
    : null,
});
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The shipped differences of one plugin between base and head.
 * `changed`: plugin-relative paths whose bytes differ (git diff);
 * `base` / `head`: { manifest, packageJson, present(rel) }.
 */
export function shippedChanges(changed, base, head) {
  const listedBase = listedOf(base.manifest, base.packageJson, base.present);
  const listedHead = listedOf(head.manifest, head.packageJson, head.present);
  const listed = (rel) => listedBase(rel) || listedHead(rel);
  const result = changed.filter((rel) => isShippedFile(rel, listed));
  if (!same(withoutVersion(base.manifest), withoutVersion(head.manifest)))
    result.push("manifest.json (other than version)");
  if (!same(packOptions(base.packageJson), packOptions(head.packageJson)))
    result.push("package.json (vestra options or bundled dependencies)");
  if (
    changed.includes("package-lock.json") &&
    (base.packageJson?.vestra?.bundle || head.packageJson?.vestra?.bundle)
  )
    result.push("package-lock.json (bundled plugin)");
  return result.sort();
}

/**
 * The verdict for one plugin: null when it is fine, else a message.
 * A plugin without a base manifest is new and needs no bump.
 */
export function verdict(name, changed, base, head) {
  if (!base.manifest) return null;
  const shipped = shippedChanges(changed, base, head);
  if (!shipped.length) return null;
  const from = base.manifest.version;
  const to = head.manifest?.version;
  if (isHigher(to, from)) return null;
  const list = shipped.slice(0, 12).map((file) => `    ${file}`);
  if (shipped.length > 12) list.push(`    … ${shipped.length - 12} more`);
  return [
    `${name}: shipped files changed but version ${to ?? "(none)"} is not higher than ${from}; bump plugins/${name}/manifest.json`,
    ...list,
  ].join("\n");
}

// --- git ---------------------------------------------------------------

class GitError extends Error {}

function git(repo, args, { allowFail = false } = {}) {
  const result = spawnSync("git", args, {
    cwd: repo,
    encoding: "buffer",
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.error) throw new GitError(`git: ${result.error.message}`);
  if (result.status !== 0) {
    if (allowFail) return null;
    throw new GitError(
      `git ${args.join(" ")}: ${result.stderr.toString("utf8").trim()}`,
    );
  }
  return result.stdout;
}

const zList = (buffer) =>
  buffer
    ? buffer
        .toString("utf8")
        .split("\0")
        .filter((line) => line.length)
    : [];

/** A side of the comparison: a commit (`ref`) or the working tree (null). */
function side(repo, ref) {
  const read = (path) => {
    if (ref === null) {
      const file = join(repo, path);
      return existsSync(file) ? readFileSync(file) : null;
    }
    return git(repo, ["show", `${ref}:${path}`], { allowFail: true });
  };
  const json = (path) => {
    const bytes = read(path);
    if (!bytes) return null;
    try {
      return JSON.parse(bytes.toString("utf8"));
    } catch {
      return { unreadable: path };
    }
  };
  const names = () => {
    if (ref === null)
      return readdirSync(join(repo, PLUGINS), { withFileTypes: true })
        .filter((item) => item.isDirectory())
        .map((item) => item.name);
    return zList(
      git(repo, ["ls-tree", "-z", "--name-only", `${ref}:${PLUGINS}`]),
    );
  };
  return {
    names,
    plugin(name) {
      const dir = `${PLUGINS}/${name}`;
      return {
        manifest: json(`${dir}/manifest.json`),
        packageJson: json(`${dir}/package.json`),
        present: (rel) => read(`${dir}/${rel}`) !== null,
      };
    },
  };
}

function latestReleaseTag(repo) {
  const tag = git(
    repo,
    ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", "HEAD"],
    {
      allowFail: true,
    },
  );
  if (!tag)
    throw new GitError(
      "no release tag (v*) reachable from HEAD; fetch tags with full history (fetch-depth: 0) or pass --base <ref>",
    );
  return tag.toString("utf8").trim();
}

/** Plugin-relative changed paths per plugin name. */
function changedFiles(repo, base, head) {
  const paths =
    head === null
      ? [
          ...zList(
            git(repo, [
              "diff",
              "-z",
              "--name-only",
              "--no-renames",
              base,
              "--",
              PLUGINS,
            ]),
          ),
          ...zList(
            git(repo, [
              "ls-files",
              "-z",
              "--others",
              "--exclude-standard",
              "--",
              PLUGINS,
            ]),
          ),
        ]
      : zList(
          git(repo, [
            "diff",
            "-z",
            "--name-only",
            "--no-renames",
            base,
            head,
            "--",
            PLUGINS,
          ]),
        );
  const byPlugin = new Map();
  for (const path of paths) {
    const [name, ...rest] = path.slice(PLUGINS.length + 1).split("/");
    if (!rest.length) continue;
    if (!byPlugin.has(name)) byPlugin.set(name, []);
    byPlugin.get(name).push(rest.join("/"));
  }
  return byPlugin;
}

export function check(repo, { base, head = null } = {}) {
  const baseRef = base ?? latestReleaseTag(repo);
  const exists = (ref) =>
    git(repo, ["rev-parse", "--verify", "--quiet", `${ref}^{commit}`], {
      allowFail: true,
    }) !== null;
  if (!exists(baseRef))
    throw new GitError(
      `base ref ${baseRef} not found; fetch tags and full history (fetch-depth: 0)`,
    );
  if (head !== null && !exists(head))
    throw new GitError(`head ref ${head} not found`);
  const baseSide = side(repo, baseRef);
  const headSide = side(repo, head);
  const changed = changedFiles(repo, baseRef, head);
  const problems = [];
  let checked = 0;
  for (const name of headSide.names().sort()) {
    const headPlugin = headSide.plugin(name);
    if (!headPlugin.manifest) continue;
    checked++;
    const message = verdict(
      name,
      changed.get(name) ?? [],
      baseSide.plugin(name),
      headPlugin,
    );
    if (message) problems.push(message);
  }
  return { base: baseRef, head: head ?? "working tree", checked, problems };
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const args = process.argv.slice(2);
  const option = (flag) => {
    const index = args.indexOf(flag);
    return index >= 0 ? args[index + 1] : undefined;
  };
  const repo = resolve(import.meta.dirname, "../..");
  try {
    const result = check(repo, {
      base: option("--base"),
      head: option("--head") ?? null,
    });
    if (result.problems.length) {
      console.error(
        `built-in versions (${result.base} → ${result.head}): ${result.problems.length} plugin(s) changed without a version bump\n${result.problems.join("\n")}`,
      );
      process.exit(1);
    }
    console.log(
      `built-in versions (${result.base} → ${result.head}): ${result.checked} built-in plugins, every changed one has a higher version`,
    );
  } catch (error) {
    if (!(error instanceof GitError)) throw error;
    console.error(`built-in versions: ${error.message}`);
    process.exit(2);
  }
}
