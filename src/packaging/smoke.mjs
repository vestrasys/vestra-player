#!/usr/bin/env node
/**
 * Smoke test of a packaged player (proposal §4.3, §7): starts it headless
 * with an empty data directory and a PATH without node, ffmpeg or Homebrew,
 * and requires every built-in plugin to reach runtime.ready (panel plugins
 * may instead report "mounts require a native shell", which is what
 * headless mode answers them).
 *
 *   node packaging/smoke.mjs --exe <vestra-host[.exe]> --plugins <Resources/plugins>
 *     [--seconds 8] [--node-on-path]
 *
 * --node-on-path puts the bundled node's directory on PATH: only for host
 * builds from before the install-layout resolver (S7 host side), which
 * still look up `node` on PATH. Remove it from the workflows once the host
 * branch is merged.
 */
import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fail, parseArgs, readJson } from "./lib.mjs";

const options = parseArgs(process.argv.slice(2), ["node-on-path"]);
if (!options.exe || !options.plugins)
  fail(
    "usage: smoke.mjs --exe <file> --plugins <dir> [--seconds N] [--node-on-path]",
  );
const exe = resolve(options.exe);
const builtins = (
  await readJson(join(resolve(options.plugins), "builtin.json"))
).plugins;
const windows = process.platform === "win32";
const home = await mkdtemp(join(tmpdir(), "vestra-smoke-"));

const systemPath = windows
  ? [
      join(process.env.SystemRoot ?? "C:\\Windows", "System32"),
      process.env.SystemRoot ?? "C:\\Windows",
    ]
  : ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
if (options["node-on-path"])
  systemPath.unshift(windows ? join(dirname(exe), "node") : dirname(exe));
const env = {
  PATH: systemPath.join(delimiter),
  HOME: home,
  ...(windows
    ? {
        APPDATA: join(home, "AppData", "Roaming"),
        LOCALAPPDATA: join(home, "AppData", "Local"),
        USERPROFILE: home,
        SystemRoot: process.env.SystemRoot,
        TEMP: process.env.TEMP,
        TMP: process.env.TMP,
      }
    : { TMPDIR: process.env.TMPDIR ?? "/tmp" }),
};
const seconds = String(options.seconds ?? 8);
console.log(`smoke: ${exe} --headless --seconds ${seconds} (PATH=${env.PATH})`);
const result = spawnSync(exe, ["--headless", "--seconds", seconds], {
  env,
  encoding: "utf8",
  timeout: (Number(seconds) + 60) * 1000,
});
const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
console.log(output);
await rm(home, { recursive: true, force: true });

const problems = [];
if (result.error) problems.push(`could not run: ${result.error.message}`);
if (result.status !== 0)
  problems.push(`exit status ${result.status} (signal ${result.signal})`);
const failed = output
  .split("\n")
  .filter((line) => /failed to start/.test(line));
if (failed.length) problems.push(`${failed.length} plugin(s) failed to start`);
const ready = output
  .split("\n")
  .filter((line) => /runtime\.ready/.test(line)).length;
const headlessOnly = output
  .split("\n")
  .filter((line) =>
    /^HostError: mounts require a native shell/.test(line),
  ).length;
if (ready + headlessOnly < builtins.length)
  problems.push(
    `${ready} ready + ${headlessOnly} headless-only < ${builtins.length} built-in plugins`,
  );
if (problems.length) fail(`smoke test failed: ${problems.join("; ")}`);
console.log(
  `smoke test passed: ${ready} plugins ready, ${headlessOnly} need a native shell, ${builtins.length} built in`,
);
