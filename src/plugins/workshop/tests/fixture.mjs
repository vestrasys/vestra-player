// Shared test setup: the real plugin on the SDK test host, with real agent
// workers (Node, permission model) talking to a local mock OpenAI server.
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after } from "node:test";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
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

export async function server(reply, options) {
  const mock = await mockOpenAI(reply, options);
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
 * 「自定义 · OpenAI 格式」 (its first listed model, mock-1) with KEY.
 */
export async function startAi({ data, compat, secrets = {}, deny = false } = {}) {
  const host = new TestHost();
  // `deny`: the user refuses the keychain's authorization dialog.
  if (deny)
    host.handle("secrets.get", () => {
      throw new HostError("failed", "User canceled the operation.");
    });
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
  /** A custom endpoint: Base URL (Enter lists its models), then the key. */
  const configure = async (url, { format = "OpenAI", model } = {}) => {
    page.emit("choose-provider", page.props.providers.indexOf(`自定义 · ${format} 格式`));
    await host.settle();
    page.emit("accept-base-url", url);
    await until(host, () => page.props.listing === false && page.props.message !== "", "model list");
    if (model) page.emit("choose-model", model);
    page.emit("save-key", KEY);
    await until(host, () => props.configured === true && page.props.listing === false && /key 已保存|获取到|没能获取/.test(page.props.message), "configured");
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
    /** The rows, assistant text read back from its Markdown document (`body`). */
    rows: () => props.rows.map((row) => ({ ...row, text: host.markdown(row.body)?.text || row.text })),
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
