import type { FilterStatus, Plugin } from "../../sdk/plugin/index.ts";
import { clamp, defaults, nodesFor, presets } from "./model.ts";
import type { Picture } from "./model.ts";

type Color = "brightness" | "contrast" | "saturation" | "gamma";
const colors: readonly Color[] = ["brightness", "contrast", "saturation", "gamma"];
/** Keyboard steps (reference player W/E, R/T, Y/U): label, field, keys for −1 and +1. */
const steps: readonly [string, Color, string, string][] = [
  ["亮度", "brightness", "W", "E"],
  ["对比度", "contrast", "R", "T"],
  ["饱和度", "saturation", "Y", "U"],
];
const signed = (n: number) => (n > 0 ? `+${n}` : `${n}`);

/** Picture tool: one window, one owned video filter, menu commands with reference player keys. */
export default async function activate(plugin: Plugin) {
  let picture: Picture = { ...defaults };
  let enabled = true;

  const filter = plugin.video.filter("picture");
  const window = plugin.windows.create("picture.slint");
  const apply = () => filter.set(enabled ? nodesFor(picture) : []);
  /** Best-effort feedback: a missing OSD (headless) must not look like a picture failure. */
  const osd = (text: string) => void plugin.osd.show(text).catch(() => undefined);

  window.bind(() => ({
    ...picture,
    effectEnabled: enabled,
    failed: filter.status.state === "failed",
    status: describe(filter.status),
    detail: filter.status.state === "failed" ? filter.status.error.message : "",
  }));
  window.on("adjust", (field: string, value: number) => {
    if (!Number.isFinite(value)) return;
    if (field === "blurStrength") picture.blurStrength = clamp(value, 0, 10);
    else if ((colors as string[]).includes(field)) picture[field as Color] = clamp(value, -100, 100);
    else return;
    apply();
  });
  window.on("resetField", (field: string) => {
    if (field === "blurStrength") picture.blurStrength = defaults.blurStrength;
    else if ((colors as string[]).includes(field)) picture[field as Color] = defaults[field as Color];
    else return;
    apply();
  });
  window.on("preset", (name: string) => {
    const preset = presets[name];
    if (!preset) return;
    picture = { ...preset };
    apply();
  });
  window.on("toggleEffect", (on: boolean) => {
    enabled = on;
    apply();
  });
  window.on("toggleBlur", (on: boolean) => {
    picture.blurEnabled = on;
    apply();
  });
  window.on("retry", () => filter.retry());

  const reset = () => {
    picture = { ...defaults };
    apply();
    osd("画面调节已复位");
  };
  const toggleBlur = () => {
    picture.blurEnabled = !picture.blurEnabled;
    apply();
    osd(`高斯模糊: ${picture.blurEnabled ? "开" : "关"}`);
  };
  const step = (label: string, field: Color, delta: number) => {
    picture[field] = clamp(Math.round(picture[field]) + delta, -100, 100);
    apply();
    osd(`${label} ${delta > 0 ? "+" : "-"}1% (${signed(picture[field])}%)`);
  };

  // 视频 ▶ 401–419 (round allocation): the window first, then reference player's color keys.
  const group = "domain";
  plugin.commands.add({ id: "video.picture", path: ["视频", "画面调节…"], order: 401, group, run: () => window.show() });
  plugin.commands.add({ id: "video.reset", path: ["视频", "复位"], order: 402, group, bindings: ["Q"], run: reset });
  steps.forEach(([label, field, down, up], i) => {
    plugin.commands.add({
      id: `video.${field}.down`,
      path: ["视频", `${label} -1%`],
      order: 403 + i * 2,
      group,
      bindings: [down],
      repeat: true,
      run: () => step(label, field, -1),
    });
    plugin.commands.add({
      id: `video.${field}.up`,
      path: ["视频", `${label} +1%`],
      order: 404 + i * 2,
      group,
      bindings: [up],
      repeat: true,
      run: () => step(label, field, 1),
    });
  });
  plugin.commands.add({
    id: "video.gaussian",
    path: ["视频", "高斯模糊"],
    order: 409,
    group,
    bindings: ["Ctrl+B"],
    checked: () => picture.blurEnabled,
    run: toggleBlur,
  });
  apply();
}

/** One short Chinese line; only a failure needs words (the detail goes in a tooltip). */
function describe(status: FilterStatus) {
  return status.state === "failed" ? "画面处理失败" : "";
}
