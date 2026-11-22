import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { settleFiles } from "./settle.ts";
import activate from "../main.ts";

/**
 * Files another plugin's file resolver (§11) turns into generated streams. The
 * TestHost only simulates this plugin's own resolvers, so the player side is
 * played here: an open of a `.game` file is accepted as `pending` (loading, the
 * path as source) until the test answers it; other opens complete at once.
 * `.game` stands for any extension some plugin resolves; the playlist knows none.
 */
type Intent = { kind: string; source?: string; expected?: Json };
const CANCELLED = "source resolution cancelled, expired or superseded";
const pause = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

async function start() {
  const root = await realpath(await mkdtemp(join(tmpdir(), "vestra-playlist-streams-")));
  const [a, game, b] = ["a.mp4", "game.game", "b.mp4"].map((name) => join(root, name));
  for (const file of [a!, game!, b!]) await writeFile(file, "x");
  const host = new TestHost();
  settleFiles(host);
  host.tools.handle("ffprobe", () => {
    throw new HostError("failed", "no duration");
  });
  const { stop } = await startPlugin((plugin) => activate(plugin), {
    directory: new URL("..", import.meta.url),
    host,
    locale: "zh-Hans",
  });
  const panel = host.mounted();
  // `PLAYLIST_TEST_HIDDEN=1`: the list is closed after the start, so the plugin hears the
  // source events instead of a stream (features.test.ts runs its own cases both ways).
  if (process.env.PLAYLIST_TEST_HIDDEN) {
    await host.command("playlist.close").run();
    await host.settle();
  }

  const book = new Map<number, Json>();
  let resolving: number | null = null;
  const ticket = () => {
    const { stamp, cycle } = host.source;
    return { stamp: { ...stamp }, cycle, owner: 1 };
  };
  host.handle("source.submit", (p) => {
    const intent = p.intent as Intent;
    const operation = Number(p.operationId);
    if (intent.kind !== "open") throw new Error(`unexpected ${intent.kind}`);
    host.openElsewhere(intent.source!, "loading");
    const result = { status: intent.source!.endsWith(".game") ? "pending" : "completed", ticket: ticket() };
    if (result.status === "pending") resolving = operation;
    book.set(operation, result);
    return result;
  });
  host.handle("source.operation", (p) => {
    const result = book.get(Number(p.operationId));
    if (!result) throw new HostError("unknown-expired", "UnknownExpired");
    return result;
  });
  const answer = (result: Json) => {
    assert.ok(resolving !== null, "a resolution is pending");
    book.set(resolving, result);
    resolving = null;
  };

  const app = {
    host,
    a: a!,
    game: game!,
    b: b!,
    rows: () => (panel.props.rows ?? []) as unknown as { label: string; current: boolean; failed: boolean; tooltip: string }[],
    current: () => app.rows().findIndex((row) => row.current),
    opens: () => host.sent("source.submit").map((call) => (call.intent as unknown as Intent).source),
    osd: () => host.osd.at(-1),
    until: async (check: () => boolean, what: string) => {
      for (const deadline = Date.now() + 15_000; !check(); await pause(5))
        if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    },
    /** Wait until the plugin's open of the `.game` file reached the player. */
    resolving: () => app.until(() => resolving !== null, "the resolution"),
    /** The resolver answered: the player selects its stream; the source keeps the path. */
    started: async () => {
      host.setSource({
        kind: "producer",
        phase: "ready",
        capabilities: { pause: true, seek: false, rate: false, restart: false, close: true, snapshotSource: false },
      });
      answer({ status: "completed", ticket: ticket() });
      await app.until(() => app.current() === 1, "the stream row is current");
      await host.settle();
    },
    /** The resolver failed with its own message. */
    failed: async (message: string) => {
      host.setSource({ phase: "error", terminalReason: { failed: message } });
      answer({ status: "failed", error: { backend: message } });
      await app.until(() => app.rows()[1]!.failed, "the row is marked failed");
      await host.settle();
    },
    /** The user closed the source (F4) while it resolved. */
    closed: async () => {
      const { stamp } = host.source;
      host.setSource({
        stamp: { ...stamp, intentVersion: stamp.intentVersion + 1 },
        phase: "empty",
        source: null,
        terminalReason: "stopped",
      });
      answer({ status: "failed", error: { backend: CANCELLED } });
      await pause(700); // the SDK polls the operation with a back-off
      await host.settle();
    },
    addFiles: async (paths: string[]) => {
      const notices = host.osd.length;
      host.dialogAnswers.push(paths);
      await host.command("playlist.add").run();
      await app.until(() => host.osd.length > notices, "the add notice");
      await host.settle();
    },
    play: async (index: number) => {
      panel.emit("play-row", (app.rows()[index] as unknown as { id: string }).id);
      await pause();
    },
    async close() {
      await stop();
      await rm(root, { recursive: true, force: true });
    },
  };
  return app;
}

test("R1: a mixed list keeps its current row and next / previous through a resolved stream", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.game, app.b]);
    await app.play(0);
    await app.until(() => app.current() === 0, "a is current");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();

    // a ends naturally: the list advances to the file a plugin resolves.
    app.host.setSource({ phase: "ended", terminalReason: "natural" });
    await app.resolving();
    await app.started();
    assert.equal(app.rows()[1]!.failed, false);
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: true });

    // next and previous step from the stream row like from any other.
    await app.host.step("next");
    await app.until(() => app.current() === 2, "b is current");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    void app.host.step("previous");
    await app.resolving();
    await app.started();
    await app.host.step("previous");
    await app.until(() => app.current() === 0, "a is current again");
    assert.deepEqual(app.opens(), [app.a, app.game, app.b, app.game, app.a]);
  } finally {
    await app.close();
  }
});

test("R1: a resolver's failure marks the row failed with its message; next goes on from it", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.game, app.b]);
    await app.play(1);
    await app.resolving();
    await app.failed("不是有效的 ROM");
    assert.match(app.rows()[1]!.tooltip, /不是有效的 ROM/);
    assert.equal(app.osd(), "无法打开 game.game：不是有效的 ROM");
    await app.host.step("next");
    await app.until(() => app.current() === 2, "b is current");
  } finally {
    await app.close();
  }
});

test("R1: closing the source while it resolves is no failure of the row", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.game, app.b]);
    await app.play(1);
    await app.resolving();
    const notices = app.host.osd.length;
    await app.closed();
    assert.equal(app.rows()[1]!.failed, false);
    assert.equal(app.current(), -1);
    assert.ok(!app.host.osd.slice(notices).some((text) => text.startsWith("无法打开")));
    // The row can be played again.
    await app.play(1);
    await app.resolving();
    await app.started();
  } finally {
    await app.close();
  }
});

test("R1 / A13: a resolved stream another plugin opened is adopted like a file", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a]);
    // Seen only once it is a stream (e.g. this plugin was busy while it resolved).
    const { stamp, cycle } = app.host.source;
    app.host.setSource({
      stamp: { generation: stamp.generation + 1, intentVersion: stamp.intentVersion + 1 },
      cycle: cycle + 1,
      kind: "producer",
      source: app.game,
      phase: "ready",
      capabilities: { pause: true, seek: false, rate: false, restart: false, close: true, snapshotSource: false },
    });
    await app.until(() => app.rows().length === 1 && app.rows()[0]!.label.endsWith("game.game"), "the list follows");
    await app.until(() => app.current() === 0, "the stream row is adopted");
    assert.deepEqual(app.opens(), [], "this plugin opened nothing");
    // An opaque openStream source is never added.
    const next = app.host.source;
    app.host.setSource({
      stamp: { generation: next.stamp.generation + 1, intentVersion: next.stamp.intentVersion + 1 },
      cycle: next.cycle + 1,
      kind: "producer",
      source: "producer://7",
      phase: "ready",
    });
    await app.until(() => app.current() === -1, "the adoption ends");
    await app.host.settle(30);
    assert.equal(app.rows().length, 1);
  } finally {
    await app.close();
  }
});
