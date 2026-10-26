#!/usr/bin/env node
/**
 * The CI checks on this machine (doc/process/ci.md): the same commands as
 * .github/workflows/_static.yml and _build-test.yml, in that order. Every
 * step runs; the summary lists the ones that failed.
 *
 *   npm run ci:local [-- --static] [-- --advisories]
 *
 *   --static      only the static checks (no clippy, test, doc)
 *   --advisories  also `cargo deny check advisories` (nightly's audit)
 *
 * Differences from CI: `portability` runs in full here (CI leaves its host
 * half to the Windows job), and clippy/test run for this machine's OS only.
 * Tools: typos, actionlint, zizmor, cargo-deny, cargo-shear
 * (`brew install typos-cli actionlint zizmor cargo-deny cargo-shear`).
 * Cargo output goes to $CARGO_TARGET_DIR, or <system temp>/vestra-target/ci.
 *
 * The plugin tests record the property values the plugins send to their
 * Slint views (VESTRA_UI_RECORD, scripts/ui-record.mjs); the host's ignored
 * test `recorded_plugin_props_match_their_slint` (run by `cargo test
 * -- --include-ignored`) sets each one on the compiled Slint component.
 */
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = new Set(process.argv.slice(2));
const source = resolve(import.meta.dirname, "..");
const repo = resolve(source, "..");
const host = join(source, "host");
const record = mkdtempSync(join(tmpdir(), "vestra-ui-record-"));
const env = {
  ...process.env,
  CARGO_TARGET_DIR:
    process.env.CARGO_TARGET_DIR || join(tmpdir(), "vestra-target", "ci"),
  VESTRA_UI_RECORD: record,
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

const failed = [];
for (const [cwd, command, extra = {}] of steps) {
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

rmSync(record, { recursive: true, force: true });
console.log(
  `\nci:local: ${steps.length - failed.length}/${steps.length} passed`,
);
for (const command of failed) console.log(`  failed: ${command}`);
process.exit(failed.length ? 1 : 0);
