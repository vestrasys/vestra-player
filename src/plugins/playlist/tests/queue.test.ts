import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Media, PlayerState } from "@vestra-player/sdk";
import { type LinkError, Queue } from "../queue.ts";

function state(overrides: Partial<PlayerState> = {}): PlayerState {
  return {
    sourceId: 4,
    kind: "media",
    phase: "ready",
    source: "/a",
    paused: false,
    position: 0,
    duration: 2,
    rate: 1,
    volume: 100,
    muted: false,
    endReason: null,
    error: null,
    can: { pause: true, seek: true, rate: true, restart: true, close: true, snapshot: true, next: false, previous: false },
    audioTracks: null,
    ...overrides,
  };
}
function media(current = true): Media & { current: boolean } {
  return { current, ready: async () => {}, onEnd: () => ({ dispose: async () => {} }), close: async () => {} };
}
function fixture() {
  const queue = new Queue();
  for (const name of ["a", "b", "c", "d"]) queue.addUrl(`https://example.org/${name}`);
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
    const said: unknown[] = [];
    queue.say = (text, args) => void said.push([text, args]);
    await queue.addFiles([path, path, alias, join(directory, "missing.mp4")]);
    assert.equal(queue.rows.length, 2);
    assert.equal(queue.selected, queue.rows[0].rowId);
    assert.equal(queue.binding, null);
    assert.deepEqual(queue.rows[1].error, { kind: "unreadable" });
    assert.equal(queue.rows[1].status, "retry");
    // 已添加 2 项，跳过 2 个重复，1 项暂不可读 in Simplified Chinese.
    assert.deepEqual(said, [
      [
        "Added: {added}, duplicates skipped: {duplicates}, not readable for now: {count}",
        { added: 2, duplicates: 2, count: 1 },
      ],
    ]);
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
  assert.equal(queue.message, "Existing link selected");
  assert.throws(() => queue.addUrl("file:///tmp/a"), (error: LinkError) => error.code === "refused");
  assert.throws(() => queue.addUrl("https://user:password@example.org/a"), (error: LinkError) => error.code === "refused");
  assert.throws(() => queue.addUrl("not a url"), (error: LinkError) => error.code === "incomplete");
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
  queue.removeSelected();
  assert.equal(queue.failed, null);
  assert.equal(queue.selected, d.rowId);
});

test("bound row follows ready, paused, loading, ended, error and closed states", () => {
  const queue = fixture(),
    row = queue.rows[0];
  queue.bind({ rowId: row.rowId, media: media() });
  queue.observe(state({ phase: "loading" }));
  assert.equal(row.status, "opening");
  queue.observe(state({ paused: true }));
  assert.equal(row.status, "paused");
  assert.equal(queue.message, "paused");
  queue.observe(state());
  assert.equal(row.status, "playing");
  queue.observe(state({ phase: "ended", endReason: "natural" }));
  assert.equal(row.status, "ended");
  assert.equal(queue.binding?.rowId, row.rowId, "the natural end itself is handled by media.onEnd");
  queue.observe(state({ phase: "error", error: "broken" }));
  assert.equal(row.status, "failed");
  assert.deepEqual(row.error, { kind: "message", text: "broken" });
  assert.equal(queue.failed, row.rowId);
  assert.equal(queue.binding, null);

  queue.bind({ rowId: row.rowId, media: media() });
  queue.observe(state({ phase: "empty" }));
  assert.equal(queue.binding, null);
  assert.equal(queue.message, "The current source was closed");
});

test("another source, even of the same path, detaches the row and releases its end listener", () => {
  const queue = fixture();
  let released = false;
  const replaced = media();
  queue.bind({ rowId: queue.rows[0].rowId, media: replaced, end: { dispose: async () => void (released = true) } });
  replaced.current = false;
  queue.observe(state({ sourceId: 5, source: "/a" }));
  assert.equal(queue.binding, null);
  assert.equal(queue.rows[0].status, "switched");
  assert.equal(queue.message, "Switched to a source outside the list");
  assert.equal(released, true);
});

test("removing current and clearing revoke progression without requesting public media changes", () => {
  const queue = fixture();
  queue.select(queue.rows[1].rowId);
  queue.bind({ rowId: queue.selected!, media: media() });
  const epoch = queue.epoch;
  queue.removeSelected();
  assert.equal(queue.binding, null);
  assert.ok(queue.epoch > epoch);
  assert.equal(queue.selected, queue.rows[1].rowId);
  assert.equal(queue.message, "The current media was removed from the list");
  queue.clear();
  assert.equal(queue.rows.length, 0);
  assert.equal(queue.selected, null);
  assert.equal(queue.message, "List cleared; the current media continues");
});

const names = (queue: Queue) => queue.rows.map((row) => row.source.slice(-1));

test("multi-select: Ctrl toggles, Shift extends from the anchor, select-all keeps the cursor", () => {
  const queue = fixture();
  const [a, b, c, d] = queue.rows.map((row) => row.rowId);
  queue.select(b);
  queue.extend(d);
  assert.deepEqual([...queue.selection].sort(), [b, c, d]);
  assert.equal(queue.selected, d);
  queue.toggle(c);
  assert.deepEqual(queue.marked().map((row) => row.rowId), [b, d]);
  queue.select(a);
  queue.selectNeighbor(2, true);
  assert.deepEqual(queue.marked().map((row) => row.rowId), [a, b, c]);
  queue.selectAll();
  assert.equal(queue.selection.size, 4);
  assert.equal(queue.selected, c);
});

test("placement moves the marked block and keeps its order; drop inserts before an index", () => {
  const queue = fixture();
  const [a, , c] = queue.rows.map((row) => row.rowId);
  queue.select(a);
  queue.toggle(c);
  assert.equal(queue.canPlace("up"), true, "c can still move up");
  queue.place("up");
  assert.deepEqual(names(queue), ["a", "c", "b", "d"]);
  queue.place("bottom");
  assert.deepEqual(names(queue), ["b", "d", "a", "c"]);
  assert.equal(queue.canPlace("down"), false);
  queue.place("top");
  assert.deepEqual(names(queue), ["a", "c", "b", "d"]);
  queue.place("down");
  assert.deepEqual(names(queue), ["b", "a", "c", "d"]);
  // Dragging a marked row moves the whole selection; an unmarked row moves alone.
  queue.drop(a, 0);
  assert.deepEqual(names(queue), ["a", "c", "b", "d"]);
  queue.drop(queue.rows[2].rowId, 4);
  assert.deepEqual(names(queue), ["a", "c", "d", "b"]);
  assert.deepEqual(queue.marked().map((row) => row.source.slice(-1)), ["b"]);
  queue.sort("reverse");
  assert.deepEqual(names(queue), ["b", "d", "c", "a"]);
  queue.sort("title");
  assert.deepEqual(names(queue), ["a", "b", "c", "d"]);
});

test("removing a multi-selection leaves the cursor after it; repeat-all wraps manual steps", () => {
  const queue = fixture();
  const [, b, c, d] = queue.rows.map((row) => row.rowId);
  queue.select(b);
  queue.extend(c);
  queue.removeSelected();
  assert.deepEqual(names(queue), ["a", "d"]);
  assert.equal(queue.selected, d);
  assert.equal(queue.neighbor(1), undefined);
  queue.repeat = "all";
  assert.equal(queue.neighbor(1)?.source.slice(-1), "a");
  assert.equal(queue.neighbor(-1)?.source.slice(-1), "a");
});

test("shuffle picks each row once per round; repeat-all starts a new round", () => {
  const queue = fixture();
  queue.setShuffle(true);
  queue.select(queue.rows[0].rowId);
  queue.played(queue.rows[0].rowId);
  const seen = new Set([queue.rows[0].rowId]);
  let anchor = queue.rows[0].rowId;
  for (let i = 0; i < 3; i++) {
    const next = queue.candidates(anchor);
    assert.ok(next.every((row) => !seen.has(row.rowId)));
    anchor = next[0].rowId;
    seen.add(anchor);
    queue.played(anchor);
  }
  assert.equal(queue.candidates(anchor).length, 0, "the round is over");
  queue.repeat = "all";
  assert.equal(queue.candidates(anchor).length, 3);
});
