// The shared runner (runtime/shared-runner.ts): several plugins in one Node
// process, one envelope `g` per line (doc/process/rounds/2026-10-shared-runner.md
// §5, §6.4). The host does not start it yet; these tests answer like the host
// will.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";

const runtime = resolve(import.meta.dirname, "../../runtime");
const plugins = resolve(import.meta.dirname, "../../plugins");

interface Line {
  g?: number;
  id?: number;
  method?: string;
  params?: unknown;
  event?: string;
  data?: Record<string, unknown> | null;
  [key: string]: unknown;
}

/** Answers a request: a message to send back, or `null` to hold it. */
type Reply = (line: Line) => object | null;
const revision: Reply = () => ({ result: { revision: 1 } });

const temporary: string[] = [];
test.after(() => {
  for (const directory of temporary)
    rmSync(directory, { recursive: true, force: true });
});
function scratch(prefix: string) {
  const directory = mkdtempSync(join(tmpdir(), prefix));
  temporary.push(directory);
  return directory;
}

/** A plugin package in a temporary directory. */
function fixture(id: string, source: string, capabilities = ["commands"]) {
  const directory = scratch("shared-plugin-");
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({ id, entry: "main.mjs", capabilities }),
  );
  writeFileSync(join(directory, "main.mjs"), source);
  return directory;
}

/** A fake host around one spawned runner (shared or own). */
class Host {
  readonly child: ChildProcessWithoutNullStreams;
  readonly lines: Line[] = [];
  readonly raw: string[] = [];
  stderr = "";
  readonly exited: Promise<number | null>;
  readonly reply: Reply;
  #waiters = new Set<() => void>();
  constructor(
    args: string[],
    reply: Reply = revision,
    env: Record<string, string> = {},
  ) {
    this.reply = reply;
    this.child = spawn(
      process.execPath,
      ["--no-experimental-webstorage", ...args],
      { stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, ...env } },
    );
    this.child.stderr.on("data", (chunk) => {
      this.stderr += String(chunk);
      this.#wake();
    });
    this.exited = new Promise((done) => this.child.once("exit", done));
    void this.exited.then(() => this.#wake());
    createInterface({ input: this.child.stdout }).on("line", (text) => {
      this.raw.push(text);
      const line = JSON.parse(text) as Line;
      this.lines.push(line);
      if (line.id !== undefined && line.method !== undefined) {
        const answer = this.reply(line);
        if (answer) this.send(this.#envelope(line, { id: line.id, ...answer }));
      }
      this.#wake();
    });
  }
  #envelope(line: Line, message: object) {
    return line.g === undefined ? message : { g: line.g, ...message };
  }
  #wake() {
    for (const waiter of this.#waiters) waiter();
  }
  send(message: object) {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }
  start(g: number, id: string, directory: string, extra: object = {}) {
    this.send({
      g: 0,
      event: "plugin.start",
      data: { g, id, directory, locale: "en", app: null, ...extra },
    });
  }
  /** Resolves with the first line (from the start) that matches. */
  wait<T>(check: () => T | undefined, what: string, ms = 15_000): Promise<T> {
    return new Promise((done, fail) => {
      const timer = setTimeout(() => {
        this.#waiters.delete(poll);
        fail(
          new Error(`timed out waiting for ${what}; stderr:\n${this.stderr}`),
        );
      }, ms);
      const poll = () => {
        const found = check();
        if (found === undefined) return;
        clearTimeout(timer);
        this.#waiters.delete(poll);
        done(found);
      };
      this.#waiters.add(poll);
      poll();
    });
  }
  line(match: (line: Line) => boolean, what: string) {
    return this.wait(() => this.lines.find(match), what);
  }
  event(g: number, event: string) {
    return this.line((l) => l.g === g && l.event === event, `${event} g=${g}`);
  }
  control(event: string, g?: number) {
    return this.line(
      (l) =>
        l.g === 0 && l.event === event && (g === undefined || l.data?.g === g),
      `${event}${g === undefined ? "" : ` for g=${g}`}`,
    );
  }
  async ping(seq: number) {
    this.send({ g: 0, event: "runner.ping", data: { seq } });
    return this.line(
      (l) => l.event === "runner.pong" && l.data?.seq === seq,
      `pong ${seq}`,
    );
  }
  of(g: number) {
    return this.lines.filter((line) => line.g === g);
  }
}

const shared = (reply?: Reply, env?: Record<string, string>) =>
  new Host([join(runtime, "shared-runner.ts")], reply, env);

const commandPlugin = (title: string) =>
  `export default async function activate(plugin) { plugin.commands.add({ id: "a", path: ["Tools", ${JSON.stringify(title)}], run() { plugin.osd.show(${JSON.stringify(title)}); } }); }`;

test("the shared runner greets the host and runs two plugins, each on its own generation", async () => {
  const held: Line[] = [];
  // Plugin 2's first request is held, to answer it with the wrong generation first.
  const host = shared((line) => {
    if (line.g === 2 && held.length === 0) {
      held.push(line);
      return null;
    }
    return { result: { revision: 1 } };
  });
  const hello = await host.control("runner.hello");
  assert.equal(hello.data?.pid, host.child.pid);
  assert.equal(hello.data?.node, process.versions.node);
  host.start(
    1,
    "test.one",
    fixture("test.one", commandPlugin("One"), ["commands", "ui.osd"]),
  );
  host.start(
    2,
    "test.two",
    fixture("test.two", commandPlugin("Two"), ["commands", "ui.osd"]),
  );
  await host.event(1, "runtime.ready");
  await host.wait(() => held[0], "plugin 2's first request");
  // A reply for a generation that is not running, or for an id the plugin
  // never sent, reaches nobody.
  host.send({ g: 99, id: held[0]!.id, result: { revision: 1 } });
  host.send({ g: 2, id: 9999, result: { revision: 1 } });
  await host.ping(1);
  assert.ok(!host.of(2).some((line) => line.event === "runtime.ready"));
  host.send({ g: 2, id: held[0]!.id, result: { fullscreen: false } });
  await host.event(2, "runtime.ready");
  // Each plugin's requests carry its own generation and its own id sequence.
  for (const g of [1, 2]) {
    const requests = host.of(g).filter((line) => line.method);
    assert.deepEqual(
      requests.map((line) => [line.id, line.method]),
      [
        [1, "window.state"],
        [2, "commands.register"],
      ],
    );
  }
  // Every line starts with its generation: the host attributes a line from
  // its first bytes (§5.4).
  for (const text of host.raw) assert.match(text, /^\{"g":\d+,/);
  // A command runs in its own plugin.
  host.send({ g: 2, event: "commands.execute", data: { id: "a" } });
  const osd = await host.line((l) => l.method === "osd.show", "osd.show");
  assert.equal(osd.g, 2);
  assert.deepEqual(osd.params, { text: "Two" });
  host.send({ g: 0, event: "runner.stop" });
  assert.equal(await host.exited, 0);
});

test("one plugin failing to activate leaves the other running and the process alive", async () => {
  const host = shared();
  host.start(
    1,
    "test.bad",
    fixture(
      "test.bad",
      "export default async function activate() { throw new Error('nope'); }",
    ),
  );
  host.start(2, "test.good", fixture("test.good", commandPlugin("Good")));
  const failed = await host.event(1, "runtime.failed");
  assert.equal(failed.data?.code, "activationFailed");
  assert.match(String(failed.data?.message), /nope/);
  await host.event(2, "runtime.ready");
  // A plugin directory without a manifest fails that plugin only.
  host.start(3, "test.missing", scratch("shared-empty-"));
  await host.event(3, "runtime.failed");
  // The host stops the failed plugin as for any failure: acknowledged.
  host.send({ g: 1, event: "runtime.stop" });
  assert.deepEqual((await host.control("runner.stopped", 1)).data, {
    g: 1,
    timedOut: false,
  });
  const pong = await host.ping(7);
  const stats = pong.data?.stats as { plugins: Record<string, unknown> };
  assert.deepEqual(Object.keys(stats.plugins), ["2"]);
  assert.match(host.stderr, /\[test\.bad\] Error: nope/);
  host.child.stdin.end();
  assert.equal(await host.exited, 0);
});

test("stopping one plugin runs its cleanup and keeps the other answering", async () => {
  const host = shared();
  const cleanup = (name: string) =>
    `export default async function activate(plugin) {
      plugin.onDispose(() => console.error("${name} disposed"));
      plugin.commands.add({ id: "a", path: ["Tools", "${name}"], run() { plugin.osd.show("${name}"); } });
    }`;
  host.start(
    4,
    "test.first",
    fixture("test.first", cleanup("first"), ["commands", "ui.osd"]),
  );
  host.start(
    5,
    "test.second",
    fixture("test.second", cleanup("second"), ["commands", "ui.osd"]),
  );
  await host.event(4, "runtime.ready");
  await host.event(5, "runtime.ready");
  host.send({ g: 4, event: "runtime.stop" });
  assert.deepEqual((await host.control("runner.stopped", 4)).data, {
    g: 4,
    timedOut: false,
  });
  assert.match(host.stderr, /\[test\.first\] first disposed/);
  // Events for the stopped plugin reach nobody; the other still runs commands.
  host.send({ g: 4, event: "commands.execute", data: { id: "a" } });
  host.send({ g: 5, event: "commands.execute", data: { id: "a" } });
  const osd = await host.line((l) => l.method === "osd.show", "osd.show");
  assert.equal(osd.g, 5);
  host.send({ g: 0, event: "runner.stop" });
  assert.equal(await host.exited, 0);
  assert.match(host.stderr, /\[test\.second\] second disposed/);
  assert.ok(!host.lines.some((l) => l.g === 4 && l.method === "osd.show"));
});

test("a cleanup that never settles is abandoned after the stop deadline", async () => {
  const host = shared(revision, { VESTRA_SHARED_STOP_MS: "300" });
  host.start(
    1,
    "test.stuck",
    fixture(
      "test.stuck",
      "export default async function activate(plugin) { plugin.onDispose(() => new Promise(() => {})); }",
      [],
    ),
  );
  await host.event(1, "runtime.ready");
  host.send({ g: 1, event: "runtime.stop" });
  assert.deepEqual((await host.control("runner.stopped", 1)).data, {
    g: 1,
    timedOut: true,
  });
  await host.ping(2);
  host.child.stdin.end();
  assert.equal(await host.exited, 0);
});

test("logs carry the plugin's id, stdout writes go to stderr, oversize requests are refused", async () => {
  const host = shared(() => ({ result: null }));
  host.start(
    1,
    "test.loud",
    fixture(
      "test.loud",
      `export default async function activate(plugin) {
        console.log("hello %s", "log");
        console.warn("careful");
        process.stdout.write("junk without newline");
        Promise.reject(new Error("forgotten"));
        const code = await plugin.osd.show("x".repeat(1024 * 1024)).then(() => "sent", (e) => e.code);
        await plugin.osd.show(code);
      }`,
      ["ui.osd"],
    ),
  );
  await host.event(1, "runtime.ready");
  assert.match(host.stderr, /^\[test\.loud\] hello log$/m);
  assert.match(host.stderr, /^\[test\.loud\] careful$/m);
  assert.match(host.stderr, /^\[test\.loud\] junk without newline$/m);
  assert.match(
    host.stderr,
    /^\[test\.loud\] unhandled rejection: Error: forgotten/m,
  );
  assert.deepEqual(
    host
      .of(1)
      .filter((l) => l.method)
      .map((l) => [l.method, l.params]),
    [
      ["window.state", {}],
      ["osd.show", { text: "limit" }],
    ],
  );
  // Nothing but protocol lines on stdout.
  for (const text of host.raw) assert.match(text, /^\{"g":\d+,/);
  host.send({ g: 0, event: "runner.stop" });
  assert.equal(await host.exited, 0);
});

test("an uncaught exception in a plugin's own callback stops that plugin only", async () => {
  const host = shared();
  host.start(
    1,
    "test.thrower",
    fixture(
      "test.thrower",
      `export default async function activate() {
        setTimeout(() => { throw new Error('late'); }, 50);
        // Left behind by the first fault: logged, not reported again.
        setTimeout(() => { throw new Error('later'); }, 150);
      }`,
      [],
    ),
  );
  host.start(
    2,
    "test.calm",
    fixture("test.calm", commandPlugin("Calm"), ["commands", "ui.osd"]),
  );
  await host.event(1, "runtime.ready");
  await host.event(2, "runtime.ready");
  const fault = await host.control("runner.fault", 1);
  assert.match(String(fault.data?.message), /late/);
  host.send({ g: 2, event: "commands.execute", data: { id: "a" } });
  assert.equal(
    (await host.line((l) => l.method === "osd.show", "osd.show")).g,
    2,
  );
  const stats = (await host.ping(3)).data?.stats as {
    plugins: Record<string, unknown>;
  };
  assert.deepEqual(Object.keys(stats.plugins), ["2"]);
  assert.match(
    host.stderr,
    /\[test\.thrower\] uncaught exception: Error: late/,
  );
  await host.wait(
    () => (/Error: later/.test(host.stderr) ? true : undefined),
    "the second throw",
  );
  await host.ping(4);
  assert.equal(host.lines.filter((l) => l.event === "runner.fault").length, 1);
  host.send({ g: 0, event: "runner.stop" });
  assert.equal(await host.exited, 0);
});

test("an uncaught exception no plugin owns ends the shared runner", async () => {
  const host = shared();
  // A listener on a process-wide emitter runs in the emitter's context, not
  // the plugin's: the runner cannot tell whose it is.
  host.start(
    1,
    "test.global",
    fixture(
      "test.global",
      "export default async function activate() { process.stdin.on('data', () => { throw new Error('orphan'); }); }",
      [],
    ),
  );
  await host.event(1, "runtime.ready");
  await host.ping(1);
  const fault = await host.control("runner.fault", 0);
  assert.match(String(fault.data?.message), /orphan/);
  assert.equal(await host.exited, 1);
});

test("the staged shared runner runs without TypeScript", async () => {
  const staged = join(scratch("runtime-"), "runtime");
  const stage = spawnSync(
    process.execPath,
    [resolve(import.meta.dirname, "../../scripts/stage-runtime.mjs"), staged],
    { encoding: "utf8" },
  );
  assert.equal(stage.status, 0, stage.stderr);
  assert.doesNotMatch(stage.stderr, /ExperimentalWarning/);
  const host = new Host([join(staged, "shared-runner.js")]);
  host.start(1, "test.staged", fixture("test.staged", commandPlugin("Staged")));
  await host.event(1, "runtime.ready");
  host.send({ g: 0, event: "runner.stop" });
  assert.equal(await host.exited, 0);
});

// The plugin API does not depend on the runner: a real built-in plugin
// writes the same messages in its own process and in the shared one, given
// the same answers (decision 3 of the design: the `plugin` object and the
// manifest do not change).
const EMPTY_SOURCE = {
  kind: "media",
  container: null,
  audioTracks: null,
  stamp: { generation: 0, intentVersion: 0 },
  cycle: 0,
  eventSequence: 0,
  phase: "empty",
  source: null,
  loadingOwner: null,
  terminalReason: null,
  paused: false,
  position: null,
  duration: null,
  capabilities: {
    pause: false,
    seek: false,
    rate: false,
    restart: false,
    close: false,
    snapshotSource: false,
  },
  output: { volume: 100, muted: false },
  rate: 1,
};
const ANSWERS: Record<string, unknown> = {
  "source.state": EMPTY_SOURCE,
  "window.state": {
    fullscreen: false,
    videoArea: { width: 780, height: 609 },
    pictureInPicture: { supported: true, active: false },
    appearance: "vestra",
    fileTypes: [],
    language: "system",
    locale: "en",
  },
  "timeline.marks": { marks: [] },
  "layout.state": { bottom: null, sidebar: null, backdrop: null },
  "processing.state": {
    configurationRevision: 0,
    audioOffsetMs: 0,
    source: { generation: 0, phase: "empty", load: null, stages: {} },
    video: [],
    audio: [],
    backendUncertain: {},
  },
  "video.view.set": { state: "waiting" },
  "video.render.set": { state: "waiting" },
  "ui.mount": { status: "ready", prepareMs: 0, shellGeneration: 1 },
  "commands.register": { revision: 1 },
};
const answer: Reply = (line) => ({ result: ANSWERS[line.method!] ?? null });

/** A plugin's messages without the envelope and in a comparable form. */
const inner = (lines: Line[]) =>
  lines.map(({ g: _, ...message }) => JSON.stringify(message));

for (const name of ["video", "playlist"]) {
  test(`built-in ${name} writes the same messages in its own process and in the shared runner`, async () => {
    const directory = join(plugins, name);
    const own = new Host([join(runtime, "runner.ts"), directory], answer, {
      VESTRA_LOCALE: "en",
      VESTRA_APP_INFO: "null",
      VESTRA_PLUGIN_DATA_DIR: scratch("own-data-"),
    });
    await own.line((l) => l.event === "runtime.ready", "own ready");
    own.child.stdin.end();
    assert.equal(await own.exited, 0);
    // A real start-up: state reads, subscriptions, registrations.
    const methods = new Set(own.lines.map((l) => l.method));
    assert.ok(methods.has("commands.register"), [...methods].join());
    assert.ok(methods.size >= 4, [...methods].join());

    const host = shared(answer);
    // A neighbour in the same process changes nothing for the plugin.
    host.start(
      1,
      "test.neighbour",
      fixture("test.neighbour", commandPlugin("N")),
    );
    host.start(7, `vestra.${name}`, directory, {
      dataDirectory: scratch("shared-data-"),
    });
    await host.event(7, "runtime.ready");
    await host.event(1, "runtime.ready");
    assert.deepEqual(inner(host.of(7)), inner(own.lines));
    host.send({ g: 0, event: "runner.stop" });
    assert.equal(await host.exited, 0);
  });
}
