// 实验测试客户端；使用真实 Rust 宿主与真实插件进程，不实现任何媒体业务。
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
function defaultBinary(): string {
  const cached = fileURLToPath(
    new URL(
      "../../../.cache/probe-target/debug/vestra-slint-experimental1",
      import.meta.url,
    ),
  );
  return existsSync(cached)
    ? cached
    : fileURLToPath(
        new URL(
          "../../host/target/debug/vestra-slint-experimental1",
          import.meta.url,
        ),
      );
}
export class TestHost {
  private child: ChildProcessWithoutNullStreams;
  private serial = 0;
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  constructor(binary = process.env.VESTRA_HOST ?? defaultBinary()) {
    this.child = spawn(binary, ["--headless"], { stdio: "pipe" });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      const message: unknown = JSON.parse(line);
      if (!message || typeof message !== "object") return;
      const m = message as Record<string, unknown>;
      const entry = this.pending.get(Number(m.id));
      if (!entry) return;
      this.pending.delete(Number(m.id));
      if (typeof m.error === "string") entry.reject(new Error(m.error));
      else entry.resolve(m.result);
    });
    this.child.stderr.on("data", (chunk) => process.stderr.write(chunk));
    this.child.on("error", (error) => {
      for (const entry of this.pending.values()) entry.reject(error);
      this.pending.clear();
    });
    this.child.on("exit", (code) => {
      for (const entry of this.pending.values())
        entry.reject(new Error(`host exited ${code}`));
      this.pending.clear();
    });
  }
  request<T = unknown>(
    method: string,
    params: Record<string, unknown> = {},
    owner = "$cli",
  ): Promise<T> {
    const id = ++this.serial;
    return new Promise<T>((resolvePromise, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`request timed out: ${method}`));
      }, 30000);
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolvePromise(value as T);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      this.child.stdin.write(
        JSON.stringify({ id, owner, method, params }) + "\n",
      );
    });
  }
  load(plugin: string) {
    return this.request<{ id: string }>("host.load", {
      manifest: resolve(plugin),
    });
  }
  action(plugin: string, name: string, value = 0) {
    return this.request("host.action", { plugin, name, value });
  }
  unload(plugin: string) {
    return this.request("host.unload", { plugin });
  }
  inspect() {
    return this.request<{
      playback: Record<string, unknown>;
      plugins: {
        id: string;
        route: string;
        properties: Record<string, unknown>;
      }[];
      source: unknown;
    }>("host.inspect");
  }
  async until(check: () => Promise<boolean>, timeout = 10000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await check()) return;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    throw new Error("host condition timed out");
  }
  async close() {
    this.child.stdin.end();
    await new Promise<void>((resolvePromise) => {
      const timer = setTimeout(() => this.child.kill("SIGKILL"), 3000);
      if (this.child.exitCode !== null) {
        clearTimeout(timer);
        resolvePromise();
      } else
        this.child.once("exit", () => {
          clearTimeout(timer);
          resolvePromise();
        });
    });
    if (this.child.exitCode !== 0) {
      throw new Error(
        `host exited abnormally: code=${this.child.exitCode}, signal=${this.child.signalCode}`,
      );
    }
  }
}
