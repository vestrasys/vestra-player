// Starts and talks to the agent worker (worker/worker.ts) of one session.
//
// The worker is this Node (process.execPath) with the permission model on:
//   read   the plugin package (worker code, vendored pi, the SDK reference) and the session folder
//   write  the session folder only (pi's JSONL; the tools stay in work/)
//   net    allowed (the provider)
// No child processes, worker threads, native addons or WASI. The API key goes
// over the IPC channel, never into the environment, arguments or a file.
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { StartMessage, ToolReply, ToPlugin, ToWorker, WorkerEvent } from "./worker/protocol.ts";

const here = import.meta.dirname;

/** Variables the worker sees: pi's switches and the user's proxy settings. */
const PASSED = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "NO_PROXY", "no_proxy", "NODE_EXTRA_CA_CERTS", "SystemRoot", "SYSTEMROOT", "TEMP", "TMP"];

export function workerArguments(sessionDir: string): string[] {
  const real = (file: string) => fs.realpathSync(file);
  return [
    "--no-experimental-webstorage",
    "--permission",
    "--allow-net",
    // --allow-net is experimental in Node 26 and says so on every start.
    "--disable-warning=ExperimentalWarning",
    `--allow-fs-read=${real(here)}`,
    `--allow-fs-read=${real(sessionDir)}`,
    `--allow-fs-write=${real(sessionDir)}`,
    path.join(real(here), "worker", "worker.ts"),
  ];
}

export function workerEnvironment(): Record<string, string> {
  const env: Record<string, string> = {
    PI_OFFLINE: "1",
    PI_TELEMETRY: "0",
    PI_SKIP_VERSION_CHECK: "1",
    // pi reads its own package.json at import; the vendored copy carries one.
    PI_PACKAGE_DIR: path.join(fs.realpathSync(here), "vendor", "pi"),
  };
  for (const name of PASSED) if (process.env[name]) env[name] = process.env[name]!;
  return env;
}

export interface WorkerHandlers {
  event(event: WorkerEvent): void;
  file(sessionFile: string): void;
  failed(message: string): void;
  exited(code: number | null, stderr: string): void;
  /** A plugin tool call: answer with AgentWorker.reply(call, …); `signal` aborts when the agent stops waiting. */
  tool?(call: string, name: string, args: Record<string, unknown>, signal: AbortSignal): void;
}

export class AgentWorker {
  private readonly child: ChildProcess;
  private readonly ready: Promise<void>;
  private stderr = "";
  private readonly calls = new Map<string, AbortController>();
  exited = false;

  constructor(options: Omit<StartMessage, "type">, handlers: WorkerHandlers) {
    // The permission model compares real paths (macOS: /tmp is /private/tmp).
    const start = {
      ...options,
      sessionDir: fs.realpathSync(options.sessionDir),
      workDir: fs.realpathSync(options.workDir),
      sessionFile: options.sessionFile && fs.realpathSync(options.sessionFile),
      docsRoots: options.docsRoots.map((root) => fs.realpathSync(root)),
    };
    this.child = spawn(process.execPath, workerArguments(start.sessionDir), {
      cwd: start.sessionDir,
      env: workerEnvironment(),
      stdio: ["ignore", "ignore", "pipe", "ipc"],
      windowsHide: true,
    });
    this.child.stderr?.on("data", (chunk: Buffer) => {
      this.stderr = (this.stderr + chunk.toString()).slice(-16384);
    });
    let resolveReady!: () => void;
    let rejectReady!: (error: Error) => void;
    this.ready = new Promise((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    this.ready.catch(() => {}); // reported through handlers.failed
    let started = false;
    this.child.on("message", (raw) => {
      const message = raw as ToPlugin;
      if (message.type === "ready") {
        started = true;
        resolveReady();
      } else if (message.type === "event") handlers.event(message.event);
      else if (message.type === "file") handlers.file(message.sessionFile);
      else if (message.type === "tool") {
        const controller = new AbortController();
        this.calls.set(message.call, controller);
        if (handlers.tool) handlers.tool(message.call, message.name, message.args, controller.signal);
        else this.reply(message.call, { ok: false, text: `Tool ${message.name} is not available` });
      } else if (message.type === "toolCancel") {
        this.calls.get(message.call)?.abort();
        this.calls.delete(message.call);
      }
      else if (message.type === "failed") {
        if (!started) rejectReady(new Error(message.message));
        handlers.failed(message.message);
      }
    });
    this.child.on("error", (error) => {
      rejectReady(error);
      handlers.failed(`Cannot start the agent process: ${error.message}`);
    });
    this.child.on("exit", (code) => {
      this.exited = true;
      for (const controller of this.calls.values()) controller.abort();
      this.calls.clear();
      rejectReady(new Error(this.lastError() || `The agent process exited (${code})`));
      handlers.exited(code, this.stderr);
    });
    this.child.once("spawn", () => this.send({ type: "start", ...start }));
  }

  /** The last line of the worker's stderr, for a failure message. */
  lastError(): string {
    return this.stderr.trim().split("\n").filter(Boolean).at(-1) ?? "";
  }

  private send(message: ToWorker) {
    if (!this.exited && this.child.connected) this.child.send(message);
  }

  /** The answer to a plugin tool call (ignored when the call was cancelled meanwhile). */
  reply(call: string, reply: ToolReply) {
    if (!this.calls.delete(call)) return;
    this.send({ type: "toolResult", call, reply });
  }

  async prompt(text: string): Promise<void> {
    await this.ready;
    this.send({ type: "prompt", text });
  }

  abort() {
    this.send({ type: "abort" });
  }

  /** Ends the worker: asks first, kills after `graceMs`. */
  async stop(graceMs = 1000): Promise<void> {
    if (this.exited) return;
    const gone = new Promise<void>((resolve) => this.child.once("exit", () => resolve()));
    this.send({ type: "shutdown" });
    const timer = setTimeout(() => this.child.kill("SIGKILL"), graceMs);
    await gone;
    clearTimeout(timer);
  }
}
