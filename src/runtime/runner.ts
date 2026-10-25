import { createInterface } from "node:readline";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
// The SDK implementation, from the package the player ships with it
// (development: the workspace package; packaged: Resources/runtime/node_modules).
import { createPlugin, HostError } from "@vestra-player/sdk/runtime";
import type { Json, Transport } from "@vestra-player/sdk/runtime";
import type { Plugin } from "@vestra-player/sdk";

// stdout is reserved for JSONL. Plugin logs use stderr.
console.log = (...args: unknown[]) => console.error(...args);
console.info = (...args: unknown[]) => console.error(...args);
console.debug = (...args: unknown[]) => console.error(...args);
const directory = process.argv[2];
if (!directory) throw new Error("plugin directory required");
const manifest = JSON.parse(
  await readFile(resolve(directory, "manifest.json"), "utf8"),
) as { id: string; entry: string; capabilities: string[] };
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
/**
 * Refusals because another plugin holds a single-owner resource: `conflict`
 * with `reason` `"slot-occupied"` (`ui.mount`: the panel's `ready` rejects),
 * `"sequence-taken"` (`sequence.set`) or `"resolver-taken"`
 * (`sourceResolvers.register`: the resolver's `ready` rejects). They do not
 * fail activation: the plugin runs without its panel, without providing
 * next/previous, or without resolving that file type or site.
 */
const TAKEN = new Set(["slot-occupied", "sequence-taken", "resolver-taken"]);
const taken = new WeakSet<Error>();
/**
 * The player reads protocol lines of at most 1 MiB, newline included, and
 * stops a plugin that writes a longer one (`host/src/plugins.rs`).
 */
const MAX_LINE = 1024 * 1024;
const send = (message: unknown) =>
  process.stdout.write(`${JSON.stringify(message)}\n`);
/**
 * Requests on the ordered pipe. The host replies to every request or stops
 * this process, so the runner waits for the real reply: a client deadline
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
  if (process.env.VESTRA_TRACE) console.error("rpc request", method, id);
  return new Promise<T>((resolve, reject) => {
    // Native dialogs have no duration limit: they end with selection/cancel.
    const timer =
      method === "dialogs.open"
        ? undefined
        : setTimeout(
            () => console.error(`rpc ${method} still waiting after 10 seconds`),
            10_000,
          );
    pending.set(id, {
      resolve: (value) => resolve(value as T),
      reject,
      timer,
    });
    process.stdout.write(`${line}\n`);
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
let dispose: (() => Promise<void>) | undefined;
let stopping = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  for (const request of pending.values()) {
    clearTimeout(request.timer);
    request.reject(new HostError("disabled", "plugin stopped"));
  }
  pending.clear();
  try {
    await dispose?.();
  } catch (error) {
    console.error(error);
  }
  process.exit(0);
}
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  try {
    const message = JSON.parse(line) as {
      id?: number;
      result?: unknown;
      error?: { code: string; message: string; reason?: string };
      event?: string;
      data?: unknown;
    };
    if (process.env.VESTRA_TRACE) console.error("rpc receive", line);
    if (message.event === "runtime.stop") {
      void stop();
      return;
    }
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
          console.error(error);
        }
      }
    }
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
});
input.on("close", () => {
  void stop();
});
/** `plugin.app`: the player hands its version and components over at start. */
function appInfo(): unknown {
  try {
    return JSON.parse(process.env.VESTRA_APP_INFO ?? "null");
  } catch {
    return null;
  }
}
try {
  const module = (await import(
    pathToFileURL(resolve(directory, manifest.entry)).href
  )) as { default?: (plugin: Plugin) => void | Promise<void> };
  // `export default async function activate(plugin)`; see doc/architecture/sdk-design.md.
  if (typeof module.default !== "function")
    throw new Error("plugin entry must default-export activate(plugin)");
  // A forgotten promise must not take the whole plugin down.
  process.on("unhandledRejection", (error) =>
    console.error("unhandled rejection:", error),
  );
  const instance = await createPlugin(
    // `plugin.i18n.locale`: the interface language the player fixed for this run.
    { request, on, locale: process.env.VESTRA_LOCALE || undefined },
    manifest,
    directory,
    process.env.VESTRA_PLUGIN_DATA_DIR || undefined,
    appInfo(),
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
      console.error(`not taken over: ${error.message}`);
    }
  }
  send({ event: "runtime.ready" });
} catch (error) {
  console.error(error);
  send({
    event: "runtime.failed",
    data: {
      code: error instanceof HostError ? error.code : "activationFailed",
      message: String(error),
    },
  });
  process.exit(1);
}
