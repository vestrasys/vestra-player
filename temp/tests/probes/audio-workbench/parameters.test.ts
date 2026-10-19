// 实验验收：测试临时音频探针，与最终 API 可能相差很大。
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  neutral,
  preset,
  update,
} from "../../../plugins/audio-workbench/parameters.ts";
import { audioFilter } from "../../../plugins/audio-workbench/filter.ts";

test("音频预设独立、参数有限且控制范围明确", () => {
  const initial = neutral();
  assert.equal(update(initial, "bass", 30).bass, 12);
  assert.equal(update(initial, "treble", -30).treble, -12);
  assert.equal(update(initial, "delay-ms", -5000).delayMs, -2000);
  assert.equal(initial.bass, 0);
  assert.equal(preset(1).voice, true);
  assert.equal(preset(2).voice, false);
  assert.throws(() => update(initial, "mid", Number.NaN));
  assert.throws(() => preset(10));
  assert.throws(() => update(initial, "unknown", 0));
});

test("音频过滤表达式只来自已验证数值，正负延迟语义不同", () => {
  assert.match(
    audioFilter({ ...neutral(), bass: 4 }),
    /equalizer=f=100:t=q:w=1:g=4\.000/,
  );
  assert.match(
    audioFilter({ ...neutral(), normalize: true }),
    /loudnorm=I=-16/,
  );
  assert.match(audioFilter({ ...neutral(), voice: true }), /highpass=f=120/);
  assert.match(audioFilter({ ...neutral(), delayMs: 250 }), /adelay=250:all=1/);
  assert.match(
    audioFilter({ ...neutral(), delayMs: -250 }),
    /atrim=start=0\.250,asetpts=PTS-STARTPTS/,
  );
  assert.throws(() => audioFilter({ ...neutral(), bass: Infinity }));
});
