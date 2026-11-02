/**
 * The own-process plugin runner: one plugin, one Node process, the player's
 * JSONL protocol on stdin/stdout. The host starts it as
 * `node runner.js <plugin directory>` (packaged; `runner.ts` in a development
 * build) with the plugin's parameters in the environment
 * (`host/src/process/mod.rs`). The plugin's life is `boot.ts`; this file owns
 * the process: its pipes, its console, its exit.
 */
import { createInterface } from "node:readline";
import { boot, readManifest, STOP_TIMEOUT_MS, stopReason } from "./boot.ts";
import type { Inbound, StopReason } from "./boot.ts";

// stderr is the player's own (inherited), often a pipe of whoever started
// the player, whose reader may be gone: a failed write is an `error` event
// that would stop this process, even from console.error. Logs are best
// effort (the player's side: host/src/diag.rs).
process.stderr.on("error", () => {});
// stdout is the pipe to the player: when it breaks, the player is gone
// (`host-lost`; before `stop` can run, nothing needs cleaning up yet).
let canStop = false;
process.stdout.on("error", () =>
  canStop ? void stop("host-lost") : process.exit(0),
);
// stdout is reserved for JSONL. Plugin logs use stderr.
console.log = (...args: unknown[]) => console.error(...args);
console.info = (...args: unknown[]) => console.error(...args);
console.debug = (...args: unknown[]) => console.error(...args);
const directory = process.argv[2];
if (!directory) throw new Error("plugin directory required");
const manifest = await readManifest(directory);
const trace = () => Boolean(process.env.VESTRA_TRACE);
const plugin = boot({
  directory,
  manifest,
  dataDirectory: process.env.VESTRA_PLUGIN_DATA_DIR || undefined,
  locale: process.env.VESTRA_LOCALE || undefined,
  app: appInfo(),
  write: (line) => process.stdout.write(`${line}\n`),
  log: (...args) => console.error(...args),
  trace: trace(),
  // A forgotten promise must not take the whole plugin down.
  entryLoaded: () =>
    process.on("unhandledRejection", (error) =>
      console.error("unhandled rejection:", error),
    ),
});
let stopping = false;
/** The first stop wins: a `runtime.stop`'s reason stays when stdin closes after it. */
async function stop(reason: StopReason) {
  if (stopping) return;
  stopping = true;
  // A plugin's cleanup that never ends must not keep the process alive (the
  // player may be gone: stdout broke).
  setTimeout(() => process.exit(0), STOP_TIMEOUT_MS);
  await plugin.stop(reason);
  process.exit(0);
}
canStop = true;
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  try {
    const message = JSON.parse(line) as Inbound;
    if (trace()) console.error("rpc receive", line);
    if (message.event === "runtime.stop") {
      void stop(stopReason(message.data));
      return;
    }
    plugin.deliver(message);
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
});
// Closed without a `runtime.stop` first: the player crashed or was killed.
input.on("close", () => {
  void stop("host-lost");
});
/** `plugin.app`: the player hands its version and components over at start. */
function appInfo(): unknown {
  try {
    return JSON.parse(process.env.VESTRA_APP_INFO ?? "null");
  } catch {
    return null;
  }
}
if ((await plugin.done) === "failed") process.exit(1);
