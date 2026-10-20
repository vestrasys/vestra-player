import { createInterface } from "node:readline";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createSdk, HostError } from "../sdk/index.ts";
import type { Json, PluginModule, Transport } from "../sdk/index.ts";
import { createPlugin } from "../sdk/plugin/index.ts";
import type { Plugin } from "../sdk/plugin/index.ts";

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
const send = (message: unknown) =>
  process.stdout.write(`${JSON.stringify(message)}\n`);
/**
 * Requests on the ordered pipe. The host replies to every request or stops
 * this process, so SDK v2 waits for the real reply: a client deadline cannot
 * know whether the host executed the request and only manufactures "unknown"
 * outcomes. The flat legacy API keeps its 10 second deadline until removed.
 */
function request<T>(
  method: string,
  params: Json,
  deadline: boolean,
): Promise<T> {
  if (stopping)
    return Promise.reject(new HostError("disabled", "plugin is stopping"));
  const id = ++nextId;
  if (process.env.VESTRA_TRACE) console.error("rpc request", method, id);
  return new Promise<T>((resolve, reject) => {
    // Native interaction has no duration limit. It ends with selection/cancel
    // or the existing owner-stop path rejects and removes it.
    const timer =
      method === "dialogs.open"
        ? undefined
        : deadline
          ? setTimeout(() => {
              pending.delete(id);
              reject(
                new HostError(
                  "timeout",
                  `${method} exceeded 10 seconds; backend outcome may be unknown`,
                ),
              );
            }, 10_000)
          : setTimeout(
              () =>
                console.error(`rpc ${method} still waiting after 10 seconds`),
              10_000,
            );
    pending.set(id, {
      resolve: (value) => resolve(value as T),
      reject,
      timer,
    });
    send({ id, method, params });
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
let dispose: void | (() => void | Promise<void>);
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
      error?: { code: string; message: string };
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
      if (message.error)
        request.reject(
          new HostError(message.error.code, message.error.message),
        );
      else request.resolve(message.result);
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
try {
  const module = (await import(
    pathToFileURL(resolve(directory, manifest.entry)).href
  )) as Partial<PluginModule> & {
    default?: (plugin: Plugin) => void | Promise<void>;
  };
  if (typeof module.default === "function") {
    // SDK v2: `export default async function activate(plugin)`.
    // A forgotten promise must not take the whole plugin down.
    process.on("unhandledRejection", (error) =>
      console.error("unhandled rejection:", error),
    );
    const instance = await createPlugin(
      {
        request: <T>(method: string, params: Json = {}) =>
          request<T>(method, params, false),
        on,
      },
      manifest,
      directory,
    );
    dispose = () => instance.dispose();
    await module.default(instance.plugin);
    await instance.activated();
  } else if (typeof module.activate === "function") {
    // Legacy flat API, kept until every plugin has migrated.
    const host = createSdk({
      request: <T>(method: string, params: Json = {}) =>
        request<T>(method, params, true),
      on,
    });
    dispose = await module.activate(host);
  } else throw new Error("plugin entry exports no activate function");
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
