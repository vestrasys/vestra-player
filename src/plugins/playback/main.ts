import type { Plugin } from "@vestra-player/sdk";
import { wireBar } from "./bar.ts";
import { addCommands } from "./commands.ts";
import { Playback } from "./controller.ts";
import { copyToClipboard } from "./copy.ts";
import { OpenUrl } from "./open-url.ts";
import { OutputMemory } from "./output-memory.ts";
import { Keyframes } from "./keyframes.ts";
import { Reopener } from "./reopen.ts";
import { PlaybackPanel } from "./panel.ts";
import { type SeekSteps, SettingsStore } from "./settings.ts";
import { Thumbnails } from "./thumbs.ts";
import type { Osd } from "./texts.ts";

/**
 * Playback: the bottom bar and the commands around the core's basic actions
 * (play/pause, stop, ±N seconds, frames, previous/next, speed ±, volume and
 * mute are the player's own, run from the bar with `host.run`): open, the
 * Playback panel in the adjust column (speed, jump, steps and the seek steps;
 * Playback…, Alt+4), and the 播放控制 chapter's features (Open
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

  const settings = new SettingsStore(plugin);
  // The last volume and mute come back once at start; later changes by anyone are saved.
  const output = new OutputMemory(plugin, player);
  void output.restore();
  const reopener = new Reopener(plugin, osd);
  const openUrl = new OpenUrl(plugin, settings, osd);
  const keyframes = new Keyframes(plugin, playback, () => plugin.update());
  const thumbnails = new Thumbnails(plugin, settings, (id) => keyframes.video(id));
  // B29: the host holds the assertion while playing; a failed start is only in its log.
  const sleep = () => plugin.power.preventSleep(settings.value.preventSleep ? "playing" : "off");
  // B06: tier 1 is the core's ← / →; the plugin gives it the step and the mode
  // (as a pair, on every start: the core's own default is exact) and keeps tiers 2–4.
  const stepSeek = () =>
    player
      .setStepSeek({ seconds: settings.value.seekSteps[0], keyframes: settings.value.seekKeyframes })
      .catch((error) => playback.report(error));
  // Tiers 2–4, read at each press. Keyframe mode is the core's step seek with
  // another step: it lands, adds up with ← / → and shows the landed position
  // itself, so no text here. Exact mode is the plugin's own seek, as before.
  const seekBy = (delta: number) =>
    settings.value.seekKeyframes
      ? plugin.host
          .run(delta < 0 ? "seek-backward" : "seek-forward", { seconds: Math.abs(delta) })
          .catch((error) => playback.report(error))
      : playback.seekBy(delta);
  let commands: { setSteps(steps: SeekSteps): Promise<void> } | null = null;
  // B06: the seek steps and "Jump to keyframes only", saved and applied at once (the
  // Playback panel's Seek steps): the core's step, and the tier 2–4 titles when they changed.
  const applySteps = async (steps: SeekSteps, seekKeyframes: boolean) => {
    const changed = steps.some((step, index) => step !== settings.value.seekSteps[index]);
    settings.update((s) => {
      s.seekSteps = steps;
      s.seekKeyframes = seekKeyframes;
    });
    await stepSeek();
    if (changed) await commands?.setSteps(steps);
  };
  const jumpToStart = () => playback.jumpToStart();
  const panel = new PlaybackPanel(plugin, { playback, keyframes, settings, applySteps, jumpToStart });

  commands = addCommands(plugin, plugin.i18n, playback, {
    openUrl,
    reopener,
    keyframes,
    panel,
    sleep: () => void sleep().catch((error) => playback.report(error)),
    settings,
    copy: (kind) => copyToClipboard(plugin.clipboard, player.state, kind, osd),
    jumpToStart,
    thumbnails,
    seekBy,
  });
  wireBar(bar, plugin, playback, settings, thumbnails, output);
  plugin.onError((error) => playback.report(error));

  reopener.observe(player.state);
  keyframes.observe(player.state);
  thumbnails.observe(player.state);
  void thumbnails.probe();
  if (settings.value.preventSleep) void sleep().catch((error) => playback.report(error));
  // The core's step is 5 s exact until someone sets it, and returns to that when this plugin stops.
  void stepSeek();

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
    output.observe(state);
    await Promise.resolve(); // the drag gesture observes the same change first
    if (replaced || playback.gestureLost) {
      playback.clearSeek();
      playback.blockedGesture = true;
      playback.dragging = false;
      bar.setInteraction({ focused: false, dragging: false, popupOpen: false });
    }
    if (replaced) panel.sourceChanged();
  });
}

