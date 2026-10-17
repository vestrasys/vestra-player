// 实验集成测试：真实 Rust、TS worker 与 FFmpeg，图像/媒体结果独立解码验证。
import test from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../../support/host.ts";

const exec = promisify(execFile);
const plugin = "capture-export";

async function pixels(path: string): Promise<Buffer> {
  const bytes = await readFile(path);
  assert.equal(bytes.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  return (
    await exec(
      "ffmpeg",
      [
        "-v",
        "error",
        "-i",
        path,
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
      ],
      { encoding: "buffer", maxBuffer: 32_000_000 },
    )
  ).stdout;
}

function mean(bytes: Buffer): number {
  return bytes.reduce((sum, value) => sum + value, 0) / bytes.length;
}

function difference(a: Buffer, b: Buffer): number {
  assert.equal(a.length, b.length);
  return (
    a.reduce((sum, value, index) => sum + Math.abs(value - b[index]), 0) /
    a.length
  );
}

test(
  "E1/E2：真实截图阶段、字幕开关、连续截图、片段快照与取消清理",
  { timeout: 120_000 },
  async (t) => {
    const temporary = !process.env.VESTRA_CAPTURE_EVIDENCE;
    const evidenceRoot = process.env.VESTRA_CAPTURE_EVIDENCE
      ? resolve(process.env.VESTRA_CAPTURE_EVIDENCE)
      : tmpdir();
    await mkdir(evidenceRoot, { recursive: true });
    const directory = await mkdtemp(join(evidenceRoot, "vestra-capture-"));
    if (temporary)
      t.after(() => rm(directory, { recursive: true, force: true }));
    const input = join(directory, "black.mp4");
    const alternate = join(directory, "red.mp4");
    for (const [path, color] of [
      [input, "black"],
      [alternate, "red"],
    ])
      await exec("ffmpeg", [
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        `color=c=${color}:s=320x180:r=30:d=30`,
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=30",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        path,
      ]);
    const subtitles = join(directory, "subtitle.srt");
    await writeFile(
      subtitles,
      "1\n00:00:00,000 --> 00:00:29,000\nSCREENSHOT SUBTITLE\n",
    );
    const host = new TestHost();
    t.after(() => host.close());
    await host.load(
      fileURLToPath(
        new URL(
          "../../../plugins/capture-export/manifest.json",
          import.meta.url,
        ),
      ),
    );
    const props = async () =>
      (await host.inspect()).plugins.find((entry) => entry.id === plugin)
        ?.properties;
    await host.until(async () => (await props())?.status === "截图与导出就绪");

    async function action(
      name: string,
      value: number,
      property: string,
      expected: number | boolean,
    ) {
      await host.action(plugin, name, value);
      await host.until(async () => (await props())?.[property] === expected);
    }

    async function start(name: string, filename: string) {
      const output = join(directory, filename);
      await rm(output, { force: true });
      const previous = (await props())?.["operation-id"];
      await host.request("host.dialogs", { paths: [output] });
      await host.action(plugin, name);
      await host.until(
        async () => (await props())?.["operation-id"] !== previous,
      );
      return output;
    }

    async function finish() {
      await host.until(async () => (await props())?.busy === false, 30_000);
      const properties = await props();
      assert.match(String(properties?.status), /已完成/);
      assert.equal(properties?.progress, 1);
    }

    await start("capture-raw", "no-media.png");
    await host.until(async () => (await props())?.busy === false);
    assert.match(String((await props())?.status), /文件媒体/);
    await host.request("player.load", { path: input });
    await host.request("player.togglePause");
    await host.request("player.seek", { seconds: 1 });
    const raw = await start("capture-raw", "raw.png");
    await finish();
    const rawPixels = await pixels(raw);
    assert.ok(mean(rawPixels) < 2, "原始黑色视频不应包含处理效果");
    const beforeOverwrite = await readFile(raw);
    const previousOperation = (await props())?.["operation-id"];
    await host.request("host.dialogs", { paths: [raw] });
    await host.action(plugin, "capture-raw");
    await host.until(
      async () =>
        (await props())?.["operation-id"] !== previousOperation &&
        (await props())?.busy === false,
    );
    assert.match(String((await props())?.status), /失败/);
    assert.deepEqual(await readFile(raw), beforeOverwrite, "已有输出不得覆盖");

    await host.request("effects.set", {
      key: "test-grade",
      kind: "video",
      filter: "eq=brightness=0.25",
    });
    const graded = await start("capture-processed", "graded.png");
    await finish();
    const gradedPixels = await pixels(graded);
    assert.ok(
      mean(gradedPixels) - mean(rawPixels) > 40,
      "处理后 PNG 应包含真实亮度处理",
    );
    await host.request("effects.set", {
      key: "test-subtitle",
      kind: "subtitle",
      filter:
        "subtitles=filename={{file:captions}}:force_style='Fontsize=30,PrimaryColour=&HFFFFFF&'",
      files: { captions: subtitles },
      sessionId: (await host.inspect()).playback.sessionId,
    });
    const subtitled = await start("capture-processed", "subtitled.png");
    await finish();
    const subtitlePixels = await pixels(subtitled);
    assert.ok(
      difference(gradedPixels, subtitlePixels) > 1,
      "实际字幕必须改变 PNG 像素",
    );
    await action("include-subtitles", 0, "include-subtitles", false);
    const excluded = await start("capture-processed", "excluded.png");
    await finish();
    assert.equal(
      difference(gradedPixels, await pixels(excluded)),
      0,
      "排除字幕保留同一画面处理结果",
    );

    await action("burst-count", 3, "burst-count", 3);
    await action("burst-interval", 0.2, "burst-interval", 0.2);
    await start("capture-burst", "burst.png");
    await finish();
    assert.equal((await props())?.["completed-count"], 3);
    for (let index = 1; index <= 3; index++)
      await pixels(
        join(directory, `burst-${String(index).padStart(3, "0")}.png`),
      );

    await host.action(plugin, "burst-count", -1);
    await host.until(async () =>
      String((await props())?.status).includes("失败"),
    );
    assert.equal(
      (await props())?.["burst-count"],
      3,
      "非法值不破坏上次成功配置",
    );

    // 慢处理链确保真实运行时可观察进度并在切换媒体后检查不可变快照。
    await host.request("effects.set", {
      key: "test-slow",
      kind: "video",
      filter: "scale=1280:720,gblur=sigma=4:steps=2",
    });
    await action("clip-duration", 8, "clip-duration", 8);
    const clip = await start("export-clip", "snapshot.mp4");
    await host.until(
      async () =>
        typeof (await props())?.["job-id"] === "string" &&
        (await props())?.["job-id"] !== "",
    );
    const sourceSession = (await props())?.["source-session"];
    await host.request("player.load", { path: alternate });
    await host.request("effects.remove", { key: "test-grade" });
    await host.request("effects.remove", { key: "test-slow" });
    await finish();
    assert.notEqual((await host.inspect()).playback.sessionId, sourceSession);
    const probe: unknown = JSON.parse(
      (
        await exec("ffprobe", [
          "-v",
          "error",
          "-show_streams",
          "-show_format",
          "-of",
          "json",
          clip,
        ])
      ).stdout,
    );
    assert.ok(
      typeof probe === "object" &&
        probe !== null &&
        "streams" in probe &&
        Array.isArray(probe.streams),
    );
    const video: unknown = probe.streams.find(
      (stream: unknown) =>
        typeof stream === "object" &&
        stream !== null &&
        "codec_type" in stream &&
        stream.codec_type === "video",
    );
    assert.ok(
      typeof video === "object" &&
        video !== null &&
        "width" in video &&
        "height" in video,
    );
    assert.equal(video.width, 1280, "保留任务开始时的缩放参数");
    assert.equal(video.height, 720);
    assert.ok(
      "duration" in video && Math.abs(Number(video.duration) - 8) < 0.1,
      "MP4片段实际时长必须匹配请求",
    );
    assert.ok(
      probe.streams.some(
        (stream: unknown) =>
          typeof stream === "object" &&
          stream !== null &&
          "codec_type" in stream &&
          stream.codec_type === "audio",
      ),
      "片段保留真实音轨",
    );
    const clipPixels = (
      await exec(
        "ffmpeg",
        [
          "-v",
          "error",
          "-i",
          clip,
          "-frames:v",
          "1",
          "-vf",
          "scale=1:1",
          "-f",
          "rawvideo",
          "-pix_fmt",
          "rgb24",
          "pipe:1",
        ],
        { encoding: "buffer" },
      )
    ).stdout;
    assert.ok(
      Math.max(...clipPixels) - Math.min(...clipPixels) < 5,
      "导出必须仍是原黑色来源及灰色处理，而非新红色视频",
    );
    assert.ok(
      mean(clipPixels) > 40 && mean(clipPixels) < 100,
      "任务保留原亮度参数快照",
    );

    await host.request("effects.set", {
      key: "test-heavy",
      kind: "video",
      filter: "scale=3840:2160,gblur=sigma=10:steps=3",
    });
    await action("clip-duration", 20, "clip-duration", 20);
    const cancelledOutput = await start("export-clip", "cancelled.mp4");
    await host.until(
      async () => String((await props())?.["job-id"] ?? "") !== "",
    );
    const progressSamples: number[] = [];
    await host.until(async () => {
      const progress = Number((await props())?.progress);
      progressSamples.push(progress);
      return progress > 0 && progress < 1;
    }, 20_000);
    await host.action(plugin, "cancel");
    await host.until(async () => (await props())?.busy === false);
    assert.match(String((await props())?.status), /已取消/);
    const cancelledState = (await props())?.["job-state"];
    assert.equal(cancelledState, "cancelled");
    assert.ok(!(await readdir(directory)).includes("cancelled.mp4"));
    assert.ok(
      !(await readdir(directory)).some((name) => name.startsWith(".vestra-")),
      "取消后临时输出、进度和诊断文件应清理",
    );

    await action("burst-count", 100, "burst-count", 100);
    await start("capture-burst", "cancel-burst.png");
    await host.until(
      async () => String((await props())?.["job-id"] ?? "") !== "",
    );
    await host.action(plugin, "cancel");
    await host.until(async () => (await props())?.busy === false);
    assert.match(String((await props())?.status), /已取消/);
    assert.ok(
      Number((await props())?.["completed-count"]) < 100,
      "连续截图必须在有限组完成前可取消",
    );

    // 任务仍在执行时停用插件，必须回收其子进程、快照和未完成文件。
    await start("export-clip", "unloaded.mp4");
    await host.until(
      async () => String((await props())?.["job-id"] ?? "") !== "",
    );

    await host.unload(plugin);
    assert.deepEqual((await host.inspect()).plugins, []);
    const diagnostics = await host.request<{
      resources: { jobs: number; snapshots: number };
    }>("host.inspect");
    assert.equal(diagnostics.resources.jobs, 0);
    assert.equal(diagnostics.resources.snapshots, 0);
    assert.ok(
      !(await readdir(directory)).some((name) => name.startsWith(".vestra-")),
    );
    assert.ok(!(await readdir(directory)).includes("unloaded.mp4"));
    await writeFile(
      join(directory, "report.json"),
      JSON.stringify(
        {
          experimental: true,
          coverage: ["E1", "E2"],
          rawMean: mean(rawPixels),
          processedMean: mean(gradedPixels),
          subtitleDifference: difference(gradedPixels, subtitlePixels),
          exportedRgb: [...clipPixels],
          progressSamples,
          cancelledState,
          guiVerified: false,
        },
        null,
        2,
      ),
    );
    if (!temporary) console.log(`截图导出真实证据：${directory}`);
  },
);
