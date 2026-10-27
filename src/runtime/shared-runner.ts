/**
 * The shared runner: several trusted plugins in one Node process and one V8
 * isolate (doc/process/rounds/2026-10-shared-runner.md §5, §6). The host
 * starts it as `node shared-runner.js` and talks JSONL on stdin/stdout like
 * to an own-process runner, except that every line carries the owner
 * generation `g` as its first key: `{"g":12,"id":7,"method":…}`. `g` 0 is the
 * runner itself:
 *
 *   S → host  {"g":0,"event":"runner.hello","data":{"pid":…,"node":"…"}}       once, at start
 *   host → S  {"g":0,"event":"plugin.start","data":{"g":12,"id":"…","directory":"…",
 *                "dataDirectory":"…","locale":"…","app":{…}}}
 *   host → S  {"g":12,"event":"runtime.stop"}                                   stop one plugin
 *   S → host  {"g":0,"event":"runner.stopped","data":{"g":12,"timedOut":false}}
 *   S → host  {"g":0,"event":"runner.fault","data":{"g":12,"message":"…"}}      an uncaught exception
 *                                                                               attributed to g (gone);
 *                                                                               g 0: unattributed, S exits 1
 *   host → S  {"g":0,"event":"runner.ping","data":{"seq":41}}
 *   S → host  {"g":0,"event":"runner.pong","data":{"seq":41,"stats":{…}}}
 *   host → S  {"g":0,"event":"runner.stop"}                                     stop all, exit 0
 *
 * Every other line is a plugin's own message, exactly as `runner.ts` writes
 * and reads it, plus `g`. A plugin's failure, stop or attributed fault never
 * ends the process; only `runner.stop`, a closed stdin or stdout, or an
 * exception no plugin owns does.
 *
 * Attribution: each plugin's activation and every message delivered to it
 * run inside `AsyncLocalStorage` with that plugin as the store, so its
 * timers, promise continuations and I/O callbacks carry it too. Console
 * lines and stray stdout writes get a `[<plugin id>] ` prefix (`[shared] `
 * when no plugin owns the context) and go to stderr.
 */
import { AsyncLocalStorage } from "node:async_hooks";
import { createInterface } from "node:readline";
import { format } from "node:util";
import { boot, readManifest, STOP_TIMEOUT_MS } from "./boot.ts";
import type { Inbound, Instance } from "./boot.ts";

interface Member {
  g: number;
  id: string;
  /** Unset until the manifest has been read. */
  instance?: Instance;
  /** False once stopped, failed or faulted: its writes are dropped. */
  alive: boolean;
  stopped?: Promise<void>;
  events: number;
  handlerMs: number;
  startedAt: number;
  readyMs?: number;
}

interface Start {
  g: number;
  id: string;
  directory: string;
  dataDirectory?: string;
  locale?: string;
  app?: unknown;
}

/** Tests shorten the stop deadline; the host never sets it. */
const stopTimeout =
  Number(process.env.VESTRA_SHARED_STOP_MS) > 0
    ? Number(process.env.VESTRA_SHARED_STOP_MS)
    : STOP_TIMEOUT_MS;
const trace = Boolean(process.env.VESTRA_TRACE);
const als = new AsyncLocalStorage<Member>();
const members = new Map<number, Member>();

// stderr: best effort, as in runner.ts.
process.stderr.on("error", () => {});
const stderr = process.stderr.write.bind(process.stderr);
const prefix = () => `[${als.getStore()?.id ?? "shared"}] `;
const logLine = (...args: unknown[]) => {
  stderr(`${prefix()}${format(...args)}\n`);
};
// stdout carries the protocol and only `post` writes it. Plugin code that
// writes stdout (a library's progress output) reaches stderr instead of
// corrupting every plugin's channel.
const stdout = process.stdout.write.bind(process.stdout);
process.stdout.write = ((chunk: unknown, ...rest: unknown[]) => {
  const text =
    typeof chunk === "string" ? chunk : Buffer.from(chunk as Uint8Array);
  stderr(`${prefix()}${String(text).replace(/\n?$/, "\n")}`);
  const done = rest.find((item) => typeof item === "function");
  if (done) process.nextTick(done as () => void);
  return true;
}) as typeof process.stdout.write;
for (const method of ["log", "info", "debug", "warn", "error"] as const)
  console[method] = logLine;

let closing = false;
process.stdout.on("error", () => void stopAll(0));

/** One line: `{"g":<g>,` followed by the inner message's own keys. */
function post(g: number, line: string) {
  if (closing && g !== 0) return;
  stdout(line === "{}" ? `{"g":${g}}\n` : `{"g":${g},${line.slice(1)}\n`);
}
const control = (event: string, data?: unknown) =>
  post(0, JSON.stringify(data === undefined ? { event } : { event, data }));

function start(data: Start) {
  const { g } = data;
  if (!Number.isSafeInteger(g) || g <= 0 || members.has(g)) {
    logLine(`plugin.start refused: generation ${g} is invalid or in use`);
    return;
  }
  const member: Member = {
    g,
    id: data.id,
    alive: true,
    events: 0,
    handlerMs: 0,
    startedAt: performance.now(),
  };
  members.set(g, member);
  void als.run(member, async () => {
    let manifest;
    try {
      manifest = await readManifest(data.directory);
    } catch (error) {
      logLine(error);
      // Stopped while the manifest was read: the host no longer expects an answer.
      if (!member.alive) return;
      post(
        g,
        JSON.stringify({
          event: "runtime.failed",
          data: { code: "activationFailed", message: String(error) },
        }),
      );
      member.alive = false;
      members.delete(g);
      return;
    }
    if (!member.alive) return;
    const instance = boot({
      directory: data.directory,
      manifest,
      dataDirectory: data.dataDirectory || undefined,
      locale: data.locale || undefined,
      app: data.app ?? null,
      write: (line) => {
        if (member.alive) post(g, line);
      },
      log: logLine,
      trace,
    });
    member.instance = instance;
    const outcome = await instance.done;
    member.readyMs = performance.now() - member.startedAt;
    // A plugin whose activation failed is disposed; the others go on.
    if (outcome === "failed" && member.alive) void retire(member);
  });
}

/**
 * Stops one plugin: its pending requests are rejected and its cleanup runs,
 * for at most the stop deadline; then it leaves the process.
 */
function retire(member: Member): Promise<boolean> {
  member.alive = false;
  const settled = als.run(member, () => member.instance?.stop()) ?? undefined;
  member.stopped ??= Promise.resolve(settled);
  return Promise.race([
    member.stopped.then(() => false),
    new Promise<boolean>((done) =>
      setTimeout(() => done(true), stopTimeout).unref(),
    ),
  ]).then((timedOut) => {
    if (members.get(member.g) === member) members.delete(member.g);
    return timedOut;
  });
}

async function stopOne(g: number) {
  const member = members.get(g);
  const timedOut = member ? await retire(member) : false;
  control("runner.stopped", { g, timedOut });
}

let exiting = false;
async function stopAll(code: number) {
  if (exiting) return;
  exiting = true;
  closing = true;
  setTimeout(() => process.exit(code), stopTimeout).unref();
  await Promise.all([...members.values()].map((member) => retire(member)));
  process.exit(code);
}

function stats() {
  const memory = process.memoryUsage();
  const plugins: Record<string, object> = {};
  for (const member of members.values()) {
    const counts = member.instance?.stats() ?? { requests: 0, pending: 0 };
    plugins[member.g] = {
      id: member.id,
      ...counts,
      events: member.events,
      handlerMs: Math.round(member.handlerMs * 10) / 10,
      readyMs:
        member.readyMs === undefined
          ? null
          : Math.round(member.readyMs * 10) / 10,
    };
  }
  return {
    heapUsed: memory.heapUsed,
    heapTotal: memory.heapTotal,
    rss: memory.rss,
    external: memory.external,
    plugins,
  };
}

function runner(message: Inbound) {
  switch (message.event) {
    case "plugin.start":
      if (typeof message.data !== "object" || message.data === null) {
        logLine("plugin.start without data");
        return;
      }
      start(message.data as Start);
      return;
    case "runner.ping":
      control("runner.pong", {
        seq: (message.data as { seq?: unknown } | undefined)?.seq ?? null,
        stats: stats(),
      });
      return;
    case "runner.stop":
      void stopAll(0);
      return;
    default:
      logLine(`unknown runner message ${String(message.event)}`);
  }
}

function deliver(g: number, message: Inbound) {
  if (message.event === "runtime.stop") {
    void stopOne(g);
    return;
  }
  const member = members.get(g);
  if (!member?.alive || !member.instance) return;
  const instance = member.instance;
  if (message.event) member.events++;
  const begin = performance.now();
  try {
    als.run(member, () => instance.deliver(message));
  } finally {
    member.handlerMs += performance.now() - begin;
  }
}

process.on("uncaughtException", (error) => {
  const member = als.getStore();
  if (member) {
    logLine("uncaught exception:", error);
    // A plugin already stopped (a timer its cleanup left behind) is not
    // reported twice.
    if (!member.alive) return;
    control("runner.fault", { g: member.g, message: String(error) });
    void retire(member);
    return;
  }
  stderr(`[shared] uncaught exception: ${format(error)}\n`);
  control("runner.fault", { g: 0, message: String(error) });
  closing = true;
  process.exit(1);
});
// A forgotten promise must not take any plugin down (never fatal, as in runner.ts).
process.on("unhandledRejection", (error) =>
  logLine("unhandled rejection:", error),
);

const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  let envelope: Inbound & { g?: unknown };
  try {
    envelope = JSON.parse(line) as Inbound & { g?: unknown };
  } catch (error) {
    logLine("unreadable line from the player:", error);
    return;
  }
  if (trace) stderr(`[shared] rpc receive ${line}\n`);
  const { g, ...message } = envelope;
  if (typeof g !== "number") {
    logLine("line from the player without a generation");
    return;
  }
  if (g === 0) runner(message);
  else deliver(g, message);
});
input.on("close", () => void stopAll(0));

control("runner.hello", { pid: process.pid, node: process.versions.node });
