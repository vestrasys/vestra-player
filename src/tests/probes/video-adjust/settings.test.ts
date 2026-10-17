// 实验测试；验证用户参数语义，不冻结最终 API。
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  change,
  filter,
  isNeutral,
  preset,
} from "../../../plugins/video-adjust/settings.ts";

test("颜色参数拒绝不支持的数值且不损坏之前的状态", () => {
  const initial = preset("neutral");
  for (const invalid of [Number.NaN, Number.POSITIVE_INFINITY, 1.01, -1.01]) {
    assert.throws(() => change(initial, "brightness", invalid), RangeError);
  }
  assert.throws(() => change(initial, "hue", 181), RangeError);
  assert.throws(() => change(initial, "contrast", -0.1), RangeError);
  assert.throws(() => change(initial, "saturation", 3.1), RangeError);
  assert.deepEqual(initial, {
    brightness: 0,
    contrast: 1,
    saturation: 1,
    hue: 0,
  });
});

test("预设是独立值且可以继续编辑和复位", () => {
  const bright = preset("bright");
  assert.ok(bright.brightness > 0);
  const edited = change(bright, "hue", 45);
  assert.equal(edited.hue, 45);
  assert.equal(bright.hue, 0);
  assert.equal(preset("bright").hue, 0);
  assert.ok(isNeutral(preset("neutral")));
  assert.equal(isNeutral(edited), false);
});

test("FFmpeg表达式包括四个参数，禁止绕过模型传递非有限参数", () => {
  const settings = { brightness: 0.2, contrast: 1.4, saturation: 0.5, hue: 90 };
  assert.equal(
    filter(settings),
    "eq=brightness=0.2:contrast=1.4:saturation=0.5,hue=h=90",
  );
  assert.throws(() => filter({ ...settings, hue: Number.NaN }), RangeError);
});
