import type { Disposable, Plugin } from "@vestra-player/sdk";
import { AutoSize } from "./auto-size.ts";
import { currentScale, SCALES } from "./geometry.ts";
import { OnTop } from "./on-top.ts";
import { quiet } from "./osd.ts";
import { PictureInPicture } from "./pip.ts";
import { AUTO_SIZES, type AutoSize as AutoChoice, ON_TOP, type OnTop as Mode, Preferences } from "./settings.ts";
import { Sizer, sizeReason } from "./size.ts";
import { VideoSize } from "./video-size.ts";

/** Always on Top ▸ <mode>, in `ON_TOP` order. */
const MODE_TITLES = {
  never: "Never",
  always: "Always",
  playing: "While Playing",
  fullscreen: "While Playing in Fullscreen",
} as const satisfies Record<Mode, string>;
const MODE_OSD = {
  never: "Always on top: Off",
  always: "Always on top: Always",
  playing: "Always on top: While playing",
  fullscreen: "Always on top: While playing in fullscreen",
} as const satisfies Record<Mode, string>;
/** When Playback Starts ▸ <choice>, in `AUTO_SIZES` order. */
const AUTO_TITLES = ["Don't Resize", "0.5×", "1×", "1.5×", "2×", "Fit to Video Aspect Ratio"] as const;
/** The settings page (Options ▸ Playback ▸ Window). */
export const PAGE_ID = "window";

/**
 * Main Window: Picture-in-Picture (H07, where the system has it) /
 * Mini Size (IA-WIN-01) / 0.5× … 2× (H03) / Fit to Video Aspect Ratio (H04) /
 * When Playback Starts (G-02) / Always on Top (H05, IA-WIN-03),
 * Hide Pointer While Playing (B30, G-06), and the Window settings
 * page. Menu sweep (B5): every command is in the command palette only
 * ("Category: Title", with its key); none has a menu path.
 * The window itself belongs to the core (`plugin.window`); this plugin
 * keeps the preferences in settings.json and applies them on activation. The
 * core undoes its effects on deactivation.
 */
export default async function activate(plugin: Plugin) {
  const { player, window } = plugin;
  const { text } = plugin.i18n;
  /**
   * Command categories (command palette, player 0.7.6): "Category: Title" must
   * read on its own once the menu path is gone. Text values, so `i18n extract`
   * finds them; "Window" and "Always on Top" reuse the page and submenu labels.
   */
  const category = {
    window: text("Window"),
    size: text("Window Size"),
    auto: text("Size on Open"),
    top: text("Always on Top"),
  };
  plugin.onError((error) => {
    // i18n-ignore: the player's (or the SDK's) own message
    quiet(plugin.osd.show(error.message));
  });

  const preferences = new Preferences(plugin);
  const video = new VideoSize(plugin);
  const onTop = new OnTop(plugin, preferences);
  const sizer = new Sizer(plugin, video);
  const pip = new PictureInPicture(plugin, video);
  const auto = new AutoSize(plugin, preferences, sizer, pip);
  video.onFirstSize = (source, size) => void auto.sized(source, size);

  const reason = () => sizeReason(plugin, video);

  // Picture-in-Picture (709, no default key) / Mini Size (710, `).
  // IA-WIN-01: ` is Mini Size on every system (id `mini`, so a user's rebinding
  // stays with it); Picture-in-Picture is Vestra's own extra command (id `pip`). The two
  // exclude each other: one is left before the other starts.
  plugin.commands.add({
    id: "mini",
    title: "Mini Size",
    category: category.window,
    order: 710,
    group: "screen",
    bindings: ["`"],
    checked: () => sizer.mini !== null,
    enabled: () => !window.state.fullscreen && (sizer.mini !== null || video.size !== null),
    busy: () => sizer.resizing || pip.switching,
    reason: () =>
      pip.switching
        ? "Switching picture-in-picture"
        : sizer.resizing
          ? "Resizing the window"
          : window.state.fullscreen
            ? "Not available in fullscreen"
            : "No video",
    run: async () => {
      if (!sizer.mini && pip.active) {
        await pip.toggle();
        if (pip.active) return; // could not leave picture-in-picture: its failure was shown
      }
      await sizer.toggleMini();
    },
  });
  // `supported` needs the main window, which may not exist yet at activation,
  // so Picture-in-Picture follows `window.onChange`: added while supported (macOS 14+).
  const pipCommand = () =>
    plugin.commands.add({
      id: "pip",
      title: "Picture-in-Picture",
      category: category.window,
      order: 709,
      group: "screen",
      checked: () => pip.active,
      enabled: () => pip.active || video.size !== null,
      busy: () => pip.switching || sizer.resizing,
      reason: () => (pip.switching ? "Switching picture-in-picture" : sizer.resizing ? "Resizing the window" : "No video"),
      run: async () => {
        // Mini Size in effect: back to the size before it first.
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
      title,
      category: category.size,
      order: 711 + index,
      group: "screen-size",
      radio: true,
      bindings: [String(index + 1)],
      checked: () => sizer.mini === null && !window.state.fullscreen && currentScale(window.state.videoArea, video.size) === scale,
      enabled: () => reason() === null,
      busy: () => sizer.resizing,
      reason: () => (sizer.resizing ? "Resizing the window" : reason()),
      run: () => sizer.scale(scale),
    }),
  );
  plugin.commands.add({
    id: "fit",
    title: "Fit to Video Aspect Ratio",
    category: category.size,
    order: 715,
    group: "screen-size",
    bindings: ["8"],
    enabled: () => reason() === null,
    busy: () => sizer.resizing,
    reason: () => (sizer.resizing ? "Resizing the window" : reason()),
    run: () => sizer.fit(),
  });

  // When Playback Starts ▸ (716–722, G-02).
  const setAuto = (choice: AutoChoice, say = true) => {
    preferences.update({ autoSize: choice });
    if (!say) return;
    if (choice === "off") quiet(plugin.osd.show("When playback starts: Don't resize the window"));
    else if (choice === "fit") quiet(plugin.osd.show("When playback starts: Fit to video aspect ratio"));
    else quiet(plugin.osd.show("When playback starts: {}×", [choice]));
  };
  const setAutoOnce = (on: boolean, say = true) => {
    preferences.update({ autoSizeOnce: on });
    if (say) quiet(plugin.osd.show(on ? "Resize only once when playback starts: On" : "Resize only once when playback starts: Off"));
  };
  AUTO_SIZES.forEach((choice, index) =>
    plugin.commands.add({
      id: `auto.${choice}`,
      title: AUTO_TITLES[index]!,
      category: category.auto,
      order: 716 + index,
      group: "screen-auto",
      radio: true,
      checked: () => preferences.value.autoSize === choice,
      run: () => setAuto(choice),
    }),
  );
  plugin.commands.add({
    id: "auto.once",
    title: "Only Once",
    category: category.auto,
    order: 722,
    group: "screen-auto-once",
    checked: () => preferences.value.autoSizeOnce,
    run: () => setAutoOnce(!preferences.value.autoSizeOnce),
  });

  // Always on Top ▸ (725–729).
  const setMode = (mode: Mode, say = true) => {
    preferences.update({ onTop: mode });
    void onTop.apply();
    if (say) quiet(plugin.osd.show(MODE_OSD[mode]));
  };
  plugin.commands.add({
    id: "top.cycle",
    title: "Cycle Mode",
    category: category.top,
    order: 725,
    group: "screen-top",
    bindings: ["Ctrl+T"],
    run: () => setMode(ON_TOP[(ON_TOP.indexOf(preferences.value.onTop) + 1) % ON_TOP.length]!),
  });
  ON_TOP.forEach((mode, index) =>
    plugin.commands.add({
      id: `top.${mode}`,
      title: MODE_TITLES[mode],
      category: category.top,
      order: 726 + index,
      group: "screen-top-mode",
      radio: true,
      checked: () => preferences.value.onTop === mode,
      run: () => setMode(mode),
    }),
  );

  // Hide Pointer While Playing (241, with playback's 240 / 242).
  const cursor = (on: boolean) => window.setCursorAutoHide(on ? "playing" : "fullscreen").catch(() => {});
  const setHideCursor = async (on: boolean, say = true) => {
    preferences.update({ hideCursorWhilePlaying: on });
    if (say) quiet(plugin.osd.show(on ? "Hide pointer while playing: On" : "Hide pointer while playing: Off"));
    await cursor(on);
  };
  plugin.commands.add({
    id: "hide-cursor",
    title: "Hide Pointer While Playing",
    category: category.window,
    order: 241,
    group: "playback-options",
    checked: () => preferences.value.hideCursorWhilePlaying,
    run: () => setHideCursor(!preferences.value.hideCursorWhilePlaying),
  });

  // Options ▸ Playback ▸ Window: the same preferences as the menu items.
  const page = plugin.settings.page("ui/page.slint", { id: PAGE_ID, category: "playback", title: "Window", order: 30 });
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
