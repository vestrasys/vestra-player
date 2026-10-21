import type { Plugin } from "../../sdk/plugin/index.ts";
import { currentScale, SCALES } from "./geometry.ts";
import { OnTop } from "./on-top.ts";
import { PictureInPicture } from "./pip.ts";
import { ON_TOP, type OnTop as Mode, Preferences } from "./settings.ts";
import { Sizer, sizeReason } from "./size.ts";
import { VideoSize } from "./video-size.ts";

const SCREEN = ["屏幕"];
const TOP = [...SCREEN, "置顶"];
const MODE_TITLES: Record<Mode, string> = { never: "不置顶", always: "总是置顶", playing: "播放时置顶" };
const MODE_OSD: Record<Mode, string> = { never: "置顶: 关闭", always: "置顶: 总是", playing: "置顶: 播放时" };

/**
 * 主窗口: 屏幕 ▶ 画中画 (H07; 迷你尺寸 where the system has none) / 倍数尺寸 (H03) /
 * 适应视频比例 (H04) / 置顶 (H05), 播放 ▶ 播放时隐藏鼠标指针 (B30). The window itself belongs to the core
 * (`plugin.window`); this plugin keeps the two preferences in settings.json
 * and applies them on activation. The core undoes its effects on deactivation.
 */
export default async function activate(plugin: Plugin) {
  const { player, window } = plugin;
  // Headless players refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const preferences = new Preferences(plugin);
  const video = new VideoSize(plugin);
  // `sizer` is read only once activation has run.
  const onTop = new OnTop(plugin, preferences, () => sizer.mini !== null);
  const sizer = new Sizer(plugin, video, osd, () => void onTop.apply());

  const reason = () => sizeReason(plugin, video);
  const sizeState = {
    enabled: () => reason() === null,
    busy: () => sizer.resizing,
    reason: () => (sizer.resizing ? "正在调整窗口" : reason()),
  };

  // 屏幕 ▶ 画中画 / 迷你尺寸 (710): the first item, so the submenu sits with the host's 全屏 (700, group "screen").
  // The id stays `mini` in both forms, so a user's rebinding of ` carries over.
  // `supported` needs the main window, which may not exist yet at activation,
  // so the form follows `window.onChange`: the command is declared again
  // (title, check, action) whenever `supported` changes.
  const position = { id: "mini", order: 710, group: "screen", bindings: ["`"] };
  const pip = new PictureInPicture(plugin, video, osd);
  const declare = (supported: boolean) =>
    supported
      ? // H07: the system picture-in-picture window (macOS).
        plugin.commands.add({
          ...position,
          path: [...SCREEN, "画中画"],
          checked: () => pip.active,
          enabled: () => pip.active || video.size !== null,
          busy: () => pip.switching,
          reason: () => (pip.switching ? "正在切换画中画" : "没有视频"),
          run: () => pip.toggle(),
        })
      : // Without system picture-in-picture (Windows, Linux, headless): the 3a 迷你尺寸.
        plugin.commands.add({
          ...position,
          path: [...SCREEN, "迷你尺寸"],
          checked: () => sizer.mini !== null,
          enabled: () => !window.state.fullscreen && (sizer.mini !== null || video.size !== null),
          busy: sizeState.busy,
          reason: () => (sizer.resizing ? "正在调整窗口" : window.state.fullscreen ? "全屏时不可用" : "没有视频"),
          run: () => sizer.toggleMini(),
        });
  let form = window.state.pictureInPicture.supported;
  let command = declare(form);
  window.onChange((state) => {
    const supported = state.pictureInPicture.supported;
    if (supported === form) return;
    form = supported;
    void command.dispose();
    command = declare(supported);
    // 迷你尺寸 in effect: back to the size before it (and the chosen on-top mode).
    if (supported) void sizer.leaveMini();
  });
  SCALES.forEach(({ scale, title }, index) =>
    plugin.commands.add({
      id: `size.${scale}`,
      path: [...SCREEN, title],
      order: 711 + index,
      group: "screen-size",
      radio: true,
      bindings: [String(index + 1)],
      checked: () => sizer.mini === null && !window.state.fullscreen && currentScale(window.state.videoArea, video.size) === scale,
      ...sizeState,
      run: () => sizer.scale(scale, title),
    }),
  );
  plugin.commands.add({
    id: "fit",
    path: [...SCREEN, "适应视频比例"],
    order: 715,
    group: "screen-size",
    bindings: ["8"],
    ...sizeState,
    run: () => sizer.fit(),
  });

  // 屏幕 ▶ 置顶 ▶ (725–728).
  const setMode = (mode: Mode) => {
    preferences.update({ onTop: mode });
    void onTop.apply();
    osd(MODE_OSD[mode]);
  };
  plugin.commands.add({
    id: "top.cycle",
    path: [...TOP, "切换置顶方式"],
    order: 725,
    group: "screen-top",
    bindings: ["Ctrl+T"],
    run: () => setMode(ON_TOP[(ON_TOP.indexOf(preferences.value.onTop) + 1) % ON_TOP.length]!),
  });
  ON_TOP.forEach((mode, index) =>
    plugin.commands.add({
      id: `top.${mode}`,
      path: [...TOP, MODE_TITLES[mode]],
      order: 726 + index,
      group: "screen-top-mode",
      radio: true,
      checked: () => preferences.value.onTop === mode,
      run: () => setMode(mode),
    }),
  );

  // 播放 ▶ 播放时隐藏鼠标指针 (241, with playback's 240 / 242).
  const cursor = (on: boolean) => window.setCursorAutoHide(on ? "playing" : "fullscreen").catch(() => {});
  plugin.commands.add({
    id: "hide-cursor",
    path: ["播放", "播放时隐藏鼠标指针"],
    order: 241,
    group: "playback-options",
    checked: () => preferences.value.hideCursorWhilePlaying,
    run: async () => {
      const on = !preferences.value.hideCursorWhilePlaying;
      preferences.update({ hideCursorWhilePlaying: on });
      osd(`播放时隐藏鼠标: ${on ? "开" : "关"}`);
      await cursor(on);
    },
  });

  video.observe(player.state);
  player.onChange((state) => {
    video.observe(state);
    void onTop.apply();
  });
  // Only a change from the core's default is sent: another plugin's last write stays.
  if (preferences.value.hideCursorWhilePlaying) void cursor(true);
  void onTop.apply();
}
