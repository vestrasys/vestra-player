import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Queue } from "../queue.ts";
import type { SourceState, SourceTicket } from "../../../sdk/source.ts";

const ticket: SourceTicket = {
  stamp: { generation: 4, intentVersion: 7 },
  cycle: 1,
  owner: 12,
};
function state(overrides: Partial<SourceState> = {}): SourceState {
  return {
    kind: "media",
    container: null,
    audioTracks: [],
    stamp: ticket.stamp,
    cycle: 1,
    eventSequence: 5,
    phase: "ready",
    source: "/a",
    loadingOwner: null,
    terminalReason: null,
    paused: false,
    position: 0,
    duration: 2,
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
    ...overrides,
  };
}
function fixture() {
  const queue = new Queue();
  for (const name of ["a", "b", "c", "d"])
    queue.addUrl(`https://example.org/${name}`);
  return queue;
}

test("realpath deduplicates same batch and symlink while inaccessible rows remain retryable", async () => {
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-"));
  try {
    const path = join(directory, "a.mp4"),
      alias = join(directory, "alias.mp4");
    await writeFile(path, "not decoded by plugin");
    await symlink(path, alias);
    const queue = new Queue();
    await queue.addFiles([path, path, alias, join(directory, "missing.mp4")]);
    assert.equal(queue.rows.length, 2);
    assert.equal(queue.selected, queue.rows[0].rowId);
    assert.equal(queue.binding, null);
    assert.match(queue.rows[1].error, /文件不存在/);
    assert.match(queue.message, /新增 2 项，跳过 2 个重复/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("link addition preserves original URL, selects without binding, and rejects credentials/non-http", () => {
  const queue = new Queue();
  queue.addUrl("https://example.org/a?x=1");
  assert.equal(queue.rows[0].source, "https://example.org/a?x=1");
  assert.equal(queue.selected, queue.rows[0].rowId);
  assert.equal(queue.binding, null);
  queue.addUrl("https://example.org/a?x=1");
  assert.equal(queue.rows.length, 1);
  assert.throws(() => queue.addUrl("file:///tmp/a"));
  assert.throws(() => queue.addUrl("https://user:password@example.org/a"));
});

test("failed navigation anchor survives a different selection, movement uses stable IDs", () => {
  const queue = fixture();
  const [, b, c, d] = queue.rows;
  queue.fail(b.rowId, "decode failure");
  queue.select(d.rowId);
  assert.equal(queue.neighbor(1)?.rowId, c.rowId);
  queue.move(-1);
  assert.equal(queue.row(d.rowId), d);
  assert.equal(queue.neighbor(1)?.rowId, d.rowId);
  queue.select(b.rowId);
  queue.remove();
  assert.equal(queue.failed, null);
  assert.equal(queue.selected, d.rowId);
});

test("natural end once per cycle; restart invalidates old token and renews end eligibility", () => {
  const queue = fixture(),
    row = queue.rows[0];
  queue.binding = { rowId: row.rowId, ticket, endedCycle: null };
  const first = queue.observe(
    state({ phase: "ended", terminalReason: "natural" }),
  );
  assert.ok(first);
  assert.equal(
    queue.observe(state({ phase: "ended", terminalReason: "natural" })),
    null,
  );
  const restarted = { generation: 4, intentVersion: 8 };
  queue.observe(state({ stamp: restarted, cycle: 2 }));
  assert.notEqual(queue.epoch, first.epoch);
  const second = queue.observe(
    state({
      stamp: restarted,
      cycle: 2,
      phase: "ended",
      terminalReason: "natural",
    }),
  );
  assert.ok(second);
  assert.equal(second.cycle, 2);
});

test("error/replacement/stopped never auto advance; external same path is detached by stamp", () => {
  for (const reason of ["stopped", "replaced", { failed: "broken" }] as const) {
    const queue = fixture();
    queue.binding = { rowId: queue.rows[0].rowId, ticket, endedCycle: null };
    assert.equal(
      queue.observe(
        state({
          phase: reason === "stopped" ? "ended" : "error",
          terminalReason: reason,
        }),
      ),
      null,
    );
  }
  const queue = fixture();
  queue.binding = { rowId: queue.rows[0].rowId, ticket, endedCycle: null };
  queue.observe(
    state({ stamp: { generation: 5, intentVersion: 9 }, source: "/a" }),
  );
  assert.equal(queue.binding, null);
});

test("removing current and clearing revoke progression without requesting public media changes", () => {
  const queue = fixture();
  queue.select(queue.rows[1].rowId);
  queue.binding = { rowId: queue.selected!, ticket, endedCycle: null };
  const epoch = queue.epoch;
  queue.remove();
  assert.equal(queue.binding, null);
  assert.ok(queue.epoch > epoch);
  assert.equal(queue.selected, queue.rows[1].rowId);
  assert.match(queue.message, /当前媒体已移出/);
  queue.clear();
  assert.equal(queue.rows.length, 0);
  assert.equal(queue.selected, null);
  assert.match(queue.message, /当前媒体继续/);
});
