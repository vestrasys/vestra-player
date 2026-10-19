import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Host, Json, UiCallback } from "../../../sdk/index.ts";
import type { SourceIntent, SourceOperation, SourceState } from "../../../sdk/source.ts";
import { activate } from "../main.ts";

type ViewRow = { rowId: string; label: string; status: string; current: boolean; selected: boolean };
const initial = (): SourceState => ({
  kind: "media", container: null, audioTracks: [], stamp: { generation: 0, intentVersion: 0 },
  cycle: 0, eventSequence: 1, phase: "empty", source: null, loadingOwner: null,
  terminalReason: null, paused: false, position: null, duration: null,
  capabilities: { pause: true, seek: true, rate: true, restart: true, close: true, snapshotSource: true },
  output: { volume: 100, muted: false }, rate: 1,
});
async function until(condition: () => boolean, timeout = 3000) {
  const deadline = Date.now() + timeout;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("consumer action did not settle");
    await new Promise(resolve => setTimeout(resolve, 2));
  }
}
async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-site01-"));
  const first = join(directory, "a.mp4"), second = join(directory, "b.mp4"), alias = join(directory, "alias.mp4");
  await writeFile(first, "consumer test, no media decode claim");
  await writeFile(second, "consumer test, no media decode claim"); await symlink(first, alias);
  let state = initial(), callback: (event: UiCallback) => void = () => {};
  const listeners = new Map<string, (value: unknown) => void>();
  const values = new Map<string, Json>();
  const submissions: { intent: SourceIntent; id: number }[] = [];
  const queries: number[] = [];
  let propertyCalls = 0;
  const event = (patch: Partial<SourceState>) => {
    state = { ...state, ...patch, eventSequence: state.eventSequence + 1 };
    listeners.get("source.state")?.(state);
  };
  let onSubmit = async (intent: SourceIntent, id: number): Promise<SourceOperation> => {
    const stamp = { generation: id, intentVersion: id };
    event({ stamp, cycle: 1, phase: "ready", source: intent.kind === "open" ? intent.source : null, terminalReason: null });
    return { status: "completed", ticket: { stamp, cycle: 1, owner: 42 } };
  };
  let onQuery = async (_id: number): Promise<SourceOperation> => { throw new Error("unexpected operation query"); };
  const host = {
    ui: {
      mount: async () => ({ status: "ready" }),
      mountState: async () => ({ owner: 42, preferred: true, effective: true, reason: null, overlay: false }),
      setMount: async (key: string, value: Json) => { values.set(key, value); propertyCalls++; },
      onCallback: (listener: typeof callback) => { callback = listener; return () => { callback = () => {}; }; },
      set: async () => {}, show: async () => {}, open: async () => {}, interaction: async () => {},
    },
    commands: { register: async () => ({}), update: async () => ({}), onExecute: () => () => {} },
    source: {
      state: async () => state,
      submit: async (intent: SourceIntent, id: number) => { submissions.push({ intent, id }); return onSubmit(intent, id); },
      operation: async (id: number) => { queries.push(id); return onQuery(id); },
    },
    dialogs: { open: async () => ({ cancelled: false, paths: [first, alias, second] }) },
    events: { subscribe: async (name: string, listener: (value: unknown) => void) => { listeners.set(name, listener); return () => { listeners.delete(name); }; } },
  } as unknown as Host;
  const dispose = await activate(host);
  const invoke = (view: string, name: string, args: Json[]) => callback({ view, callback: name, args });
  return {
    values, submissions, queries, event, first, second,
    rows: () => values.get("rows") as unknown as ViewRow[],
    addUrl: (url: string) => invoke("playlist-link", "add-link", [url]),
    action: (name: string) => invoke("$mount", "action", [name]),
    select: (id: string) => invoke("$mount", "select-row", [id]),
    calls: () => propertyCalls,
    submitWith: (handler: typeof onSubmit) => { onSubmit = handler; },
    queryWith: (handler: typeof onQuery) => { onQuery = handler; },
    close: async () => { await dispose(); await rm(directory, { recursive: true, force: true }); },
  };
}

test("URL addition is selection only; public routing receives original URL, pending/ready and natural next uses CAS", async () => {
  const app = await fixture();
  try {
    const a = "https://example.org/watch?v=one", b = "https://example.org/watch?v=two";
    app.addUrl(a); app.addUrl(b); await until(() => app.rows().length === 2);
    const id = app.rows()[0].rowId; app.select(id);
    assert.equal(app.submissions.length, 0);
    app.submitWith(async (intent, operationId) => {
      const stamp = { generation: operationId, intentVersion: operationId };
      app.event({ stamp, cycle: 1, phase: "loading", source: intent.kind === "open" ? intent.source : null });
      return { status: "pending", ticket: { stamp, cycle: 1, owner: 42 } };
    });
    app.action("play"); app.action("play");
    await until(() => app.submissions.length === 1);
    assert.deepEqual(app.submissions[0].intent, { kind: "open", source: a, startPaused: false });
    await until(() => app.rows()[0].status === "正在打开");
    app.event({ phase: "ready" }); await until(() => app.rows()[0].status === "播放中");
    // The rows property is projected before the remaining button properties.
    await new Promise(resolve => setImmediate(resolve));
    const calls = app.calls();
    for (let position = 0; position < 1000; position++) app.event({ position });
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(app.calls(), calls, "clock ticks do not rebuild unchanged list models");
    app.event({ phase: "ended", terminalReason: "natural" });
    await until(() => app.submissions.length === 2);
    assert.deepEqual(app.submissions[1].intent, { kind: "open", source: b, startPaused: false, expected: { generation: 1, intentVersion: 1 } });
    assert.equal(app.rows().find(row => row.rowId === id)?.selected, true, "auto-next keeps selection");
  } finally { await app.close(); }
});

test("provider absence is the host's real error; failed row remains and is retryable", async () => {
  const app = await fixture();
  try {
    app.addUrl("https://example.org/unavailable"); await until(() => app.rows().length === 1);
    const id = app.rows()[0].rowId;
    app.submitWith(async () => ({ status: "failed", error: { backend: "resolverUnavailable" } }));
    app.action("play"); await until(() => app.rows()[0].status === "失败");
    assert.equal(app.submissions.length, 1);
    assert.match(String(app.values.get("details")), /resolverUnavailable/);
    app.action("play"); await until(() => app.submissions.length === 2);
    assert.equal(app.rows()[0].rowId, id); assert.equal(app.rows().length, 1);
  } finally { await app.close(); }
});

test("late URL ticket loses to newer source and removed pending row never resurrects", async () => {
  const app = await fixture();
  let accept!: (result: SourceOperation) => void;
  try {
    app.addUrl("https://example.org/a"); app.addUrl("https://example.org/b");
    await until(() => app.rows().length === 2);
    app.submitWith(() => new Promise(resolve => { accept = resolve; }));
    app.action("play"); await until(() => app.submissions.length === 1);
    app.event({ phase: "ready", stamp: { generation: 100, intentVersion: 100 }, cycle: 1, source: "/external" });
    app.action("remove"); await until(() => app.rows().length === 1);
    accept({ status: "completed", ticket: { stamp: { generation: 1, intentVersion: 1 }, cycle: 1, owner: 42 } });
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(app.rows().length, 1); assert.equal(app.rows().some(row => row.current), false);
    app.event({ phase: "ended", terminalReason: "natural" });
    await new Promise(resolve => setTimeout(resolve, 10)); assert.equal(app.submissions.length, 1);
  } finally { await app.close(); }
});

test("timed-out URL opens query the same operation without resubmitting", async () => {
  const app = await fixture();
  try {
    app.addUrl("https://example.org/a"); await until(() => app.rows().length === 1);
    app.submitWith(async () => { throw Object.assign(new Error("outcome unknown"), { code: "timeout" }); });
    app.queryWith(async id => {
      const stamp = { generation: id, intentVersion: id };
      app.event({ phase: "ready", stamp, cycle: 1, source: "https://example.org/a" });
      return { status: "completed", ticket: { stamp, cycle: 1, owner: 42 } };
    });
    app.action("play"); await until(() => String(app.values.get("message")).includes("结果未知"));
    app.action("play");
    await until(() => app.rows()[0].status === "播放中");
    assert.equal(app.submissions.length, 1); assert.deepEqual(app.queries, [app.submissions[0].id]);
  } finally { await app.close(); }
});

test("local file realpath dedup and explicit startPaused false remain intact", async () => {
  const app = await fixture();
  try {
    app.action("add"); await until(() => app.rows().length === 2);
    assert.equal(app.submissions.length, 0);
    app.action("play"); await until(() => app.rows()[0].status === "播放中");
    assert.deepEqual(app.submissions[0].intent, { kind: "open", source: app.first, startPaused: false });
  } finally { await app.close(); }
});
