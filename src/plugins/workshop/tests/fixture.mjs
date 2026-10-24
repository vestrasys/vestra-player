// Shared test setup: the real plugin on the SDK test host, with real agent
// workers (Node, permission model) talking to a local mock OpenAI server.
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after } from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { mockOpenAI } from "./mock-openai.mjs";

export const KEY = "sk-test-0123456789-SECRET";
const cleanup = [];
after(() => Promise.all(cleanup.map((fn) => fn())));

export async function dataDirectory() {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-ai-test-"));
  cleanup.push(() => fs.rm(dir, { recursive: true, force: true }));
  return dir;
}

export async function server(reply) {
  const mock = await mockOpenAI(reply);
  cleanup.push(() => mock.close());
  return mock;
}

/** Waits for `condition`, then for the window to stop changing. */
export async function until(host, condition, what = "condition", ms = 20_000) {
  const deadline = Date.now() + ms;
  for (;;) {
    if (condition()) {
      const calls = host.calls.length;
      await new Promise((resolve) => setTimeout(resolve, 150));
      if (host.calls.length === calls && condition()) return;
    } else await new Promise((resolve) => setTimeout(resolve, 20));
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
  }
}

/**
 * The plugin, its window opened. `compat`: a mock server URL to configure as
 * the 「OpenAI 兼容」 provider with model mock-1 and KEY.
 */
export async function startAi({ data, compat, secrets = {} } = {}) {
  const host = new TestHost();
  for (const [name, value] of Object.entries(secrets)) host.secrets.set(name, value);
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host, ...(data ? { dataDirectory: data } : {}) });
  // Also after a failed test: the plugin's workers end with it.
  let stopped = false;
  const stop = () => (stopped ? undefined : ((stopped = true), started.stop()));
  cleanup.unshift(stop);
  await host.command("workshop.open").run();
  const window = host.window("ai");
  const page = host.settingsPage("byok");
  const props = window.props;
  const send = async (callback, ...args) => {
    window.emit(callback, ...args);
    await host.settle();
  };
  const configure = async (url) => {
    const index = page.props.providers.findIndex((label) => label.startsWith("OpenAI 兼容"));
    page.emit("choose-provider", index);
    await host.settle();
    page.emit("set-base-url", url);
    page.emit("choose-model", "mock-1");
    page.emit("save-key", KEY);
    await until(host, () => props.configured === true, "configured");
  };
  if (compat) await configure(compat);
  const ai = {
    host,
    window,
    page,
    props,
    plugin: started.plugin,
    send,
    configure,
    until: (condition, what, ms) => until(host, condition, what, ms),
    rows: () => props.rows,
    kinds: () => props.rows.map((r) => r.kind).join(","),
    /** Type and send a message, then wait until the run is over. */
    async ask(text, { wait = true } = {}) {
      window.emit("draft-changed", text);
      await send("send", text);
      if (wait) await ai.until(() => props.busy === false && props.rows.length > 1 && props.rows.at(-1).state !== 0, "reply");
    },
    stop,
  };
  return ai;
}

/** Every file under `dir` whose bytes contain `text`. */
export async function filesContaining(dir, text) {
  const found = [];
  for (const entry of await fs.readdir(dir, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile()) continue;
    const file = path.join(entry.parentPath, entry.name);
    if ((await fs.readFile(file)).includes(text)) found.push(file);
  }
  return found;
}
