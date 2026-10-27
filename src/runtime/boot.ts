/**
 * One plugin's life inside a runner: the wire client (`request`, `on`), the
 * SDK instance, activation, `runtime.ready` / `runtime.failed`, and `stop`.
 *
 * Shared by the two runner entries (doc/process/rounds/2026-10-shared-runner.md
 * §6): `runner.ts` (one plugin, its own Node process: today's wire) and
 * `shared-runner.ts` (several trusted plugins in one process, one envelope
 * per line). Everything that belongs to the process is a parameter here:
 * this module never reads `process.argv` or `process.env`, never writes
 * stdout, never exits and never touches `console`, so several boots can run
 * side by side in one isolate.
 */
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
// The SDK implementation, from the package the player ships with it
// (development: the workspace package; packaged: Resources/runtime/node_modules).
import { createPlugin, HostError } from "@vestra-player/sdk/runtime";
import type { Json, Transport } from "@vestra-player/sdk/runtime";
import type { Plugin } from "@vestra-player/sdk";

/** How long a runner waits for a plugin's cleanup before giving up on it. */
export const STOP_TIMEOUT_MS = 5000;
/**
 * The player reads protocol lines of at most 1 MiB, newline included, and
 * stops a plugin that writes a longer one (`host/src/plugins.rs`). In the
 * shared runner the limit applies to the inner message; the envelope's
 * `"g":<n>,` is the host's allowance.
 */
export const MAX_LINE = 1024 * 1024;

export interface Manifest {
  id: string;
  entry: string;
  capabilities: string[];
}

/** `manifest.json` of the plugin directory (the host checked it already). */
export async function readManifest(directory: string): Promise<Manifest> {
  return JSON.parse(
    await readFile(resolve(directory, "manifest.json"), "utf8"),
  ) as Manifest;
}

/** A line from the player, already parsed (and unwrapped from its envelope). */
export interface Inbound {
  id?: number;
  result?: unknown;
  error?: { code: string; message: string; reason?: string };
  event?: string;
  data?: unknown;
}

export interface BootOptions {
  directory: string;
  manifest: Manifest;
  /** `plugin.dataDirectory`; none: the SDK reports `unavailable`. */
  dataDirectory?: string;
  /** `plugin.i18n.locale`: the interface language the player fixed for this run. */
  locale?: string;
  /** `plugin.app`: the player's version and components. */
  app: unknown;
  /** Writes one protocol message (JSON text, no newline) to the player. */
  write(line: string): void;
  /** The runner's own diagnostics (stderr). */
  log(...args: unknown[]): void;
  /** `VESTRA_TRACE`: log every request. */
  trace: boolean;
  /**
   * Called once the entry module has loaded and default-exports a function,
   * before `createPlugin` (the own-process runner installs its process-wide
   * `unhandledRejection` handler here, where it always has).
   */
  entryLoaded?(): void;
}

export interface Instance {
  /** A message from the player for this plugin (not `runtime.stop`). */
  deliver(message: Inbound): void;
  /**
   * Rejects the pending requests and runs the plugin's cleanup; resolves when
   * the cleanup settled (no deadline here: the caller decides how long to
   * wait). Idempotent.
   */
  stop(): Promise<void>;
  /** Settles once `runtime.ready` or `runtime.failed` has been written. */
  readonly done: Promise<"ready" | "failed">;
  /** Requests written and requests still waiting for their reply. */
  stats(): { requests: number; pending: number };
}

/**
 * Refusals because another plugin holds a single-owner resource: `conflict`
 * with `reason` `"slot-occupied"` (`ui.mount`: the panel's `ready` rejects),
 * `"sequence-taken"` (`sequence.set`) or `"resolver-taken"`
 * (`sourceResolvers.register`: the resolver's `ready` rejects). They do not
 * fail activation: the plugin runs without its panel, without providing
 * next/previous, or without resolving that file type or site.
 */
const TAKEN = new Set(["slot-occupied", "sequence-taken", "resolver-taken"]);

/**
 * Starts one plugin. Returns at once (nothing is written before the entry
 * module has loaded), so the caller can route the player's lines to
 * `deliver` before activation begins.
 */
export function boot(options: BootOptions): Instance {
  const { directory, manifest, write, log } = options;
  const send = (message: unknown) => write(JSON.stringify(message));
  const taken = new WeakSet<Error>();
  let nextId = 0;
  const pending = new Map<
    number,
    {
      resolve(value: unknown): void;
      reject(error: Error): void;
      timer: NodeJS.Timeout | undefined;
    }
  >();
  const listeners = new Map<string, Set<(data: unknown) => void>>();
  let dispose: (() => Promise<void>) | undefined;
  let stopping: Promise<void> | undefined;

  /**
   * Requests on the ordered pipe. The host replies to every request or stops
   * this plugin, so the runner waits for the real reply: a client deadline
   * cannot know whether the host executed the request and would only
   * manufacture "unknown" outcomes. A slow request is logged, not abandoned.
   */
  function request<T>(method: string, params: Json = {}): Promise<T> {
    if (stopping)
      return Promise.reject(new HostError("disabled", "plugin is stopping"));
    // Refused here rather than sent: an oversized line would stop the plugin.
    // Property values meet their own limits first (`UI_VALUE_LIMITS`).
    const line = JSON.stringify({ id: nextId + 1, method, params });
    if (Buffer.byteLength(line, "utf8") + 1 > MAX_LINE)
      return Promise.reject(
        new HostError(
          "limit",
          `${method}: request exceeds the player's 1 MiB message limit`,
        ),
      );
    const id = ++nextId;
    if (options.trace) log("rpc request", method, id);
    return new Promise<T>((resolve, reject) => {
      // Native dialogs have no duration limit: they end with selection/cancel.
      const timer =
        method === "dialogs.open"
          ? undefined
          : setTimeout(
              () => log(`rpc ${method} still waiting after 10 seconds`),
              10_000,
            );
      pending.set(id, {
        resolve: (value) => resolve(value as T),
        reject,
        timer,
      });
      write(line);
    });
  }
  const on: Transport["on"] = (event, listener) => {
    const group = listeners.get(event) ?? new Set();
    group.add(listener);
    listeners.set(event, group);
    return () => {
      group.delete(listener);
    };
  };

  function deliver(message: Inbound) {
    if (message.id !== undefined) {
      const request = pending.get(message.id);
      if (!request) return;
      clearTimeout(request.timer);
      pending.delete(message.id);
      if (message.error) {
        const error = new HostError(message.error.code, message.error.message);
        if (TAKEN.has(message.error.reason ?? "")) taken.add(error);
        request.reject(error);
      } else request.resolve(message.result);
    } else if (message.event) {
      for (const listener of listeners.get(message.event) ?? []) {
        try {
          listener(message.data);
        } catch (error) {
          log(error);
        }
      }
    }
  }

  function stop(): Promise<void> {
    stopping ??= (async () => {
      for (const request of pending.values()) {
        clearTimeout(request.timer);
        request.reject(new HostError("disabled", "plugin stopped"));
      }
      pending.clear();
      try {
        await dispose?.();
      } catch (error) {
        log(error);
      }
    })();
    return stopping;
  }

  async function activate(): Promise<"ready" | "failed"> {
    try {
      const module = (await import(
        pathToFileURL(resolve(directory, manifest.entry)).href
      )) as { default?: (plugin: Plugin) => void | Promise<void> };
      // `export default async function activate(plugin)`; see doc/architecture/sdk-design.md.
      if (typeof module.default !== "function")
        throw new Error("plugin entry must default-export activate(plugin)");
      options.entryLoaded?.();
      const instance = await createPlugin(
        { request, on, locale: options.locale },
        manifest,
        directory,
        options.dataDirectory,
        options.app,
      );
      dispose = () => instance.dispose();
      await module.default(instance.plugin);
      // `activated()` throws the first rejected start-up work; called again it
      // waits for the rest. What another plugin holds is not an activation failure.
      for (;;) {
        try {
          await instance.activated();
          break;
        } catch (error) {
          if (!(error instanceof Error && taken.has(error))) throw error;
          log(`not taken over: ${error.message}`);
        }
      }
      send({ event: "runtime.ready" });
      return "ready";
    } catch (error) {
      log(error);
      send({
        event: "runtime.failed",
        data: {
          code: error instanceof HostError ? error.code : "activationFailed",
          message: String(error),
        },
      });
      return "failed";
    }
  }

  return {
    deliver,
    stop,
    done: activate(),
    stats: () => ({ requests: nextId, pending: pending.size }),
  };
}
