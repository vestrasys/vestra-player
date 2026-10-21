import type { Disposable, Plugin } from "../../sdk/plugin/index.ts";
import { AutoSize } from "./auto-size.ts";
import { currentScale, SCALES } from "./geometry.ts";
import { OnTop } from "./on-top.ts";
import { PictureInPicture } from "./pip.ts";
import { AUTO_SIZES, type AutoSize as AutoChoice, ON_TOP, type OnTop as Mode, Preferences } from "./settings.ts";
import { Sizer, sizeReason } from "./size.ts";
import { VideoSize } from "./video-size.ts";

const SCREEN = ["屏幕"];
const TOP = [...SCREEN, "置顶"];
const AUTO = [...SCREEN, "开始播放时"];
const MODE_TITLES: Record<Mode, string> = {
  never: "不置顶",
  always: "总是置顶",
  playing: "播放时置顶",
  fullscreen: "全屏播放时置顶",
};
const MODE_OSD: Record<Mode, string> = {
  never: "置顶: 关闭",
  always: "置顶: 总是",
  playing: "置顶: 播放时",
  fullscreen: "置顶: 全屏播放时",
};
const AUTO_TITLES = new Map<AutoChoice, string>([
  ["off", "不调整"],
  [0.5, "0.5倍"],
  [1, "1倍"],
  [1.5, "1.5倍"],
  [2, "2倍"],
  ["fit", "适应视频比例"],
]);
/** The settings page (选项 ▶ 播放 ▶ 窗口). */
export const PAGE_ID = "window";

/**
 * 主窗口: 屏幕 ▶ 画中画 (H07, where the system has it) / 迷你尺寸 (IA-WIN-01) /
 * 倍数尺寸 (H03) / 适应视频比例 (H04) / 开始播放时 (G-02) / 置顶 (H05,
 * IA-WIN-03), 播放 ▶ 播放时隐藏鼠标指针 (B30, G-06), and the 窗口 settings
 * page. The window itself belongs to the core (`plugin.window`); this plugin
 * keeps the preferences in settings.json and applies them on activation. The
 * core undoes its effects on deactivation.
 */
export default async function activate(plugin: Plugin) {
  const { player, window } = plugin;
  // Headless players refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  plugin.onError((error) => osd(error.message));

  const preferences = new Preferences(plugin);
  const video = new VideoSize(plugin);
  const onTop = new OnTop(plugin, preferences);
  const sizer = new Sizer(plugin, video, osd);
  const pip = new PictureInPicture(plugin, video, osd);
  const auto = new AutoSize(plugin, preferences, sizer, pip);
  video.onFirstSize = (source, size) => void auto.sized(source, size);

  const reason = () => sizeReason(plugin, video);
  const sizeState = {
    enabled: () => reason() === null,
    busy: () => sizer.resizing,
    reason: () => (sizer.resizing ? "正在调整窗口" : reason()),
  };

  // 屏幕 ▶ 画中画 (709, no default key) / 迷你尺寸 (710, `): the first items, so
  // the submenu sits with the host's 全屏 (700, group "screen").
  // IA-WIN-01: ` is 迷你尺寸 on every system (id `mini`, so a user's rebinding
  // stays with it); 画中画 is Vestra's own extra command (id `pip`). The two
  // exclude each other: one is left before the other starts.
  plugin.commands.add({
    id: "mini",
    path: [...SCREEN, "迷你尺寸"],
    order: 710,
    group: "screen",
    bindings: ["`"],
    checked: () => sizer.mini !== null,
    enabled: () => !window.state.fullscreen && (sizer.mini !== null || video.size !== null),
    busy: () => sizer.resizing || pip.switching,
    reason: () =>
      pip.switching
        ? "正在切换画中画"
        : sizer.resizing
          ? "正在调整窗口"
          : window.state.fullscreen
            ? "全屏时不可用"
            : "没有视频",
    run: async () => {
      if (!sizer.mini && pip.active) {
        await pip.toggle();
        if (pip.active) return; // could not leave 画中画: its failure was shown
      }
      await sizer.toggleMini();
    },
  });
  // `supported` needs the main window, which may not exist yet at activation,
  // so 画中画 follows `window.onChange`: added while supported (macOS 14+).
  const pipCommand = () =>
    plugin.commands.add({
      id: "pip",
      path: [...SCREEN, "画中画"],
      order: 709,
      group: "screen",
      checked: () => pip.active,
      enabled: () => pip.active || video.size !== null,
      busy: () => pip.switching || sizer.resizing,
      reason: () => (pip.switching ? "正在切换画中画" : sizer.resizing ? "正在调整窗口" : "没有视频"),
      run: async () => {
        // 迷你尺寸 in effect: back to the size before it first.
        if (!pip.active && sizer.mini) await sizer.leaveMini();
        await pip.toggle();
      },
    });
  let pipEntry: Disposable | null = window.state.pictureInPicture.supported ? pipCommand() : null;
  window.onChange((state) => {
    const supported = state.pictureInPicture.supported;
    if (supported && !pipEntry) pipEntry = pipCommand();
    if (!supported && pipEntry) {
      void pipEntry.dispose();
      pipEntry = null;
    }
    void onTop.apply();
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

  // 屏幕 ▶ 开始播放时 ▶ (716–722, G-02).
  const setAuto = (choice: AutoChoice, say = true) => {
    preferences.update({ autoSize: choice });
    if (say) osd(choice === "off" ? "开始播放时: 不调整窗口" : `开始播放时: ${AUTO_TITLES.get(choice)}`);
  };
  const setAutoOnce = (on: boolean, say = true) => {
    preferences.update({ autoSizeOnce: on });
    if (say) osd(`开始播放时只调整一次: ${on ? "开" : "关"}`);
  };
  AUTO_SIZES.forEach((choice, index) =>
    plugin.commands.add({
      id: `auto.${choice}`,
      path: [...AUTO, AUTO_TITLES.get(choice)!],
      order: 716 + index,
      group: "screen-auto",
      radio: true,
      checked: () => preferences.value.autoSize === choice,
      run: () => setAuto(choice),
    }),
  );
  plugin.commands.add({
    id: "auto.once",
    path: [...AUTO, "只调整一次"],
    order: 722,
    group: "screen-auto-once",
    checked: () => preferences.value.autoSizeOnce,
    run: () => setAutoOnce(!preferences.value.autoSizeOnce),
  });

  // 屏幕 ▶ 置顶 ▶ (725–729).
  const setMode = (mode: Mode, say = true) => {
    preferences.update({ onTop: mode });
    void onTop.apply();
    if (say) osd(MODE_OSD[mode]);
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
  const setHideCursor = async (on: boolean, say = true) => {
    preferences.update({ hideCursorWhilePlaying: on });
    if (say) osd(`播放时隐藏鼠标: ${on ? "开" : "关"}`);
    await cursor(on);
  };
  plugin.commands.add({
    id: "hide-cursor",
    path: ["播放", "播放时隐藏鼠标指针"],
    order: 241,
    group: "playback-options",
    checked: () => preferences.value.hideCursorWhilePlaying,
    run: () => setHideCursor(!preferences.value.hideCursorWhilePlaying),
  });

  // 选项 ▶ 播放 ▶ 窗口: the same preferences as the menu items.
  const page = plugin.settings.page("ui/page.slint", { id: PAGE_ID, category: "playback", title: "窗口", order: 30 });
  page.bind(() => ({
    autoIndex: AUTO_SIZES.indexOf(preferences.value.autoSize),
    autoOnce: preferences.value.autoSizeOnce,
    topIndex: ON_TOP.indexOf(preferences.value.onTop),
    hideCursor: preferences.value.hideCursorWhilePlaying,
  }));
  page.on("setAuto", (index: number) => AUTO_SIZES[index] !== undefined && setAuto(AUTO_SIZES[index]!, false));
  page.on("setAutoOnce", (on: boolean) => setAutoOnce(on, false));
  page.on("setTop", (index: number) => ON_TOP[index] !== undefined && setMode(ON_TOP[index]!, false));
  page.on("setHideCursor", (on: boolean) => void setHideCursor(on, false));

  const follow = (state: typeof player.state) => {
    auto.observe(state.sourceId);
    video.observe(state);
  };
  follow(player.state);
  player.onChange((state) => {
    follow(state);
    void onTop.apply();
  });
  // Only a change from the core's default is sent: another plugin's last write stays.
  if (preferences.value.hideCursorWhilePlaying) void cursor(true);
  void onTop.apply();
}
