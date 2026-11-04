// Node processes the SDK tests start (runners, scripts). Every one has a
// deadline and none outlives its test: a child left running keeps its pipes
// open, and with them the test file's process, so `node --test` waits for it
// until the CI job's timeout (main.yml run 41824920594).
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import type {
  ChildProcess,
  SpawnOptions,
  SpawnSyncReturns,
} from "node:child_process";

/** How long a child may run before it is killed and its test fails. */
export const CHILD_TIMEOUT_MS = 30_000;

const live = new Set<ChildProcess>();
const ends = new WeakMap<ChildProcess, Promise<number | null>>();

/** `node <args>`, killed by `killChildren` if still running after its test. */
export function spawnNode<T extends ChildProcess = ChildProcess>(
  args: string[],
  options: SpawnOptions = {},
): T {
  const child = spawn(process.execPath, args, options) as T;
  live.add(child);
  ends.set(
    child,
    new Promise((done) => {
      child.once("close", (code) => {
        live.delete(child);
        done(code);
      });
      child.once("error", () => {
        live.delete(child);
        done(null);
      });
    }),
  );
  return child;
}

function kill(child: ChildProcess) {
  live.delete(child);
  child.kill("SIGKILL");
  // A child that does not die at once (stuck in its exit) still must not
  // hold this process open.
  child.stdin?.destroy();
  child.stdout?.destroy();
  child.stderr?.destroy();
  child.unref();
}

/**
 * For `afterEach`: kills every child a test left running (it failed or timed
 * out before the child exited).
 */
export function killChildren() {
  for (const child of live) kill(child);
}

/**
 * The child's exit code once it has exited and its stdout and stderr are read
 * to the end (`close`, not `exit`: at `exit` the pipes may still hold output).
 * Past `ms` the child is killed and the promise rejects.
 */
export function closed(
  child: ChildProcess,
  what: string,
  ms = CHILD_TIMEOUT_MS,
): Promise<number | null> {
  const end = ends.get(child);
  assert.ok(end, "a child started by spawnNode");
  let timer: NodeJS.Timeout | undefined;
  const deadline = new Promise<never>((_, fail) => {
    timer = setTimeout(() => {
      kill(child);
      fail(new Error(`${what} did not exit within ${ms} ms`));
    }, ms);
  });
  return Promise.race([end, deadline]).finally(() => clearTimeout(timer));
}

/**
 * `node <args>` to its end, killed after `CHILD_TIMEOUT_MS` (spawnSync blocks
 * the test runner, so the runner's own timeouts cannot fire).
 */
export function runNode(args: string[]): SpawnSyncReturns<string> {
  const result = spawnSync(process.execPath, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: CHILD_TIMEOUT_MS,
    killSignal: "SIGKILL",
  });
  assert.ifError(result.error);
  return result;
}
