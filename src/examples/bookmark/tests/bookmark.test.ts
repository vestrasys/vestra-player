import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

test("mark the position, then jump back to it", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 600, position: 42 });
  await startPlugin(activate, {
    directory: new URL("..", import.meta.url),
    host,
  });

  await host.command("bookmark.open").run();
  const window = host.window("bookmark");
  window.emit("mark");
  await host.settle();
  assert.equal(window.props.label, "书签：42.0 秒");

  host.setSource({ position: 300 }); // 播放继续向前
  window.emit("jump");
  await host.settle();
  assert.equal(host.source.position, 42);
});
