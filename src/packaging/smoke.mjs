#!/usr/bin/env node
/**
 * Smoke test of a packaged player (proposal §4.3, §7): starts it headless
 * with an empty data directory and a PATH without node, ffmpeg or Homebrew,
 * and requires every built-in plugin to reach runtime.ready (panel plugins
 * may instead report "mounts require a native shell", which is what
 * headless mode answers them).
 *
 *   node packaging/smoke.mjs --exe <vestra-host[.exe]> --plugins <Resources/plugins>
 *     [--seconds N]   default 15, on Windows 45
 */
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { fail, parseArgs, readJson, walk } from "./lib.mjs";

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
// A cold start starts 17 Node processes on the built-ins in place; on a
// Windows runner antivirus scans each file on first read, so allow more time.
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
