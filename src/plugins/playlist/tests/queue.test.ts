import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Media, PlayerState } from "../../../sdk/plugin/index.ts";
import { Queue } from "../queue.ts";

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
    await queue.addFiles([path, path, alias, join(directory, "missing.mp4")]);
    assert.equal(queue.rows.length, 2);
    assert.equal(queue.selected, queue.rows[0].rowId);
    assert.equal(queue.binding, null);
    assert.match(queue.rows[1].error, /文件不存在/);
    assert.equal(queue.rows[1].status, "待重试");
    assert.match(queue.message, /新增 2 项，跳过 2 个重复，1 项待重试/);
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
  assert.equal(queue.message, "已选择已有链接");
  assert.throws(() => queue.addUrl("file:///tmp/a"), /只接受不含账户密码的 HTTP\(S\) 链接/);
  assert.throws(() => queue.addUrl("https://user:password@example.org/a"), /只接受/);
  assert.throws(() => queue.addUrl("not a url"), /请输入完整 HTTP\(S\) 链接/);
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

test("bound row follows ready, paused, loading, ended, error and closed states", () => {
  const queue = fixture(),
    row = queue.rows[0];
  queue.bind({ rowId: row.rowId, media: media() });
  queue.observe(state({ phase: "loading" }));
  assert.equal(row.status, "正在打开");
  queue.observe(state({ paused: true }));
  assert.equal(row.status, "已暂停");
  assert.equal(queue.message, "已暂停");
  queue.observe(state());
  assert.equal(row.status, "播放中");
  queue.observe(state({ phase: "ended", endReason: "natural" }));
  assert.equal(row.status, "已结束");
  assert.equal(queue.binding?.rowId, row.rowId, "the natural end itself is handled by media.onEnd");
  queue.observe(state({ phase: "error", error: "broken" }));
  assert.equal(row.status, "失败");
  assert.equal(row.error, "broken");
  assert.equal(queue.failed, row.rowId);
  assert.equal(queue.binding, null);

  queue.bind({ rowId: row.rowId, media: media() });
  queue.observe(state({ phase: "empty" }));
  assert.equal(queue.binding, null);
  assert.equal(queue.message, "当前来源已关闭");
});

test("another source, even of the same path, detaches the row and releases its end listener", () => {
  const queue = fixture();
  let released = false;
  const replaced = media();
  queue.bind({ rowId: queue.rows[0].rowId, media: replaced, end: { dispose: async () => void (released = true) } });
  replaced.current = false;
  queue.observe(state({ sourceId: 5, source: "/a" }));
  assert.equal(queue.binding, null);
  assert.equal(queue.rows[0].status, "已切换");
  assert.equal(queue.message, "已切换到列表外来源");
  assert.equal(released, true);
});

test("removing current and clearing revoke progression without requesting public media changes", () => {
  const queue = fixture();
  queue.select(queue.rows[1].rowId);
  queue.bind({ rowId: queue.selected!, media: media() });
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
