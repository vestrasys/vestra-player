// 实验集成测试：真实 Rust 宿主、Node 插件和 FFmpeg 像素，不替代 GUI 验收。
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../../support/host.ts";
import type { JobState } from "../../../sdk/types.ts";

const exec = promisify(execFile);
type Resource = { owner: string; key: string; kind: string; filter: string };

function difference(a: Buffer, b: Buffer): number {
  assert.equal(a.length, b.length);
  let total = 0;
  for (let i = 0; i < a.length; i++) total += Math.abs(a[i] - b[i]);
  return total / a.length;
}

function mean(data: Buffer): number {
  return data.reduce((sum, value) => sum + value, 0) / data.length;
}

test(
  "V1-V3：真实宿主色彩处理、独立 owner 共存、更新顺序与卸载隔离",
  { timeout: 90_000 },
  async (t) => {
    const temporary = !process.env.VESTRA_VIDEO_EVIDENCE;
    const directory = process.env.VESTRA_VIDEO_EVIDENCE
      ? resolve(process.env.VESTRA_VIDEO_EVIDENCE)
      : await mkdtemp(join(tmpdir(), "vestra-video-adjust-"));
    await mkdir(directory, { recursive: true });
    if (temporary)
      t.after(() => rm(directory, { recursive: true, force: true }));
    const input = join(directory, "source.mp4");
    await exec("ffmpeg", [
      "-v",
      "error",
      "-y",
      "-f",
      "lavfi",
      "-i",
      "testsrc2=size=160x120:rate=5:duration=1",
      "-c:v",
      "libx264",
      "-pix_fmt",
      "yuv420p",
      input,
    ]);
    const host = new TestHost();
    t.after(() => host.close());
    const manifest = fileURLToPath(
      new URL("../../../plugins/video-adjust/manifest.json", import.meta.url),
    );
    await host.load(manifest);
    const properties = async () =>
      (await host.inspect()).plugins.find(
        (plugin) => plugin.id === "video-adjust",
      )?.properties;
    await host.until(
      async () => (await properties())?.status === "画面调节就绪",
    );
    const resources = () => host.request<Resource[]>("effects.list");
    const waitValue = (key: string, value: number) =>
      host.until(async () => (await properties())?.[key] === value);
    const measures: Record<string, number> = {};

    async function image(name: string): Promise<Buffer> {
      const output = join(directory, `${name}.png`);
      await rm(output, { force: true });
      const job = await host.request<JobState>("media.start", {
        input,
        output,
        format: "image",
        processing: "current",
      });
      await host.until(async () => {
        const status = await host.request<JobState>("media.status", {
          id: job.id,
        });
        if (status.state === "failed" || status.state === "cancelled")
          throw new Error(status.error ?? status.state);
        return status.state === "succeeded";
      });
      return (
        await exec(
          "ffmpeg",
          [
            "-v",
            "error",
            "-i",
            output,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
          ],
          { encoding: "buffer", maxBuffer: 2_000_000 },
        )
      ).stdout;
    }

    const raw = await image("raw");
    // $cli 是独立测试客户端 owner，既不冒用此插件，也不依赖兄弟插件。
    await host.request("effects.set", {
      key: "independent-blur",
      kind: "video",
      filter: "gblur=sigma=2:steps=2",
    });
    const blurred = await image("independent-blur");
    assert.ok(difference(raw, blurred) > 2, "第二 owner 必须确实影响像素");

    await host.action("video-adjust", "brightness", 0.2);
    await waitValue("brightness", 0.2);
    const both = await image("both-brightness");
    measures.brightnessMeanGain = mean(both) - mean(blurred);
    assert.ok(measures.brightnessMeanGain > 15, "亮度增加必须改变真实像素均值");
    const chain = await resources();
    assert.equal(chain.length, 2);
    assert.notEqual(chain[0].owner, chain[1].owner);
    const order = chain.map((resource) => `${resource.owner}/${resource.key}`);
    await host.action("video-adjust", "contrast", 1.4);
    await waitValue("contrast", 1.4);
    assert.deepEqual(
      (await resources()).map(
        (resource) => `${resource.owner}/${resource.key}`,
      ),
      order,
      "更新资源不能悄悄改变处理顺序",
    );

    for (const [parameter, value] of [
      ["contrast", 1.5],
      ["saturation", 0],
      ["hue", 90],
    ] as const) {
      await host.action("video-adjust", "reset");
      await host.until(async () => (await resources()).length === 1);
      await host.action("video-adjust", parameter, value);
      await waitValue(parameter, value);
      measures[`${parameter}Difference`] = difference(
        blurred,
        await image(parameter),
      );
      assert.ok(
        measures[`${parameter}Difference`] > 3,
        `${parameter} 必须改变真实像素`,
      );
    }

    await host.action("video-adjust", "preset-bright");
    await waitValue("brightness", 0.12);
    await host.action("video-adjust", "brightness-up");
    await waitValue("brightness", 0.17);
    await host.action("video-adjust", "saturation", -1);
    await host.until(async () =>
      String((await properties())?.status).includes("失败"),
    );
    assert.equal(
      (await properties())?.saturation,
      1.05,
      "无效输入不能破坏最近成功配置",
    );

    await host.action("video-adjust", "reset");
    await host.until(async () => (await resources()).length === 1);
    assert.deepEqual(
      await resources(),
      [chain[0]],
      "复位只撤销自己，保留另一 owner 原始资源",
    );
    assert.equal(difference(blurred, await image("after-reset")), 0);

    await host.action("video-adjust", "preset-cinema");
    await waitValue("brightness", -0.04);
    await host.unload("video-adjust");
    assert.deepEqual(await resources(), [chain[0]], "卸载同样保留另一 owner");
    assert.equal(difference(blurred, await image("after-unload")), 0);
    await host.request("effects.remove", { key: "independent-blur" });
    assert.equal(difference(raw, await image("after-all-removed")), 0);
    await writeFile(
      join(directory, "report.json"),
      JSON.stringify(
        {
          experimental: true,
          coverage: ["V1", "V2", "V3"],
          measures,
          processingOrder: order,
          guiVerified: false,
        },
        null,
        2,
      ),
    );
  },
);
