// 实验字幕契约与真实宿主测试；不替代最终产品验收。
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtemp,
  writeFile,
  readFile,
  rm,
  mkdir,
  copyFile,
} from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { TestHost } from "../../support/host.ts";
import {
  parseSrt,
  serializeSrt,
  subtitleFilter,
} from "../../../plugins/subtitle-workbench/subtitles.ts";
const subtitles =
  "1\n00:00:00,000 --> 00:00:00,800\nFIRST CAPTION\n\n2\n00:00:01,000 --> 00:00:02,000\nSECOND CAPTION\n";
const plugin = fileURLToPath(
  new URL("../../../plugins/subtitle-workbench/manifest.json", import.meta.url),
);
test("SRT validates actual timeline, synchronizes and escapes source boundary", () => {
  const cues = parseSrt(subtitles);
  assert.equal(cues.length, 2);
  assert.equal(cues[1].start, 1);
  assert.match(serializeSrt(cues, 0.5), /00:00:01,500 --> 00:00:02,500/);
  assert.throws(() => parseSrt("1\n00:61:00,000 --> 00:62:00,000\nbad"));
  assert.throws(() => parseSrt("1\n00:00:02,000 --> 00:00:01,000\nbad"));
  assert.throws(() => subtitleFilter("/tmp/a';bad", 24, false));
  assert.throws(() => subtitleFilter("captions", Infinity, false));
});
test(
  "S1/S2 real worker + Rust/FFmpeg: render, tracks, style, delay, stale session, retry, cancel, cleanup",
  { timeout: 60000 },
  async () => {
    const directory = await mkdtemp(join(tmpdir(), "vestra-subtitle-test-"));
    const host = new TestHost();
    try {
      const media = join(directory, "sample.mp4");
      const secondMedia = join(directory, "other.mp4");
      const srt = join(directory, "source's [captions].srt");
      await writeFile(srt, subtitles);
      execFileSync("ffmpeg", [
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x180:r=10:d=3",
        "-c:v",
        "libx264",
        "-y",
        media,
      ]);
      execFileSync("ffmpeg", [
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:s=320x180:r=10:d=3",
        "-c:v",
        "libx264",
        "-y",
        secondMedia,
      ]);
      await host.request("player.load", { path: media });
      await host.load(plugin);
      async function state() {
        return (
          (await host.inspect()).plugins.find(
            (entry) => entry.id === "subtitle-workbench",
          )?.properties ?? {}
        );
      }
      async function action(name: string, value = 0) {
        await host.action("subtitle-workbench", name, value);
      }
      async function wait(
        check: (properties: Record<string, unknown>) => boolean,
      ) {
        await host.until(async () => check(await state()));
      }
      await wait((p) => p.status === "字幕工作台就绪");
      await host.request("host.dialogs", { paths: [srt] });
      await action("open");
      await wait((p) => p["cue-count"] === 2);
      const effects = await host.request<{ filter: string }[]>("effects.list");
      assert.match(effects[0].filter, /subtitles=/);
      async function image(
        name: string,
        processing?: "current" | { snapshot: string },
        time = 0.3,
      ): Promise<Buffer> {
        const output = join(directory, name + ".png");
        const job = await host.request<{ id: string }>("media.start", {
          input: media,
          output,
          format: "image",
          start: time,
          ...(processing === "current"
            ? { processing }
            : processing
              ? { processingSnapshot: processing.snapshot }
              : {}),
        });
        await host.until(async () => {
          const s = await host.request<{ state: string; error?: string }>(
            "media.status",
            { id: job.id },
          );
          if (s.state === "failed") throw new Error(s.error);
          return s.state === "succeeded";
        });
        assert((await readFile(output)).length > 0);
        return execFileSync("ffmpeg", [
          "-v",
          "error",
          "-i",
          output,
          "-pix_fmt",
          "rgb24",
          "-f",
          "rawvideo",
          "-",
        ]);
      }
      const raw = await image("raw");
      const caption = await image("caption", "current");
      assert(raw.every((value) => value < 10));
      assert(caption.some((value) => value > 150));
      const frozen = await host.request<{ id: string }>("effects.snapshot", {
        includeSubtitles: true,
      });
      const secondCue = await image("second-cue", "current", 1.3);
      assert(secondCue.some((value) => value > 150));
      assert.notDeepEqual(
        secondCue,
        caption,
        "seek must use absolute subtitle timeline",
      );
      await action("font-size", 40);
      await wait((p) => p["font-size"] === 40);
      const large = await image("large", "current");
      assert.notDeepEqual(large, caption);
      assert.deepEqual(
        await image("frozen-after-update", { snapshot: frozen.id }),
        caption,
        "registered subtitle assets must stay immutable when plugin rewrites its SRT cache",
      );
      await action("yellow");
      await wait((p) => p.yellow === true);
      const colored = await image("yellow", "current");
      assert.notDeepEqual(colored, large);
      await action("delay", 1);
      await wait((p) => p.delay === 1);
      assert.deepEqual(await image("delayed", "current"), raw);
      await action("toggle");
      await wait((p) => p.visible === false);
      assert.equal((await host.request<unknown[]>("effects.list")).length, 0);
      await action("toggle");
      await wait((p) => p.visible === true);
      await action("delay", 0);
      await wait((p) => p.delay === 0);
      await action("translate");
      await wait((p) => p.translating === true);
      await host.request("player.load", { path: secondMedia });
      await wait((p) => p["cue-count"] === 0 && p.translating === false);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 450));
      assert.equal((await state()).translated, false);
      assert.equal((await host.request<unknown[]>("effects.list")).length, 0);
      await host.request("host.dialogs", { paths: [srt] });
      await action("open");
      await wait((p) => p["cue-count"] === 2);
      await action("mock-fail-next");
      await action("translate");
      await wait(
        (p) => typeof p.status === "string" && p.status.includes("翻译失败"),
      );
      await action("translate");
      await wait((p) => p.translated === true && p.translating === false);
      await action("original");
      await wait((p) => p.translated === false);
      await action("translate");
      await wait((p) => p.translating === true);
      await action("cancel");
      await wait((p) => p.translating === false);
      assert.equal((await state()).translated, false);
      await action("next-cue");
      await wait((p) => p["cue-index"] === 2);
      const evidence = fileURLToPath(
        new URL(
          "../../../../.cache/probe-evidence/subtitle-workbench/",
          import.meta.url,
        ),
      );
      await mkdir(evidence, { recursive: true });
      for (const name of [
        "raw",
        "caption",
        "second-cue",
        "large",
        "yellow",
        "delayed",
      ]) {
        await copyFile(
          join(directory, `${name}.png`),
          join(evidence, `${name}.png`),
        );
      }
      await writeFile(
        join(evidence, "evidence.json"),
        JSON.stringify(
          {
            cues: 2,
            sourceSrt: subtitles,
            originalNonBlackBytes: raw.filter((value) => value > 10).length,
            captionBrightBytes: caption.filter((value) => value > 150).length,
            largeBrightBytes: large.filter((value) => value > 150).length,
            sessionIsolation: true,
            translationRetry: true,
            cancellation: true,
          },
          null,
          2,
        ),
      );
      await host.unload("subtitle-workbench");
      assert.deepEqual(
        await image("frozen-after-unload", { snapshot: frozen.id }),
        caption,
        "snapshot owned by another consumer survives subtitle plugin disposal and cache deletion",
      );
      await host.request("effects.releaseSnapshot", { id: frozen.id });
      for (const name of ["frozen-after-update", "frozen-after-unload"]) {
        await copyFile(
          join(directory, `${name}.png`),
          join(evidence, `${name}.png`),
        );
      }
      await writeFile(
        join(evidence, "snapshot-evidence.json"),
        JSON.stringify(
          {
            baseline: "probe-0.3",
            afterCacheRewriteMatchesOriginal: true,
            afterProducerUnloadMatchesOriginal: true,
          },
          null,
          2,
        ),
      );
      assert.equal((await host.request<unknown[]>("effects.list")).length, 0);
    } finally {
      await host.close();
      await rm(directory, { recursive: true, force: true });
    }
  },
);

test("S2 replaceable async provider honors cancellation and fails once before retry", async () => {
  const { MockTranslationProvider } =
    await import("../../../plugins/subtitle-workbench/translation.ts");
  const provider = new MockTranslationProvider();
  const controller = new AbortController();
  const pending = provider.translate(parseSrt(subtitles), controller.signal);
  controller.abort();
  await assert.rejects(pending, /abort/i);
  provider.failNext = true;
  await assert.rejects(
    provider.translate(parseSrt(subtitles), new AbortController().signal),
    /暂时不可用/,
  );
  const translated = await provider.translate(
    parseSrt(subtitles),
    new AbortController().signal,
  );
  assert.equal(translated[0].text, "[译] FIRST CAPTION");
  assert.equal(translated[1].start, 1);
});
