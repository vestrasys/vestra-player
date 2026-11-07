#!/usr/bin/env node
/**
 * Smoke test of a packaged player (proposal §4.3, §7): starts it headless
 * with an empty data directory and a PATH without node, ffmpeg or Homebrew,
 * and requires every built-in plugin to reach runtime.ready (panel plugins
 * may instead report "mounts require a native shell", which is what
 * headless mode answers them), and the start-up line to name the release
 * version of the packaged release.json (<plugins>/../release.json).
 *
 *   node packaging/smoke.mjs --exe <vestra-host[.exe]> --plugins <Resources/plugins>
 *     [--seconds N]   default 15, on Windows 45
 */
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import {
  exists,
  fail,
  parseArgs,
  readJson,
  reportErrors,
  walk,
} from "./lib.mjs";

reportErrors();
const options = parseArgs(process.argv.slice(2));
if (!options.exe || !options.plugins)
  fail("usage: smoke.mjs --exe <file> --plugins <dir> [--seconds N]");
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
// A cold start starts the built-ins from their files in place; on a Windows
// runner antivirus scans each file on first read, so allow more time.
const seconds = String(options.seconds ?? (windows ? 45 : 15));
console.log(`smoke: ${exe} --headless --seconds ${seconds} (PATH=${env.PATH})`);
const started = Date.now();
const lines = [];
const result = await new Promise((done) => {
  const child = spawn(exe, ["--headless", "--seconds", seconds], { env });
  const timer = setTimeout(() => child.kill(), (Number(seconds) + 60) * 1000);
  for (const stream of [child.stdout, child.stderr]) {
    let rest = "";
    stream.setEncoding("utf8");
    stream.on("data", (chunk) => {
      const parts = (rest + chunk).split(/\r?\n/);
      rest = parts.pop();
      for (const line of parts) {
        lines.push(line);
        console.log(`[${((Date.now() - started) / 1000).toFixed(1)}s] ${line}`);
      }
    });
    stream.on("end", () => rest && lines.push(rest));
  }
  child.on("error", (error) => {
    clearTimeout(timer);
    done({ error });
  });
  child.on("close", (status, signal) => {
    clearTimeout(timer);
    done({ status, signal });
  });
});
console.log(`host exited after ${((Date.now() - started) / 1000).toFixed(1)}s`);
const output = lines.join("\n");
const dataFiles = (await walk(home).catch(() => [])).filter(
  (file) => !file.includes("/node_modules/"),
);

const problems = [];
if (result.error) problems.push(`could not run: ${result.error.message}`);
if (result.status !== 0)
  problems.push(`exit status ${result.status} (signal ${result.signal})`);
const failed = output
  .split("\n")
  .filter((line) => /failed to start/.test(line));
if (failed.length) problems.push(`${failed.length} plugin(s) failed to start`);
// The host reads the release version from <resources>/release.json and logs
// `start: Vestra Player <release> (host <host>, <build>) on …`; a missing or
// unreadable file falls back to the host version, which this catches.
const releaseFile = join(dirname(resolve(options.plugins)), "release.json");
if (exists(releaseFile)) {
  const expected = (await readJson(releaseFile)).version;
  const shown = /^start: Vestra Player (\S+) \(host /m.exec(output)?.[1];
  if (shown !== expected)
    problems.push(
      `start-up line shows version ${shown ?? "(no start line)"}, release.json says ${expected}`,
    );
  else console.log(`smoke: the player reports release ${shown}`);
} else problems.push(`${releaseFile} is missing`);
// Counted per plugin, so one plugin reporting twice cannot stand in for
// another that never got there. The host logs `runtime <owner>
// runtime.ready: …` (one owner per plugin instance).
const ready = new Set(
  output
    .split("\n")
    .map((line) => /\bruntime (\d+) runtime\.ready\b/.exec(line)?.[1])
    .filter(Boolean),
).size;
// The shared runner prefixes a built-in's output with its id
// (`[vestra.playlist] HostError: …`); a plugin in its own runner has no
// prefix, so each such line counts on its own.
const headlessLines = output
  .split("\n")
  .map((line) =>
    /^(?:\[([^\]\s]+)\] )?HostError: mounts require a native shell/.exec(line),
  )
  .filter(Boolean);
const headlessOnly =
  new Set(headlessLines.filter((m) => m[1]).map((m) => m[1])).size +
  headlessLines.filter((m) => !m[1]).length;
if (ready + headlessOnly < builtins.length)
  problems.push(
    `${ready} ready + ${headlessOnly} headless-only < ${builtins.length} built-in plugins`,
  );
if (problems.length) {
  // What the host left in its data directory says how far start-up got.
  console.log(`data directory (${dataFiles.length} files):`);
  for (const file of dataFiles.slice(0, 100)) console.log(`  ${file}`);
  await rm(home, { recursive: true, force: true });
  fail(`smoke test failed: ${problems.join("; ")}`);
}
await rm(home, { recursive: true, force: true });
console.log(
  `smoke test passed: ${ready} plugins ready, ${headlessOnly} need a native shell, ${builtins.length} built in`,
);
