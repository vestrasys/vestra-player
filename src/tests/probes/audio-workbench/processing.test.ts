// 实验真实音频证据：通过公共媒体任务执行后测 PCM，不以请求 ack 代替结果。
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { TestHost } from "../../support/host.ts";
import {
  decode,
  firstSignalSeconds,
  impulse,
  magnitude,
  rms,
  tones,
  writeWave,
} from "./metrics.ts";

function job(value: unknown): { id: string; state: string } {
  assert.ok(
    typeof value === "object" &&
      value !== null &&
      "id" in value &&
      "state" in value,
  );
  assert.equal(typeof value.id, "string");
  assert.equal(typeof value.state, "string");
  return { id: String(value.id), state: String(value.state) };
}

test(
  "Rust 公共媒体任务实际改变 EQ/人声/响度并实现正负延迟",
  { timeout: 120000 },
  async (t) => {
    const root = await mkdtemp(join(tmpdir(), "vestra-audio-"));
    const h = new TestHost();
    try {
      await h.load(
        fileURLToPath(
          new URL(
            "../../../plugins/audio-workbench/manifest.json",
            import.meta.url,
          ),
        ),
      );
      async function action(
        name: string,
        value: number,
        property: string,
        expected: number | boolean,
      ) {
        await h.action("audio-workbench", name, value);
        await h.until(
          async () =>
            (await h.inspect()).plugins.find(
              (plugin) => plugin.id === "audio-workbench",
            )?.properties[property] === expected,
        );
      }
      const source = join(root, "tones.wav");
      await writeWave(source, tones([100, 1000, 10000]));
      let next = 0;
      async function processAudio(input: string) {
        const output = join(root, `output-${next++}.wav`);
        const started = job(
          await h.request("media.start", {
            input,
            output,
            format: "audio",
            processing: "current",
          }),
        );
        for (let attempt = 0; attempt < 300; attempt++) {
          const current = job(
            await h.request("media.status", { id: started.id }),
          );
          if (current.state === "succeeded") return decode(output);
          assert.equal(
            current.state,
            "running",
            `音频处理失败 ${JSON.stringify(current)}`,
          );
          await delay(20);
        }
        throw new Error("音频任务未完成");
      }
      const original = decode(source);
      await action("bass", 9, "bass", 9);
      await action("mid", -9, "mid", -9);
      await action("treble", 6, "treble", 6);
      const eq = await processAudio(source);
      assert.ok(
        magnitude(eq, 100) / magnitude(original, 100) > 2,
        "低频应提升",
      );
      assert.ok(
        magnitude(eq, 1000) / magnitude(original, 1000) < 0.5,
        "中频应降低",
      );
      assert.ok(
        magnitude(eq, 10000) / magnitude(original, 10000) > 1.6,
        "高频应提升",
      );
      await action("reset", 0, "bass", 0);
      await action("normalize", 1, "normalize", true);
      const normalized = await processAudio(source);
      assert.ok(
        rms(normalized) > rms(original) * 2,
        "低电平音源规范化应提高响度",
      );
      const speechSource = join(root, "speech-tones.wav");
      await writeWave(speechSource, tones([50, 2500, 15000]));
      const speechOriginal = decode(speechSource);
      await action("reset", 0, "normalize", false);
      await action("voice", 1, "voice", true);
      const speech = await processAudio(speechSource);
      assert.ok(
        magnitude(speech, 2500) > magnitude(speechOriginal, 2500) * 1.3,
      );
      assert.ok(magnitude(speech, 50) < magnitude(speechOriginal, 50) * 0.4);
      assert.ok(
        magnitude(speech, 15000) < magnitude(speechOriginal, 15000) * 0.4,
      );
      const pulse = join(root, "pulse.wav");
      await writeWave(pulse, impulse());
      await action("reset", 0, "voice", false);
      await action("delay-ms", 250, "delay-ms", 250);
      const later = await processAudio(pulse);
      await action("delay-ms", -250, "delay-ms", -250);
      const earlier = await processAudio(pulse);
      assert.ok(Math.abs(firstSignalSeconds(later) - 1.25) < 0.003);
      assert.ok(Math.abs(firstSignalSeconds(earlier) - 0.75) < 0.003);
      t.diagnostic(
        JSON.stringify({
          eqRatios: [100, 1000, 10000].map(
            (frequency) =>
              magnitude(eq, frequency) / magnitude(original, frequency),
          ),
          normalizedRmsRatio: rms(normalized) / rms(original),
          voiceRatios: [50, 2500, 15000].map(
            (frequency) =>
              magnitude(speech, frequency) /
              magnitude(speechOriginal, frequency),
          ),
          delayedPulseSeconds: firstSignalSeconds(later),
          advancedPulseSeconds: firstSignalSeconds(earlier),
        }),
      );
    } finally {
      await h.close();
      await rm(root, { recursive: true, force: true });
    }
  },
);

test(
  "独立 worker 参数跨媒体保留，卸载仅移除自己的效果且实际音频回到其他 owner 的处理",
  { timeout: 60000 },
  async () => {
    const root = await mkdtemp(join(tmpdir(), "vestra-audio-lifecycle-"));
    const h = new TestHost();
    try {
      for (const name of ["first.mp4", "second.mp4"]) {
        execFileSync("ffmpeg", [
          "-v",
          "error",
          "-f",
          "lavfi",
          "-i",
          "color=c=black:s=64x64:d=2",
          "-f",
          "lavfi",
          "-i",
          "sine=frequency=1000:duration=2",
          "-c:v",
          "libx264",
          "-preset",
          "ultrafast",
          "-c:a",
          "aac",
          "-shortest",
          join(root, name),
        ]);
      }
      // The CLI connection is an independent owner; it cannot impersonate a plugin.
      await h.request("effects.set", {
        key: "foreign",
        kind: "audio",
        filter: "volume=0.5",
      });
      await h.load(
        fileURLToPath(
          new URL(
            "../../../plugins/audio-workbench/manifest.json",
            import.meta.url,
          ),
        ),
      );
      await h.action("audio-workbench", "preset-dialogue");
      await h.until(
        async () =>
          (await h.inspect()).plugins.find(
            (plugin) => plugin.id === "audio-workbench",
          )?.properties.voice === true,
      );
      await h.request("player.load", { path: join(root, "first.mp4") });
      const firstSession = (await h.inspect()).playback.sessionId;
      await h.request("player.load", { path: join(root, "second.mp4") });
      const inspected = await h.inspect();
      assert.notEqual(inspected.playback.sessionId, firstSession);
      assert.equal(inspected.plugins.length, 1, "没有隐含安装其他探针");
      assert.equal(inspected.plugins[0]?.properties.voice, true);
      const effects = await h.request<unknown[]>("effects.list");
      assert.equal(effects.length, 2);
      await h.unload("audio-workbench");
      const remaining = await h.request<unknown[]>("effects.list");
      assert.deepEqual(remaining, [
        {
          owner: "$cli",
          key: "foreign",
          kind: "audio",
          filter: "volume=0.5",
        },
      ]);
      const output = join(root, "remaining.wav");
      const started = job(
        await h.request("media.start", {
          input: join(root, "second.mp4"),
          output,
          format: "audio",
          processing: "current",
        }),
      );
      await h.until(
        async () =>
          job(await h.request("media.status", { id: started.id })).state ===
          "succeeded",
      );
      const baseline = decode(join(root, "second.mp4"));
      const processed = decode(output);
      assert.ok(
        Math.abs(rms(processed) / rms(baseline) - 0.5) < 0.02,
        "停用后必须真实只保留另一 owner 的 0.5 音量",
      );
    } finally {
      await h.close();
      await rm(root, { recursive: true, force: true });
    }
  },
);
