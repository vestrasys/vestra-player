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
import { Failure, type Osd } from "./texts.ts";
import { TimeJump } from "./time-jump.ts";

/**
 * Playback: the bottom bar and the commands around the core's basic actions
 * (play/pause, stop, ±N seconds, frames, previous/next, speed ±, volume and
 * mute are the player's own, run from the bar with `host.run`): open, the
 * Jump to Time and custom speed windows, and the 播放控制 chapter's features (Open
 * URL, Reopen Current/Last File, Copy to Clipboard, keyframes, seek steps, Jump
 * to Start, Disable Screen Saver While Playing, the seek bar's hover time and
 * thumbnail), and the seek line's marks (B22: `timeline.marks` and the A-B loop).
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const bar = plugin.panels.mount("bottom", "ui/bar.slint");
  // Headless hosts refuse the display; that never fails the action itself.
  const osd: Osd = (text, args) => void (args ? plugin.osd.show(text, args) : plugin.osd.show(text)).catch(() => {});
  // i18n-ignore: the player's (or the SDK's) own message
  const raw = (text: string) => void plugin.osd.show(text).catch(() => {});
  const playback = new Playback(player, plugin.dialogs, () => plugin.update(), osd, raw);

  const jump = new Prompt(
    plugin,
    "playback-jump",
    "jump",
    () => time(player.state.position),
    (value) => {
      const seconds = parseTime(value);
      if (seconds === null || !playback.canSeek() || seconds > player.state.duration!)
        throw new Failure("time-range");
      const target = playback.seek(seconds);
      if (target !== null) playback.notify("Seek: {} / {}", playback.positionArgs(target));
    },
  );
  const customRate = new Prompt(
    plugin,
    "playback-rate",
    "rate",
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
      if (state.error) osd("Open failed: {}", [state.error]);
      else osd("Open failed");
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

