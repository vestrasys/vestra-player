#!/usr/bin/env node
/**
 * The CI checks on this machine (doc/process/ci.md): the same commands as
 * .github/workflows/_static.yml and _build-test.yml, in that order. Every
 * step runs; the summary lists the ones that failed.
 *
 *   npm run ci:local [-- --static] [-- --advisories] [-- --strict-sdk]
 *
 *   --static      only the static checks (no clippy, test, doc)
 *   --advisories  also `cargo deny check advisories` (nightly's audit)
 *   --strict-sdk  VESTRA_SDK_STRICT=1: the SDK's product data (built-in
 *                 commands, Slint copy, Chinese texts, palette colours) must
 *                 match the player too, as when the SDK dependency is
 *                 upgraded and before a release (agentic-workflow.md §1.6)
 *
 * Without --strict-sdk, product-data mismatches only warn (test:sdk TODOs,
 * sdk:sync-slint warnings); the steps write them to VESTRA_SDK_ADVISORY_FILE
 * and the summary lists them under `advisories:`.
 *
 * Differences from CI: `portability` runs in full here (CI leaves its host
 * half to the Windows job), and clippy/test run for this machine's OS only.
 * Tools: typos, actionlint, zizmor, cargo-deny, cargo-shear
 * (`brew install typos-cli actionlint zizmor cargo-deny cargo-shear`).
 * Cargo output goes to $CARGO_TARGET_DIR, or <system temp>/vestra-target/ci.
 * First, `npm ci` runs in src/ and in each plugin with a package-lock.json
 * whose node_modules does not match its lockfile (CI always installs fresh).
 *
 * The plugin tests record the property values the plugins send to their
 * Slint views (VESTRA_UI_RECORD, scripts/ui-record.mjs); the host's ignored
 * test `recorded_plugin_props_match_their_slint` (run by `cargo test
 * -- --include-ignored`) sets each one on the compiled Slint component.
 */
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";

const args = new Set(process.argv.slice(2));
const source = resolve(import.meta.dirname, "..");
const repo = resolve(source, "..");
const host = join(source, "host");
const record = mkdtempSync(join(tmpdir(), "vestra-ui-record-"));
const advisoryDir = mkdtempSync(join(tmpdir(), "vestra-sdk-advisories-"));
const advisories = join(advisoryDir, "advisories.txt");
const env = {
  ...process.env,
  CARGO_TARGET_DIR:
    process.env.CARGO_TARGET_DIR || join(tmpdir(), "vestra-target", "ci"),
  VESTRA_UI_RECORD: record,
  VESTRA_SDK_ADVISORY_FILE: advisories,
  ...(args.has("--strict-sdk") ? { VESTRA_SDK_STRICT: "1" } : {}),
};

const steps = [
  [repo, "typos"],
  [repo, "actionlint"],
  [repo, "zizmor --offline --persona regular .github"],
  [host, "cargo fmt --all --check"],
  [host, "cargo deny --locked check licenses bans sources"],
  ...(args.has("--advisories")
    ? [[host, "cargo deny --locked check advisories"]]
    : []),
  [host, "cargo shear"],
  [repo, "node src/scripts/check-unsafe-policy.mjs"],
  [source, "npm run check"],
  [source, "npm run format:check"],
  [source, "npm run boundaries"],
  [source, "npm run licenses"],
  [source, "node packaging/icon.mjs --check"],
  [source, "npm run test:packaging"],
  [source, "npm run portability"],
  [source, "npm run test:sdk"],
  [source, "npm run plugins:build"],
  [source, "npm run plugin -- test --all"],
  [source, "npm run sdk:sync-slint"],
  ...(args.has("--static")
    ? []
    : [
        [
          host,
          "cargo clippy --locked --workspace --all-targets -- -D warnings",
        ],
        [
          host,
          "cargo test --locked --workspace --no-fail-fast -- --include-ignored",
        ],
        [
          host,
          "cargo doc --locked --no-deps --workspace --document-private-items",
          { RUSTDOCFLAGS: "-D warnings" },
        ],
      ]),
];

// CI runs `npm ci` in src/ and in every plugin with its own lockfile
// (.github/actions/setup-node). A worktree's node_modules can lag behind its
// lockfile, which shows up later as unrelated type errors; reinstall first.
let installFailed = false;
for (const dir of [
  source,
  ...readdirSync(join(source, "plugins"))
    .map((name) => join(source, "plugins", name))
    .filter((dir) => existsSync(join(dir, "package-lock.json"))),
]) {
  const stale = staleDeps(dir);
  if (!stale) continue;
  const name = relative(repo, dir).replaceAll("\\", "/");
  console.log(`deps: ${name}: ${stale}; running npm ci`);
  const result = spawnSync("npm", ["ci", "--no-audit", "--no-fund"], {
    cwd: dir,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    console.log(`ci:local: npm ci failed in ${name}; run it there by hand`);
    installFailed = true;
    break;
  }
}

/**
 * Why `dir/node_modules` does not match `dir/package-lock.json`, or null.
 * Compares the lockfile with npm's record of the installed tree
 * (node_modules/.package-lock.json) and checks each installed folder exists.
 */
function staleDeps(dir) {
  const read = (path) => JSON.parse(readFileSync(path, "utf8")).packages ?? {};
  const hidden = join(dir, "node_modules", ".package-lock.json");
  if (!existsSync(hidden)) return "node_modules is missing or incomplete";
  const want = read(join(dir, "package-lock.json"));
  let have;
  try {
    have = read(hidden);
  } catch {
    return "node_modules/.package-lock.json is unreadable";
  }
  for (const [path, entry] of Object.entries(want)) {
    if (path === "") continue;
    const got = have[path];
    if (!got) {
      if (entry.optional) continue; // another platform's optional package
      return `${path} is not installed`;
    }
    for (const key of ["version", "resolved", "integrity", "link"])
      if (got[key] !== entry[key]) return `${path} differs from the lockfile`;
    if (!existsSync(join(dir, path))) return `${path} is missing`;
  }
  for (const path of Object.keys(have))
    if (!(path in want)) return `${path} is not in the lockfile`;
  return null;
}

const failed = [];
// Never process.exit() (packaging/lib.mjs reportErrors: it can deadlock on
// Node 26); an install failure skips the steps and sets the exit code.
for (const [cwd, command, extra = {}] of installFailed ? [] : steps) {
  console.log(`\n== ${command}`);
  const [program, ...rest] = command.split(" ");
  const started = Date.now();
  const result = spawnSync(program, rest, {
    cwd,
    env: { ...env, ...extra },
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  const seconds = ((Date.now() - started) / 1000).toFixed(1);
  if (result.error?.code === "ENOENT") {
    console.log(`not installed: ${program} (see the header of this script)`);
    failed.push(`${command} (not installed)`);
  } else if (result.status !== 0) {
    console.log(`FAILED (${seconds}s)`);
    failed.push(command);
  } else console.log(`ok (${seconds}s)`);
}

const warned = existsSync(advisories)
  ? readFileSync(advisories, "utf8").split("\n").filter(Boolean)
  : [];
rmSync(record, { recursive: true, force: true });
rmSync(advisoryDir, { recursive: true, force: true });
if (installFailed) process.exitCode = 1;
else {
  console.log(
    `\nci:local: ${steps.length - failed.length}/${steps.length} passed`,
  );
  for (const command of failed) console.log(`  failed: ${command}`);
  if (warned.length) {
    console.log(
      `advisories: ${warned.length} (SDK product data; --strict-sdk fails on them)`,
    );
    for (const line of warned) console.log(`  ${line}`);
  }
  process.exitCode = failed.length ? 1 : 0;
}
