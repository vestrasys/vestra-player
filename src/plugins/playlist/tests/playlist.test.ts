import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "../../../sdk/plugin/index.ts";
import { startPlugin } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

type ViewRow = { rowId: string; label: string; status: string; selected: boolean; current: boolean };
type Intent = { kind: string; source?: string; startPaused?: boolean; expected?: Json };

async function start() {
  const directory = await mkdtemp(join(tmpdir(), "vestra-playlist-"));
  const a = join(directory, "a.mp4"),
    b = join(directory, "b.mp4"),
    alias = join(directory, "alias.mp4");
  await writeFile(a, "plugin test, no media decode claim");
  await writeFile(b, "plugin test, no media decode claim");
  await symlink(a, alias);
  const { host, stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url) });
  const panel = host.mounted();
  const app = {
    host,
    a,
    b,
    alias,
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    prop: (name: string) => panel.props[name],
    command: (name: string) => host.command(`playlist.${name}`),
    /** Intents of every source submission, oldest first. */
    opens: () => host.sent("source.submit").map((call) => call.intent as unknown as Intent),
    emit: async (callback: string, ...args: Json[]) => {
      panel.emit(callback, ...args);
      await host.settle();
    },
    action: (name: string) => app.emit("action", name),
    /** Fire a sidebar action while an open is held back (settling would wait for it). */
    actionNow: async (name: string) => {
      panel.emit("action", name);
      await new Promise((resolve) => setTimeout(resolve, 10));
    },
    addFiles: async (paths: string[]) => {
      host.dialogAnswers.push(paths);
      await app.action("add");
    },
    addUrl: async (url: string) => {
      await app.command("link").run();
      host.window("playlist-link").emit("add-link", url);
      await host.settle();
    },
    /** The current source plays to its natural end. */
    end: () => host.setSource({ phase: "ended", terminalReason: "natural" }),
    /** The user replays the current source (same source, new playthrough). */
    restart: () => {
      const { stamp, cycle } = host.source;
      host.setSource({ stamp: { ...stamp, intentVersion: stamp.intentVersion + 1 }, cycle: cycle + 1, phase: "ready" });
    },
    async close() {
      await stop();
      await rm(directory, { recursive: true, force: true });
    },
  };
  return app;
}

test("menus, sidebar and initial state are declared like before", async () => {
  const app = await start();
  try {
    const paths = [...app.host.commands.values()].map((command) => command.declaration.path.join("/"));
    assert.deepEqual(paths, [
      "文件/播放列表/添加文件…",
      "文件/播放列表/添加链接…",
      "文件/播放列表/播放选中项",
      "文件/播放列表/上一项",
      "文件/播放列表/下一项",
      "文件/播放列表/移除选中项",
      "文件/播放列表/上移",
      "文件/播放列表/下移",
      "文件/播放列表/清空列表",
      "视图/显示播放列表",
    ]);
    assert.equal(app.host.mounted().slot, "sidebar");
    assert.equal(app.host.mounted().component, "PlaylistSidebar");
    assert.equal(app.prop("message"), "添加文件或链接；选择后点击播放");
    assert.equal(app.prop("details"), "尚未选择项目");
    assert.deepEqual(app.command("play").state, {
      enabled: false,
      checked: null,
      busy: false,
      reason: "当前没有可操作项",
    });
    assert.equal(app.command("visibility").state.checked, true);
    // The close button and the menu change the user's preference.
    await app.emit("hide");
    assert.equal(app.host.layout.sidebar?.preferred, false);
    assert.equal(app.command("visibility").state.checked, false);
    await app.command("visibility").run();
    assert.equal(app.host.layout.sidebar?.preferred, true);
    assert.equal(app.command("visibility").state.checked, true);
    assert.equal(app.host.sent("ui.mount")[0]!.title, "播放列表");
    const orders = [...app.host.commands.values()].map(({ declaration: d }) => ("order" in d ? d.order : null));
    assert.deepEqual(orders, [208, 209, 207, 203, 205, 211, 204, 206, 210, 501]);
    await app.emit("focus-changed", true);
    assert.deepEqual(app.host.mounted().locks, { focused: true, dragging: false, popupOpen: false });
  } finally {
    await app.close();
  }
});

test("adding files dedups by realpath, never plays, and plays only on request with startPaused false", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.alias, app.b]);
    assert.equal(app.rows().length, 2);
    assert.equal(app.prop("message"), "新增 2 项，跳过 1 个重复，0 项待重试");
    assert.equal(app.opens().length, 0);
    assert.equal(app.rows()[0].selected, true);
    app.host.dialogAnswers.push(null); // cancelled: nothing changes
    await app.action("add");
    assert.equal(app.prop("message"), "新增 2 项，跳过 1 个重复，0 项待重试");
    await app.action("play");
    assert.deepEqual(app.opens()[0], { kind: "open", source: app.a, startPaused: false });
    assert.equal(app.rows()[0].status, "正在打开");
    assert.equal(app.command("play").state.busy, true, "open stays unsettled while loading");
    assert.equal(app.command("next").state.reason, "正在打开或结果未知");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[0].status, "播放中");
    assert.equal(app.rows()[0].current, true);
    assert.equal(app.command("next").state.enabled, true);
    app.host.setSource({ paused: true });
    await app.host.settle();
    assert.equal(app.rows()[0].status, "已暂停");
  } finally {
    await app.close();
  }
});

test("the playlist provides next/previous to other plugins with the same rules as its menu", async () => {
  const app = await start();
  try {
    assert.deepEqual(app.host.sequence, { canNext: false, canPrevious: false });
    await app.addFiles([app.a, app.b]);
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: false });
    assert.equal(app.host.source.sequence?.next, true, "the host merges the flags into the source state");
    await app.host.step("next");
    await app.host.settle();
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: app.b, startPaused: false });
    await assert.rejects(app.host.step("next"), { code: "unavailable" });
  } finally {
    await app.close();
  }
});

test("URL rows: adding only selects; the original URL goes to public routing; one open at a time", async () => {
  const app = await start();
  try {
    const a = "https://example.org/watch?v=one",
      b = "https://example.org/watch?v=two";
    await app.addUrl(a);
    assert.equal(app.host.window("playlist-link").props.message, "链接已添加，仅选中；显式播放需可用的来源提供者");
    assert.equal(app.host.window("playlist-link").visible, false);
    await app.addUrl(b);
    await app.addUrl("ftp://example.org/x");
    assert.equal(app.host.window("playlist-link").props.message, "只接受不含账户密码的 HTTP(S) 链接");
    assert.equal(app.rows().length, 2);
    assert.equal(app.rows()[1].selected, true);
    assert.equal(app.opens().length, 0);
    await app.emit("select-row", app.rows()[0].rowId);
    assert.equal(app.prop("details"), a);

    // A resolver takes a while: the row shows it is opening and play is refused meanwhile.
    let accept!: () => void;
    app.host.handle("source.submit", () => new Promise<void>((resolve) => (accept = resolve)).then(() => {
      app.host.openElsewhere(a, "loading"); // stands in for the accepted URL open
      const { stamp, cycle } = app.host.source;
      return { status: "completed", ticket: { stamp, cycle, owner: 1 } };
    }));
    await app.actionNow("play");
    await app.actionNow("play");
    app.host.mounted().emit("play-row", app.rows()[0].rowId);
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(app.host.sent("source.submit").length, 1);
    assert.deepEqual(app.opens()[0], { kind: "open", source: a, startPaused: false });
    assert.equal(app.rows()[0].status, "正在打开");
    assert.equal(app.prop("can-play"), false);
    accept();
    await app.host.settle();
    app.host.reset("source.submit");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[0].status, "播放中");
    assert.equal(app.rows()[0].current, true);
  } finally {
    await app.close();
  }
});

test("clock ticks do not resend unchanged list models or command states", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.action("play");
    app.host.setSource({ phase: "ready", duration: 1000 });
    await app.host.settle();
    const sends = () => app.host.sent("ui.setMount").length + app.host.sent("commands.update").length;
    const before = sends();
    for (let position = 0; position < 1000; position++) app.host.setSource({ position });
    await app.host.settle();
    assert.equal(sends(), before);
  } finally {
    await app.close();
  }
});

test("natural end advances once in current order, conditional on the ended state, keeping the selection", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/c");
    await app.addFiles([app.a, app.b]);
    // Rows are c, a, b; moving c down gives a, c, b.
    await app.emit("select-row", app.rows()[0].rowId);
    await app.action("move-down");
    assert.deepEqual(app.rows().map((row) => row.label.replace(/^\d+\. /, "")), ["a.mp4", "https://example.org/c", "b.mp4"]);
    const c = app.rows()[1].rowId;
    await app.emit("select-row", app.rows()[0].rowId);
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.emit("select-row", app.rows()[2].rowId); // the user is looking at b
    const ended = { ...app.host.source.stamp };
    app.end();
    app.end(); // a repeated end event of the same playthrough
    await app.host.settle();
    assert.equal(app.opens().length, 2);
    assert.deepEqual(app.opens()[1], { kind: "open", source: "https://example.org/c", startPaused: false, expected: ended });
    assert.equal(app.rows()[0].status, "已切换");
    assert.equal(app.rows()[1].current, true);
    assert.equal(app.rows().find((row) => row.rowId === c)?.selected, false);
    assert.equal(app.rows()[2].selected, true, "auto-next keeps selection");
  } finally {
    await app.close();
  }
});

test("a replay renews end eligibility; the last row reports that the list is finished", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a]);
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.end();
    await app.host.settle();
    assert.equal(app.prop("message"), "列表已播完");
    assert.equal(app.rows()[0].status, "已结束");
    app.restart();
    await app.host.settle();
    assert.equal(app.prop("message"), "播放中");
    assert.equal(app.rows()[0].current, true, "a replay keeps the row bound");
    app.end();
    await app.host.settle();
    assert.equal(app.prop("message"), "列表已播完");
    assert.equal(app.opens().length, 1);
  } finally {
    await app.close();
  }
});

test("a replay that wins the race against auto-advance keeps the newer source", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.end();
    app.restart(); // while the next file's path is being checked
    await app.host.settle();
    assert.equal(app.opens().length, 2, "the conditional open was submitted");
    assert.equal(app.prop("message"), "自动推进已过期，保留较新的来源");
    assert.equal(app.host.source.source, app.a);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 2, "the list no longer follows the replayed source");
    // Explicit play still works afterwards.
    assert.equal(app.command("play").state.enabled, true);
  } finally {
    await app.close();
  }
});

test("errors, stops and other sources never advance; another open of the same path is not taken over", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.action("play");
    app.host.setSource({ phase: "error", terminalReason: { failed: "broken" } });
    await app.host.settle();
    assert.equal(app.rows()[0].status, "失败");
    assert.equal(app.prop("details"), `${app.a}\nbroken`);
    assert.equal(app.opens().length, 1);

    await app.action("play");
    app.host.setSource({ phase: "ended", terminalReason: "stopped" });
    await app.host.settle();
    assert.equal(app.opens().length, 2);

    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.host.openElsewhere(app.a);
    await app.host.settle();
    assert.equal(app.prop("message"), "已切换到列表外来源");
    assert.equal(app.rows()[0].status, "已切换");
    assert.equal(app.rows()[0].current, false);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 3);
  } finally {
    await app.close();
  }
});

test("a failed open stays on its row, keeps the navigation anchor and can be retried", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/a");
    await app.addUrl("https://example.org/b");
    await app.addUrl("https://example.org/c");
    const [a, b, c] = app.rows().map((row) => row.rowId);
    await app.emit("select-row", b);
    app.host.handle("source.submit", () => ({ status: "failed", error: { backend: "resolverUnavailable" } }));
    await app.action("play");
    assert.equal(app.rows()[1].status, "失败");
    assert.match(String(app.prop("details")), /resolverUnavailable/);
    assert.equal(app.prop("message"), "打开失败：resolverUnavailable");
    await app.emit("select-row", a);
    app.host.reset("source.submit");
    await app.action("next"); // from the failed row b, not from the selection a
    assert.equal(app.opens()[1].source, "https://example.org/c");
    assert.equal(app.rows().find((row) => row.rowId === c)?.status, "正在打开");
    app.host.setSource({ phase: "ready" });
    await app.emit("select-row", b);
    await app.command("play").run(); // retry
    assert.equal(app.opens()[2].source, "https://example.org/b");
    assert.equal(app.rows().length, 3);
    assert.equal(app.rows()[1].rowId, b);
  } finally {
    await app.close();
  }
});

test("a late accepted reply cannot reclaim a newer external source or resurrect a removed row", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    // The host accepts our open at once, but the reply arrives after a newer source.
    const deferOpen = () => {
      let accept!: () => void;
      app.host.handle("source.submit", () => {
        app.host.openElsewhere(app.a, "loading");
        const { stamp, cycle } = app.host.source;
        return new Promise<void>((resolve) => (accept = resolve)).then(() => ({
          status: "completed",
          ticket: { stamp, cycle, owner: 1 },
        }));
      });
      return () => accept();
    };
    let accept = deferOpen();
    await app.actionNow("play");
    app.host.openElsewhere(app.a); // same path, opened elsewhere
    accept();
    await app.host.settle();
    assert.equal(app.prop("message"), "已切换到列表外来源");
    assert.equal(app.rows().some((row) => row.current), false);
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 1);

    accept = deferOpen();
    await app.actionNow("play");
    app.host.openElsewhere(app.b);
    await app.actionNow("remove");
    assert.equal(app.prop("message"), "正在打开的媒体已移出列表");
    accept();
    await app.host.settle();
    app.host.reset("source.submit");
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows().some((row) => row.current), false);
    assert.equal(app.prop("message"), "当前媒体已移出列表");
    app.end();
    await app.host.settle();
    assert.equal(app.opens().length, 2);
  } finally {
    await app.close();
  }
});

test("a lost open reply is resolved by the same operation, never resubmitted", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/a");
    app.host.handle("source.submit", () => {
      app.host.openElsewhere("https://example.org/a", "loading"); // the host did accept it
      throw new Error("reply lost");
    });
    const { stamp, cycle } = { stamp: { generation: 1, intentVersion: 1 }, cycle: 1 };
    app.host.handle("source.operation", () => ({ status: "completed", ticket: { stamp, cycle, owner: 1 } }));
    await app.action("play");
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[0].status, "播放中");
    assert.equal(app.host.sent("source.submit").length, 1);
    assert.deepEqual(
      app.host.sent("source.operation").map((call) => call.operationId),
      [app.host.sent("source.submit")[0].operationId],
    );
  } finally {
    await app.close();
  }
});

test("removing the current row and clearing keep the public media playing", async () => {
  const app = await start();
  try {
    await app.addFiles([app.a, app.b]);
    await app.action("play");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.action("remove");
    assert.equal(app.prop("message"), "当前媒体已移出列表");
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows()[0].selected, true);
    await app.action("clear");
    assert.equal(app.prop("message"), "列表已清空，当前媒体继续");
    assert.equal(app.rows().length, 0);
    assert.equal(app.prop("can-clear"), false);
    assert.equal(app.host.source.phase, "ready");
    assert.equal(app.opens().filter((intent) => intent.kind !== "open").length, 0);
  } finally {
    await app.close();
  }
});

test("an accepted URL open replaced by another source while resolving is not a row failure", async () => {
  const app = await start();
  try {
    await app.addUrl("https://example.org/a");
    await app.addUrl("https://example.org/b");
    const [a, b] = app.rows().map((row) => row.rowId);
    await app.emit("select-row", a);
    // Accepted (pending while the resolver runs); then another source takes over.
    let outcome: Json = null;
    app.host.handle("source.submit", async (params) => {
      app.host.reset("source.submit");
      const done = (await app.host.request("source.submit", params)) as { ticket: Json };
      app.host.handle("source.operation", () => outcome ?? { status: "pending", ticket: done.ticket });
      return { status: "pending", ticket: done.ticket };
    });
    await app.actionNow("play");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal(app.rows()[0].status, "正在打开");
    app.host.openElsewhere("/elsewhere.mp4");
    outcome = { status: "failed", error: { backend: "source resolution cancelled, expired or superseded" } };
    await app.host.settle(600);
    app.host.reset("source.operation");
    assert.equal(app.prop("message"), "已切换到列表外来源");
    assert.notEqual(app.rows()[0].status, "失败");
    assert.equal(app.prop("can-play"), true); // the open is released
    // No failed anchor on row a: "next" navigates from the selection, not from a.
    await app.emit("select-row", b);
    assert.equal(app.prop("can-next"), false);
  } finally {
    await app.close();
  }
});
