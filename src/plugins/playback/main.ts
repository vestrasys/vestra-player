import type { Plugin } from "../../sdk/plugin/index.ts";
import { wireBar } from "./bar.ts";
import { addCommands } from "./commands.ts";
import { Playback } from "./controller.ts";
import { copyToClipboard } from "./copy.ts";
import { parseTime, rateText, time } from "./format.ts";
import { OpenUrl } from "./open-url.ts";
import { Prompt } from "./prompt.ts";
import { Reopener } from "./reopen.ts";
import { SettingsStore } from "./settings.ts";
import { SleepGuard } from "./sleep.ts";

/**
 * Basic playback: the bottom bar, the open/close, 播放 and 声音 commands, the
 * 跳转到 window, and the 播放控制 chapter's features (打开链接, 重开当前/最后文件,
 * 复制到剪贴板, 跳到开头, 播放时禁止屏保, 进度条悬停显示时间).
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const bar = plugin.panels.mount("bottom", "ui/bar.slint");
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: string) => void plugin.osd.show(text).catch(() => {});
  const playback = new Playback(player, plugin.dialogs, () => plugin.update(), osd);

  const jump = new Prompt(
    plugin,
    "playback-jump",
    { windowTitle: "跳转到", groupTitle: "时间定位(时:分:秒.毫秒)", actionLabel: "定位" },
    () => time(player.state.position),
    (value) => {
      const seconds = parseTime(value);
      if (seconds === null || !playback.canSeek() || seconds > player.state.duration!)
        throw new Error("请输入来源时长范围内的HH:MM:SS.mmm");
      const target = playback.seek(seconds);
      if (target !== null) playback.notify(`跳转: ${playback.positionText(target)}`);
    },
  );
  const customRate = new Prompt(
    plugin,
    "playback-rate",
    { windowTitle: "自定义速度", groupTitle: "播放速度(0.25–4倍)", actionLabel: "确定" },
    () => rateText(player.state.rate),
    (value) => playback.setRate(value.trim() ? Number(value) : Number.NaN),
  );

  const dataDirectory = () => plugin.dataDirectory;
  const settings = new SettingsStore(dataDirectory);
  const reopener = new Reopener(plugin, dataDirectory, osd);
  const sleep = new SleepGuard(plugin, settings, osd);
  const openUrl = new OpenUrl(plugin, settings, osd);

  const run = addCommands(plugin, playback, jump, customRate, {
    openUrl,
    reopener,
    sleep,
    settings,
    copy: (kind) => copyToClipboard(player.state, kind, osd),
    jumpToStart: () => playback.jumpToStart(),
  });
  wireBar(bar, player, plugin.panels.sidebar, playback, run, settings);
  plugin.onError((error) => playback.report(error));

  reopener.observe(player.state);
  sleep.sync();

  // Another source, or anyone else's change during a drag, ends the gesture;
  // further pointer moves wait for the next press. A failed load is announced once.
  let { sourceId, phase } = player.state;
  player.onChange(async (state) => {
    const replaced = state.sourceId !== sourceId;
    if (state.phase === "error" && (replaced || phase !== "error"))
      osd(state.error ? `打开失败：${state.error}` : "打开失败");
    sourceId = state.sourceId;
    phase = state.phase;
    reopener.observe(state);
    sleep.sync();
    await Promise.resolve(); // the drag gesture observes the same change first
    if (replaced || playback.gestureLost) {
      playback.clearSeek();
      playback.blockedGesture = true;
      playback.dragging = false;
      bar.setInteraction({ focused: false, dragging: false, popupOpen: false });
    }
    if (replaced) {
      await jump.invalidate();
      await customRate.invalidate();
    }
  });
}
