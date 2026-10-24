// One child process of a plugin tool (the CLI, a test run): this Node
// (process.execPath), fixed arguments, a time limit, capped output, and
// cancellation through an AbortSignal. Never a shell.
import { spawn } from "node:child_process";

export interface RunOptions {
  cwd: string;
  env: Record<string, string>;
  timeoutMs: number;
  /** Bytes kept of stdout and of stderr each (the rest is dropped, the child goes on). */
  maxOutput?: number;
  signal?: AbortSignal;
}

export interface RunResult {
  code: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
  cancelled: boolean;
  /** Output beyond maxOutput was dropped. */
  truncated: boolean;
}

/** Variables every child may see besides its own: proxies, the Windows basics. */
const PASSED = ["SystemRoot", "SYSTEMROOT", "windir", "PATHEXT", "LANG", "LC_ALL"];

export function baseEnvironment(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const name of PASSED) if (process.env[name]) env[name] = process.env[name]!;
  return env;
}

export function runNode(args: string[], options: RunOptions): Promise<RunResult> {
  const max = options.maxOutput ?? 256 * 1024;
  return new Promise((resolve) => {
    if (options.signal?.aborted) {
      resolve({ code: null, stdout: "", stderr: "", timedOut: false, cancelled: true, truncated: false });
      return;
    }
    const child = spawn(process.execPath, args, { cwd: options.cwd, env: options.env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    let stdout = "";
    let stderr = "";
    let truncated = false;
    let timedOut = false;
    let cancelled = false;
    const keep = (current: string, chunk: Buffer) => {
      if (current.length >= max) {
        truncated = true;
        return current;
      }
      const next = current + chunk.toString("utf8");
      if (next.length > max) truncated = true;
      return next.slice(0, max);
    };
    child.stdout.on("data", (chunk: Buffer) => (stdout = keep(stdout, chunk)));
    child.stderr.on("data", (chunk: Buffer) => (stderr = keep(stderr, chunk)));
    const kill = () => child.kill("SIGKILL");
    const timer = setTimeout(() => {
      timedOut = true;
      kill();
    }, options.timeoutMs);
    const abort = () => {
      cancelled = true;
      kill();
    };
    options.signal?.addEventListener("abort", abort, { once: true });
    const done = (code: number | null) => {
      clearTimeout(timer);
      options.signal?.removeEventListener("abort", abort);
      resolve({ code, stdout, stderr, timedOut, cancelled, truncated });
    };
    child.on("error", (error) => {
      stderr += String(error.message);
      done(null);
    });
    child.on("close", (code) => done(code));
  });
}
