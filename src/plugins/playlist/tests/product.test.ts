import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createSdk } from "../../../sdk/index.ts";
import type { Json } from "../../../sdk/index.ts";
import type {
  SourceIntent,
  SourceOperation,
  SourceState,
} from "../../../sdk/source.ts";
import { activate } from "../main.ts";

const empty = (): SourceState => ({
  kind: "media",
  container: null,
  audioTracks: [],
  stamp: { generation: 0, intentVersion: 0 },
  cycle: 0,
  eventSequence: 1,
  phase: "empty",
  source: null,
  loadingOwner: null,
  terminalReason: null,
  paused: false,
  position: null,
  duration: null,
  capabilities: {
    pause: true,
    seek: true,
    rate: true,
    restart: true,
    close: true,
    snapshotSource: true,
  },
  output: { volume: 100, muted: false },
  rate: 1,
});
type ViewRow = { rowId: string; status: string; current: boolean };
async function until(condition: () => boolean) {
  const deadline = Date.now() + 2000;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("product action did not settle");
    await new Promise((resolve) => setImmediate(resolve));
  }
}
async function product() {
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-product-"));
  const paths = [join(directory, "a.mp4"), join(directory, "b.mp4")];
  for (const path of paths)
    await writeFile(path, "consumer test; no backend decoding claimed");
  const listeners = new Map<string, Set<(data: unknown) => void>>();
  const properties = new Map<string, Json>();
  const submissions: { intent: SourceIntent; id: number }[] = [];
  let state = empty(),
    setCalls = 0;
  const emit = (name: string, value: unknown) => {
    for (const listener of listeners.get(name) ?? []) listener(value);
  };
  const sourceEvent = (next: Partial<SourceState>) => {
    state = { ...state, ...next, eventSequence: state.eventSequence + 1 };
    emit("source.state", state);
  };
  let submit: (
    intent: SourceIntent,
    id: number,
  ) => Promise<SourceOperation> = async (intent, id) => {
    const stamp = { generation: id, intentVersion: id };
    sourceEvent({
      phase: "ready",
      stamp,
      cycle: 1,
      source: intent.kind === "open" ? intent.source : null,
      paused: false,
      terminalReason: null,
    });
    // This event deliberately precedes the acceptance reply.
    return { status: "completed", ticket: { stamp, cycle: 1, owner: 9 } };
  };
  const host = createSdk({
    async request<T>(method: string, params: Json = {}): Promise<T> {
      const data = params as Record<string, Json>;
      let result: unknown;
      if (method === "ui.setMount") {
        properties.set(data.property as string, data.value);
        setCalls++;
      } else if (method === "ui.mount")
        result = { status: "ready", prepareMs: 0, shellGeneration: 2 };
      else if (method === "ui.mountState")
        result = {
          owner: 9,
          preferred: true,
          effective: true,
          reason: null,
          overlay: false,
        };
      else if (method === "commands.register" || method === "commands.update")
        result = { revision: data.revision };
      else if (method === "source.state") result = state;
      else if (method === "source.submit") {
        const intent = data.intent as SourceIntent,
          id = data.operationId as number;
        submissions.push({ intent, id });
        result = await submit(intent, id);
      } else if (method === "dialogs.open")
        result = { cancelled: false, paths };
      else if (
        !["events.subscribe", "events.unsubscribe", "ui.interaction"].includes(
          method,
        )
      )
        throw new Error(`unexpected RPC ${method}`);
      return result as T;
    },
    on(name, listener) {
      const group = listeners.get(name) ?? new Set();
      listeners.set(name, group);
      group.add(listener);
      return () => {
        group.delete(listener);
      };
    },
  });
  const dispose = await activate(host);
  const action = (name: string) =>
    emit("ui.callback", { view: "$mount", callback: "action", args: [name] });
  const rows = () => properties.get("rows") as unknown as ViewRow[];
  return {
    paths,
    submissions,
    sourceEvent,
    action,
    rows,
    properties,
    setSubmit: (next: typeof submit) => {
      submit = next;
    },
    setCalls: () => setCalls,
    async close() {
      await dispose();
      await rm(directory, { recursive: true, force: true });
    },
  };
}

test("real activation actions add without play, accept ready-before-reply, coalesce clocks, and CAS natural next", async () => {
  const app = await product();
  try {
    app.action("add");
    await until(() => app.rows().length === 2);
    assert.equal(app.submissions.length, 0);
    app.action("play");
    await until(() => app.rows()[0].status === "播放中");
    assert.deepEqual(app.submissions[0].intent, {
      kind: "open",
      source: app.paths[0],
      startPaused: false,
    });
    assert.equal(app.rows()[0].current, true);
    await new Promise((resolve) => setImmediate(resolve));
    const calls = app.setCalls();
    for (let position = 0; position < 1000; position++)
      app.sourceEvent({ position });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(
      app.setCalls(),
      calls,
      "position ticks must not rebuild list models or unchanged properties",
    );
    app.setSubmit(async () => ({ status: "failed", error: "stale" }));
    app.sourceEvent({ phase: "ended", terminalReason: "natural" });
    await until(() => app.submissions.length === 2);
    assert.deepEqual(app.submissions[1].intent, {
      kind: "open",
      source: app.paths[1],
      startPaused: false,
      expected: { generation: 1, intentVersion: 1 },
    });
    await until(() => String(app.properties.get("message")).includes("过期"));
    app.sourceEvent({ phase: "ended", terminalReason: "natural" });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(app.submissions.length, 2);
  } finally {
    await app.close();
  }
});

test("late accepted reply cannot reclaim newer external source or resurrect removed row", async () => {
  const app = await product();
  let accept!: (result: SourceOperation) => void;
  try {
    app.action("add");
    await until(() => app.rows().length === 2);
    app.setSubmit(
      () =>
        new Promise((resolve) => {
          accept = resolve;
        }),
    );
    app.action("play");
    await until(() => app.submissions.length === 1);
    app.sourceEvent({
      phase: "ready",
      stamp: { generation: 50, intentVersion: 60 },
      cycle: 1,
      source: app.paths[0],
    });
    app.action("remove");
    await until(() => app.rows().length === 1);
    accept({
      status: "completed",
      ticket: {
        stamp: { generation: 1, intentVersion: 1 },
        cycle: 1,
        owner: 9,
      },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(app.rows().length, 1);
    assert.equal(
      app.rows().some((row) => row.current),
      false,
    );
    app.sourceEvent({ phase: "ended", terminalReason: "natural" });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(app.submissions.length, 1);
  } finally {
    await app.close();
  }
});
