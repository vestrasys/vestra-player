// 独立实验验收：处理中引用的字幕资源不能改变已获得的不可变快照。
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../support/host.ts";

test("E2/I2: managed snapshot assets survive source edits/deletion, owner unload and handle release", async (t) => {
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
  // This was accepted by probe-0.2 and produced different pixels after edits.
  // probe-0.3 must reject the undeclared resource rather than imply immutability.
  await assert.rejects(
    host.request("effects.set", {
      key: "unmanaged",
      kind: "subtitle",
      filter: `subtitles=filename='${captions}':force_style='Fontsize=24'`,
      sessionId: (await host.inspect()).playback.sessionId,
    }),
    /file|managed|resource/i,
  );
  await writeFile(join(directory, "package.json"), '{"type":"module"}');
  await writeFile(
    join(directory, "manifest.json"),
    JSON.stringify({
      id: "acceptance-captions",
      name: "Independent snapshot source",
      experimental: true,
      entry: "index.ts",
      ui: {
        file: "ui.slint",
        component: "CaptionPanel",
        global: "CaptionState",
      },
    }),
  );
  await writeFile(
    join(directory, "ui.slint"),
    "// 实验 fixture，最终 API 可能差异很大。\nexport global CaptionState { in-out property<string> status; callback action(string,float); }\nexport component CaptionPanel inherits Rectangle { Text { text: CaptionState.status; } }",
  );
  const sdk = fileURLToPath(new URL("../../sdk/types.ts", import.meta.url));
  await writeFile(
    join(directory, "index.ts"),
    `// 实验 fixture；最终 API 可能差异很大。\nimport type {PluginContext} from ${JSON.stringify(sdk)};\nexport async function activate(ctx:PluginContext){
    const sessionId=(await ctx.player.snapshot()).sessionId;
    await ctx.effects.set({key:"captions",kind:"subtitle",sessionId,files:{captions:${JSON.stringify(captions)}},filter:"subtitles=filename='{{file:captions}}':force_style='Fontsize=24'"});
    await ctx.ui.patch({status:"ready"});
    return async()=>{await ctx.effects.remove("captions");};
  }`,
  );
  await host.load(join(directory, "manifest.json"));
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (plugin) => plugin.properties.status === "ready",
    ),
  );
  const snapshot = await host.request<{ id: string }>("effects.snapshot");
  async function render(name: string, release = false) {
    const output = join(directory, `${name}.png`);
    const job = await host.request<{ id: string }>("media.start", {
      input: media,
      output,
      format: "image",
      start: 0.3,
      processingSnapshot: snapshot.id,
    });
    if (release)
      await host.request("effects.releaseSnapshot", { id: snapshot.id });
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
  await rm(captions);
  assert(
    (await render("deleted")).equals(original),
    "deleting the source file cannot invalidate the snapshot",
  );
  await host.unload("acceptance-captions");
  assert(
    (await render("unloaded")).equals(original),
    "unloading the asset provider cannot invalidate another owner's snapshot",
  );
  assert(
    (await render("released", true)).equals(original),
    "a started job must retain its assets after the snapshot handle is released",
  );
  const final = await host.request<{
    resources: { managedAssets: number; snapshots: number };
  }>("host.inspect");
  assert.equal(
    final.resources.managedAssets,
    0,
    "the last completed job releases its managed copy",
  );
  assert.equal(final.resources.snapshots, 0);
});
