import type {
  FilterStatus,
  Plugin,
  PlayerState,
} from "../../sdk/plugin/index.ts";
import { clamp, defaults, nodesFor, presets } from "./model.ts";

/** Picture tool: one window, one owned video filter, two menu entrances. */
export default async function activate(plugin: Plugin) {
  let picture = { ...defaults };
  let enabled = true;
  let problem = "";

  const filter = plugin.video.filter("picture");
  const window = plugin.windows.create("picture.slint");
  const apply = () => {
    problem = "";
    filter.set(enabled ? nodesFor(picture) : []);
  };

  window.bind(() => ({
    ...picture,
    effectEnabled: enabled,
    status: problem || describe(filter.status, plugin.player.state),
  }));
  window.on("adjust", (field: string, value: number) => {
    if (!Number.isFinite(value)) return;
    if (field === "blurStrength") picture.blurStrength = clamp(value, 0, 20);
    else if (field === "brightness" || field === "contrast" || field === "saturation" || field === "gamma")
      picture[field] = clamp(value, -100, 100);
    else return;
    apply();
  });
  window.on("preset", (name: string) => {
    const preset = presets[name];
    if (!preset) return;
    picture = { ...preset };
    apply();
  });
  window.on("reset", () => {
    picture = { ...defaults };
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
  window.on("hideTool", () => window.hide());
  plugin.onError((error) => (problem = `窗口更新失败：${error.message}`));

  // Both entrances open the same tool; hiding it keeps the effect.
  plugin.commands.add({ id: "video.picture", path: ["视频", "画面调节…"], run: () => window.show() });
  plugin.commands.add({ id: "video.gaussian", path: ["视频", "高斯模糊…"], run: () => window.show() });
  apply();
}

function describe(status: FilterStatus, player: PlayerState) {
  switch (status.state) {
    case "waiting":
      return player.phase === "empty"
        ? "没有媒体；已保留设置"
        : `媒体状态 ${player.phase}；已保留设置`;
    case "applying":
      return "正在应用画面设置…";
    case "applied":
      return player.paused ? "已接受；暂停画面可能在继续播放后更新" : "画面设置已接受";
    case "failed":
      return `画面处理失败：${status.error.message}`;
  }
}
