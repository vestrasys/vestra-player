import type { Plugin } from "@vestra-player/sdk";
import { wireBar } from "./bar.ts";
import { addCommands } from "./commands.ts";
import { Playback } from "./controller.ts";
import { copyToClipboard } from "./copy.ts";
import { parseTime, rateText, time } from "./format.ts";
import { OpenUrl } from "./open-url.ts";
import { Keyframes } from "./keyframes.ts";
import { Prompt } from "./prompt.ts";
import { Reopener } from "./reopen.ts";
import { DEFAULT_STEPS, type SeekSteps, SettingsStore } from "./settings.ts";
import { Thumbnails } from "./thumbs.ts";
import { TimeJump } from "./time-jump.ts";

/**
 * Playback: the bottom bar and the commands around the core's basic actions
 * (play/pause, stop, ±N seconds, frames, previous/next, speed ±, volume and
 * mute are the player's own, run from the bar with `host.run`): open, the
 * 跳转到 and custom speed windows, and the 播放控制 chapter's features (打开链接,
 * 重开当前/最后文件, 复制到剪贴板, 关键帧, 跳转秒数, 跳到开头, 播放时禁止屏保,
 * 进度条悬停显示时间 and 缩略图), and the seek line's marks (B22: `timeline.marks`
 * and the A-B loop).
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

  const settings = new SettingsStore(plugin);
  const reopener = new Reopener(plugin, osd);
  const openUrl = new OpenUrl(plugin, settings, osd);
  const keyframes = new Keyframes(plugin, playback, () => plugin.update());
  const thumbnails = new Thumbnails(plugin, settings, (id) => keyframes.video(id));
  // B29: the host holds the assertion while playing; a failed start is only in its log.
  const sleep = () => plugin.power.preventSleep(settings.value.preventSleep ? "playing" : "off");
  // B06: tier 1 is the core's ← / →; the plugin gives it the step and keeps tiers 2–4.
  const stepSeek = (seconds: number) => player.setStepSeek({ seconds }).catch((error) => playback.report(error));
  let commands: { setSteps(steps: SeekSteps): Promise<void> } | null = null;
  const timeJump = new TimeJump(plugin, settings, async (steps) => {
    await stepSeek(steps[0]);
    await commands?.setSteps(steps);
  });

  commands = addCommands(plugin, playback, jump, customRate, {
    openUrl,
    reopener,
    keyframes,
    timeJump,
    sleep: () => void sleep().catch((error) => playback.report(error)),
    settings,
    copy: (kind) => copyToClipboard(plugin.clipboard, player.state, kind, osd),
    jumpToStart: () => playback.jumpToStart(),
    thumbnails,
  });
  wireBar(bar, plugin, playback, settings, thumbnails);
  plugin.onError((error) => playback.report(error));

  reopener.observe(player.state);
  keyframes.observe(player.state);
  thumbnails.observe(player.state);
  void thumbnails.probe();
  if (settings.value.preventSleep) void sleep().catch((error) => playback.report(error));
  // The core's step is 5 until someone sets it; it returns to 5 when this plugin stops.
  if (settings.value.seekSteps[0] !== DEFAULT_STEPS[0]) void stepSeek(settings.value.seekSteps[0]);

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
    keyframes.observe(state);
    thumbnails.observe(state);
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

