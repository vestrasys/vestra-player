import { existsSync, readFileSync } from "node:fs";
import type { ImageRef, Panel, PlayerState, Plugin } from "@vestra-player/sdk";
import { CoverCache, fileName, type Track } from "./covers.ts";
import { imageProblem } from "./images.ts";
import { fileUrlPath, HOST_RULES, isUrl, type PathRules } from "./local-path.ts";
import { Logo, type LogoNotice } from "./logo.ts";
import { SettingsStore } from "./settings.ts";

/** What the backdrop draws: the idle logo, the audio cover, the loading screen, or nothing (plain Theme.bg-video). */
export type Mode = "idle" | "audio" | "loading" | "none";
/** What the video area holds, as this plugin judges it from `phase`, `kind` and `stats().video`. */
type Kind = "idle" | "audio" | "video" | "loading";

/** Longest source name on the loading screen (the Slint text elides as well). */
const LOADING_NAME = 60;
/**
 * G-16b: the name after "Opening: ": a file's name; a link's host and last
 * path segment (just the host when it has no path). Cut with "…".
 */
export function loadingName(source: string | null, rules: PathRules = HOST_RULES) {
  if (!source) return "";
  let name = fileName(source, rules);
  try {
    // A link shows its host; a path (even `C://x` on Windows) or a local `file:` URL does not.
    const url = isUrl(source, rules) && fileUrlPath(source, rules) === null ? new URL(source) : null;
    if (url?.hostname) name = name === url.hostname ? url.hostname : `${url.hostname}/${name}`;
  } catch {
    // Not a URL after all: the file name.
  }
  const chars = [...name];
  return chars.length > LOADING_NAME ? `${chars.slice(0, LOADING_NAME - 1).join("")}…` : name;
}

export const PAGE_ID = "logo-cover";

/** The notices of this plugin (a union of literals, so `i18n extract` finds them). */
type Notice =
  | LogoNotice
  | "Album cover for audio only: On"
  | "Album cover for audio only: Off"
  | "Logo when idle: On"
  | "Logo when idle: Off"
  | "Cover cache cleared";

/** "12.3 MB" for the page's cache line (units are not translated). */
function size(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * Album Cover and Idle Screen: one backdrop panel (R-U3) that the player shows when
 * there is no video frame. There is no `mode` from the player: the source
 * phase `empty` is the idle screen (H12, logo), `loading` is the loading
 * screen (G-16b: the player shows the backdrop while loading, G-16a), and a
 * ready source whose `stats().video` is null is audio only (J11, cover and
 * tags). A generated stream (`kind` `stream`, e.g. a file resolver's) has
 * pictures of its own that mpv does not see: always video. `error` is the
 * idle screen: the source that failed has nothing to show, and the one
 * before it is gone (never its card). Audio files with a cover picture are
 * shown by mpv as a video track, so the backdrop is not shown for them
 * (README, known behaviour).
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  // Headless hosts refuse the display; that never fails the action itself.
  const osd = (text: Notice) => void plugin.osd.show(text).catch(() => {});
  // i18n-ignore: the player's (or the SDK's) own message
  const report = (message: string) => void plugin.osd.show(message).catch(() => {});
  plugin.onError((error) => report(error.message));

  const settings = new SettingsStore(plugin);
  const logo = new Logo(plugin, settings, osd);
  let dataDirectory: string | null = null;
  try {
    dataDirectory = plugin.dataDirectory;
  } catch {
    dataDirectory = null;
  }
  const covers = dataDirectory ? new CoverCache(dataDirectory, plugin.tools) : null;
  covers?.load();

  // ------------------------------------------------------------------ scene
  let kind: Kind = "idle";
  let loading = "";
  /** The source the kind and track belong to (-1: none judged yet). */
  let judged = -1;
  let track: Track | null = null;
  let cover: ImageRef | null = null;
  let preparing: AbortController | null = null;

  const mode = (): Mode => {
    if (kind === "idle") return settings.value.idle.on ? "idle" : "none";
    if (kind === "audio") return settings.value.audio.on ? "audio" : "none";
    if (kind === "loading") return "loading";
    return "none";
  };

  /** Read tags and the cover of the current audio source (aborted when the source changes). */
  async function prepare(state: PlayerState) {
    preparing?.abort();
    cover = null;
    track = { title: fileName(state.source ?? ""), artist: "", album: "", cover: null };
    if (!covers || !state.source || !settings.value.audio.on) return;
    const controller = (preparing = new AbortController());
    try {
      const found = await covers.lookup(state.source, controller.signal);
      if (controller.signal.aborted || judged !== state.sourceId) return;
      track = found;
      cover = found.cover ? plugin.ui.image(found.cover) : null;
    } catch {
      // Aborted: a newer source took over.
    } finally {
      if (preparing === controller) preparing = null;
      refreshUsage();
      plugin.update();
    }
  }

  /**
   * Is the ready source audio only? A generated stream never is (its frames
   * do not go through mpv, so `stats().video` is null); otherwise
   * `stats().video`, with the backdrop's own state as the fallback.
   */
  async function judge(state: PlayerState) {
    let audio = false;
    if (state.kind !== "stream")
      try {
        audio = (await player.stats()).video === null;
      } catch {
        audio = plugin.panels.backdrop.state?.effective === true;
      }
    // A newer source was judged meanwhile.
    if (judged !== state.sourceId) return;
    kind = audio ? "audio" : "video";
    if (audio) await prepare(state);
    plugin.update();
  }

  function observe(state: PlayerState) {
    // error: the idle screen, never the card of the source before (G-16b).
    if (state.phase === "empty" || state.phase === "error") {
      preparing?.abort();
      kind = "idle";
      judged = -1;
      track = null;
      cover = null;
      return;
    }
    if (state.sourceId !== judged && preparing) preparing.abort();
    if (state.phase === "loading") {
      kind = "loading";
      loading = loadingName(state.source);
      // Judged again once ready (a reload of the same source too).
      judged = -1;
      return;
    }
    if ((state.phase === "ready" || state.phase === "ended") && state.sourceId !== judged) {
      judged = state.sourceId;
      void judge(state);
    }
  }

  // ------------------------------------------------------------------ backdrop
  const view = plugin.scope();
  let mounted = false;
  const backdrop: Panel = view.panels.mount("backdrop", "ui/backdrop.slint");
  backdrop.bind(() => ({
    mode: mode(),
    title: track?.title ?? "",
    artist: track?.artist ?? "",
    album: track?.album ?? "",
    loading,
    // Images only after mounting: a bad picture must never fail the mount itself.
    customLogo: mounted && logo.image !== null,
    hasCover: mounted && cover !== null,
    ...(mounted && logo.image ? { logo: logo.image } : {}),
    ...(mounted && cover ? { cover } : {}),
  }));
  void backdrop.ready.then(
    () => {
      mounted = true;
      plugin.update();
    },
    () => {},
  );
  /** The player could not show a picture: check both again, drop what is broken. */
  view.onError((error) => {
    if (error.code !== "invalid" && error.code !== "missing") return report(error.message);
    const broken = (path: string) => !existsSync(path) || imageProblem(readFileSync(path)) !== null;
    if (logo.image && broken(logo.image.path)) logo.fallBack();
    if (cover && broken(cover.path)) cover = null;
    plugin.update();
  });
  // The backdrop came back while a ready source counts as video: look again
  // (stats() may have been read before the video track was known).
  plugin.panels.backdrop.onChange((state) => {
    const now = player.state;
    if (state?.effective && kind === "video" && (now.phase === "ready" || now.phase === "ended")) void judge(now);
  });

  // ------------------------------------------------------------------ actions
  function setAudio(on: boolean) {
    if (on === settings.value.audio.on) return;
    settings.update((s) => void (s.audio.on = on));
    osd(on ? "Album cover for audio only: On" : "Album cover for audio only: Off");
    // Turned on while an audio file plays: read its cover now.
    if (on && kind === "audio" && !cover && !preparing) void prepare(player.state);
  }
  function setIdle(on: boolean) {
    if (on === settings.value.idle.on) return;
    settings.update((s) => void (s.idle.on = on));
    osd(on ? "Logo when idle: On" : "Logo when idle: Off");
  }
  let choosing = false;
  async function chooseLogo() {
    if (choosing) return;
    choosing = true;
    try {
      await logo.choose();
    } finally {
      choosing = false;
    }
  }

  /**
   * Command palette categories ("Category: Title", player 0.7.6): the menu each command sat in
   * (menu sweep B5: all four are in the command palette only, with no menu path).
   * Text values, so `i18n extract` finds them; "Appearance" reuses the former submenu's translation,
   * "Audio" and "Reset" the player's words for those menus.
   */
  const category = { audio: plugin.i18n.text("Audio"), appearance: plugin.i18n.text("Appearance"), reset: plugin.i18n.text("Reset") };
  plugin.commands.add({
    id: "audio-cover",
    category: category.audio,
    title: "Show Album Cover for Audio Only",
    order: 530,
    group: "visual",
    checked: () => settings.value.audio.on,
    run: () => setAudio(!settings.value.audio.on),
  });
  plugin.commands.add({
    id: "idle-logo",
    category: category.appearance,
    title: "Show Logo When Idle",
    order: 845,
    group: "settings-logo",
    checked: () => settings.value.idle.on,
    run: () => setIdle(!settings.value.idle.on),
  });
  plugin.commands.add({
    id: "choose-logo",
    category: category.appearance,
    title: "Choose Logo Image…",
    order: 846,
    group: "settings-logo",
    busy: () => choosing,
    run: chooseLogo,
  });
  plugin.commands.add({
    id: "reset-logo",
    category: category.reset,
    title: "Idle Logo",
    order: 850,
    group: "reset-appearance",
    enabled: () => logo.custom,
    reason: () => (logo.custom ? null : "Using the default logo"),
    run: () => logo.reset(),
  });

  // ------------------------------------------------------------------ settings page
  const page = plugin.settings.page("ui/page.slint", {
    id: PAGE_ID,
    category: "appearance",
    title: "Logo and Album Cover",
    order: 20,
  });
  let pageReady = false;
  let usage = covers?.usage ?? { entries: 0, bytes: 0 };
  function refreshUsage() {
    if (covers) usage = covers.usage;
  }
  page.bind(() => ({
    idleOn: settings.value.idle.on,
    audioOn: settings.value.audio.on,
    customLogo: pageReady && logo.image !== null,
    ...(pageReady && logo.image ? { logo: logo.image } : {}),
    // The sentence is in ui/page.slint; this side passes the numbers.
    cacheAvailable: covers !== null,
    cacheEntries: usage.entries,
    cacheSize: size(usage.bytes),
    canClear: usage.entries > 0,
  }));
  void page.ready.then(
    () => {
      pageReady = true;
      plugin.update();
    },
    () => {},
  );
  page.on("setIdle", (on: boolean) => {
    setIdle(on);
    page.set({ idleOn: settings.value.idle.on });
  });
  page.on("setAudio", (on: boolean) => {
    setAudio(on);
    page.set({ audioOn: settings.value.audio.on });
  });
  page.on("chooseLogo", chooseLogo);
  page.on("resetLogo", () => logo.reset());
  page.on("clearCache", async () => {
    if (!covers) return;
    await covers.clear();
    refreshUsage();
    osd("Cover cache cleared");
  });
  player.onChange(observe);
  observe(player.state);
}
