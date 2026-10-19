// 独立实验验收：处理中引用的字幕资源不能改变已获得的不可变快照。
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TestHost } from "../support/host.ts";

test("E2/I2: a processing snapshot retains subtitle content when its provider changes the file", async (t) => {
  const directory = await mkdtemp(
    join(tmpdir(), "vestra-acceptance-snapshot-"),
  );
  t.after(() => rm(directory, { recursive: true, force: true }));
  const media = join(directory, "source.mp4");
  const captions = join(directory, "active.srt");
  execFileSync("ffmpeg", [
    "-v",
    "error",
    "-f",
    "lavfi",
    "-i",
    "color=c=black:s=320x180:r=10:d=3",
    "-c:v",
    "libx264",
    media,
  ]);
  await writeFile(
    captions,
    "1\n00:00:00,000 --> 00:00:02,000\nORIGINAL CAPTION\n",
  );
  const host = new TestHost();
  t.after(() => host.close());
  await host.request("player.load", { path: media });
  await host.request("effects.set", {
    key: "captions",
    kind: "subtitle",
    filter: `subtitles=filename='${captions}':force_style='Fontsize=24'`,
    sessionId: (await host.inspect()).playback.sessionId,
  });
  const snapshot = await host.request<{ id: string }>("effects.snapshot");
  async function render(name: string) {
    const output = join(directory, `${name}.png`);
    const job = await host.request<{ id: string }>("media.start", {
      input: media,
      output,
      format: "image",
      start: 0.3,
      processingSnapshot: snapshot.id,
    });
    await host.until(async () => {
      const state = await host.request<{ state: string; error?: string }>(
        "media.status",
        { id: job.id },
      );
      if (state.state === "failed") throw new Error(state.error);
      return state.state === "succeeded";
    });
    return execFileSync("ffmpeg", [
      "-v",
      "error",
      "-i",
      output,
      "-f",
      "rawvideo",
      "-pix_fmt",
      "rgb24",
      "pipe:1",
    ]);
  }
  const original = await render("before");
  assert(
    original.some((byte) => byte > 150),
    "the original subtitle must really render",
  );
  await writeFile(captions, "1\n00:00:00,000 --> 00:00:02,000\nREPLACED\n");
  const later = await render("after");
  assert.equal(
    later.equals(original),
    true,
    "a later task in the same snapshot group must retain the original subtitle",
  );
  await host.request("effects.releaseSnapshot", { id: snapshot.id });
});
