import { readdir } from "node:fs/promises";
import { basename, dirname, isAbsolute, join } from "node:path";
import type { AudioDevice, AudioTrack, Plugin, PlayerState, PluginError, Scope } from "@vestra-player/sdk";
import {
  DEVICE_POLL_MS,
  SELECTED_DEVICE,
  SYNC_STEP_MS,
  TABS,
  channelModes,
  cleanName,
  clip,
  companions,
  delay,
  deviceTitles,
  matches,
  neutral,
  neutralClarity,
  neutralProcessing,
  nodes,
  parse,
  presetOf,
  presets,
  serialize,
  syncKeeps,
  syncSeconds,
  tenth,
  trackRow,
} from "./model.ts";
import type { ChannelMode, Settings, SyncKeep, TrackRow } from "./model.ts";

/** The player's Audio menu. */
const AUDIO = "@audio";

/** Template arguments that are data (file names, device names, errors) are cut so the message fits the OSD's 200 characters. */
const arg = (text: string) => clip(text, 120);
/** Best-effort feedback: a missing OSD (headless) must not look like a processing failure. */
const quiet = (shown: Promise<void>) => void shown.catch(() => undefined);

/**
 * Audio workbench: one window and one owned audio chain, plus the track menu.
 * The equalizer, presets, normalize, crystalizer and channels are global and
 * saved in `settings.json`; only the audio sync follows the source, as its keep mode says.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const { text } = plugin.i18n;
  /**
   * A track in the menu and the OSD: its data ("1. 日语 [jpn] (aac)"; `{}` keeps it untranslated),
   * or "Track 2 (opus)" without title and language; an external one gets "(external)".
   */
  const trackLabel = (row: TrackRow) => {
    const label = row.text ? text("{}", [row.text]) : row.codec ? text("Track {} ({})", [row.n, row.codec]) : text("Track {}", [row.n]);
    return row.external ? text("{} (external)", [label]) : label;
  };
  /**
   * The same track in the window's Track list (a text value the player translates): its data,
   * or "Track 2 (opus)"; an external one says so in one sentence ("Track 2 (opus) (external)").
   */
  const trackItem = (row: TrackRow) => {
    if (row.text) return row.external ? text("{} (external)", [row.text]) : text("{}", [row.text]);
    if (row.codec) return row.external ? text("Track {} ({}) (external)", [row.n, row.codec]) : text("Track {} ({})", [row.n, row.codec]);
    return row.external ? text("Track {} (external)", [row.n]) : text("Track {}", [row.n]);
  };
  /**
   * Command palette categories ("Category: Title", player 0.7.6): one per submenu, "Audio" for
   * the top-level items, "Reset" for the Reset menu's. Text values, so `i18n extract` finds them;
   * the submenu ones reuse the menu labels and so their translations.
   */
  const category = {
    audio: text("Audio"),
    track: text("Audio Track"),
    channels: text("Channels"),
    device: text("Output Device"),
    preset: text("Equalizer Preset"),
    sync: text("Audio Sync"),
    processing: text("Audio Processing"),
    reset: text("Reset"),
  };
  /** Where a command's control also lives: the Audio Workbench window (palette, player 0.7.6). */
  const HOME = "audio.workbench";
  const filter = plugin.audio.filter("workbench");
  const workbenchWindow = plugin.windows.create("ui/workbench.slint", { id: "audio-workbench" });
  const store = settingsStore(plugin);
  const stored = parse(store.value);
  let settings = stored.settings;
  let syncKeep: SyncKeep = stored.syncKeep;
  let autoLoad = stored.autoLoad;
  let tab = stored.tab;
  let device = stored.device;
  let deviceName = stored.deviceName;
  let source: number | null = null;
  let track = "";
  // A change was requested, or saved settings are in effect: "Applied" then means something.
  let touched = nodes(settings).length > 0 || settings.delayMs !== 0;
  /** Writes after 300 ms; the SDK writes what is pending when the plugin stops. */
  const persist = () => store.save(serialize({ settings, syncKeep, autoLoad, tab, device, deviceName, extra: stored.extra }));

  // Channel count of the selected track (`stats()`), read when the source or the track changes.
  let channelCount: number | null = null;
  const mono = () => channelCount === 1;

  const ready = () => {
    const { phase, audioTracks } = player.state;
    return phase === "ready" && (audioTracks === null || audioTracks.some((t) => t.selected));
  };
  const busy = () => filter.status.state === "applying";
  const apply = () => filter.set({ nodes: nodes(settings, { mono: mono() }), delayMs: settings.delayMs });
  /** Menu and window edits are ignored without a selected track, like the disabled controls. */
  const edit = (next: Settings) => {
    if (!ready()) return false;
    settings = next;
    touched = true;
    apply();
    persist();
    return true;
  };
  /** Choosing a preset (like dragging a band) turns the equalizer on. */
  const preset = (index: number) => {
    const chosen = presets[index];
    if (chosen) edit({ ...settings, equalizer: true, gains: [...chosen.gains] });
    return chosen;
  };
  const syncBy = (steps: number) => {
    if (edit({ ...settings, delayMs: delay(settings.delayMs + steps * SYNC_STEP_MS) }))
      quiet(plugin.osd.show("Audio sync: {} s", [syncSeconds(settings.delayMs)]));
  };
  const setKeep = (keep: SyncKeep) => {
    syncKeep = keep;
    persist();
  };
  const setChannels = (mode: ChannelMode) => {
    if (mono() || !edit({ ...settings, channels: mode })) return;
    const chosen = channelModes.find((m) => m.id === mode)!;
    quiet(plugin.osd.show(mode !== "stereo" && channelCount !== null && channelCount > 2 ? chosen.mixed : chosen.osd));
  };

  let probe = 0;
  const readChannels = async () => {
    const ticket = ++probe;
    const stats = await player.stats().catch(() => null);
    if (ticket !== probe) return;
    const count = stats?.audio?.channels ?? null;
    if (count === channelCount) return;
    channelCount = count;
    apply();
    plugin.update();
  };
  let reread: ReturnType<typeof setTimeout> | undefined;
  plugin.onDispose(() => clearTimeout(reread));

  // A new source resets only the delay, and only under Don't Keep. A track switch keeps everything;
  // re-setting the same value also drops a failure the previous track left behind.
  let probed = "";
  const follow = (state: PlayerState) => {
    const current = state.audioTracks?.filter((t) => t.selected).map((t) => t.id).join(",") ?? "?";
    if (state.sourceId !== source || current !== track) {
      const newSource = source !== null && state.sourceId !== source;
      source = state.sourceId;
      track = current;
      if (newSource && syncKeep === "none" && settings.delayMs !== 0) settings = { ...settings, delayMs: 0 };
      channelCount = null;
      apply(); // a never-applied neutral chain sends nothing
    }
    // Read the channel count once the track plays. The decoder may still report the
    // previous track at first, so read again a second later.
    const key = `${source}|${track}`;
    if (state.phase !== "ready" || probed === key) return;
    probed = key;
    void readChannels();
    clearTimeout(reread);
    reread = setTimeout(() => void readChannels(), 1000);
    reread.unref?.();
  };
  player.onChange(follow);
  follow(player.state);

  // ---------------------------------------------------------------- A11 external audio
  let loadedFor = { source: -1, paths: new Set<string>() };
  /** Paths this plugin loaded into the source `id` (reset when the source changes). */
  const loaded = (id: number) => {
    if (loadedFor.source !== id) loadedFor = { source: id, paths: new Set() };
    return loadedFor.paths;
  };
  let loading = false;
  const loadReason = (): "Loading" | "No file is playing" | "Generated video cannot take external audio" | null => {
    const { phase, kind } = player.state;
    if (phase === "loading") return "Loading";
    if (phase !== "ready") return "No file is playing";
    return kind === "stream" ? "Generated video cannot take external audio" : null;
  };
  const loadExternal = async () => {
    if (loading || loadReason()) return;
    loading = true;
    plugin.update();
    try {
      const seen = player.state;
      const path = await plugin.dialogs.openFile({ title: "Load External Audio" });
      if (!path) return;
      const paths = loaded(seen.sourceId);
      if (paths.has(path)) return quiet(plugin.osd.show("This audio is already loaded"));
      try {
        await player.addAudioTrack(path, { ifCurrent: seen });
        paths.add(path);
        quiet(plugin.osd.show("Audio loaded: {}", [arg(basename(path))]));
      } catch (error) {
        if ((error as PluginError).code === "stale") quiet(plugin.osd.show("The source changed; the audio was not loaded"));
        else quiet(plugin.osd.show("Cannot load audio: {}", [arg((error as Error).message)]));
      }
    } finally {
      loading = false;
    }
  };
  /** Same-named audio next to a local file, added without selecting it; failures are silent. */
  const scanned = new Set<number>();
  const autoLoadFor = async (state: PlayerState) => {
    const path = state.source;
    if (!autoLoad || state.phase !== "ready" || state.kind !== "media" || !path || scanned.has(state.sourceId)) return;
    if (path.includes("://") || !isAbsolute(path)) return;
    scanned.add(state.sourceId);
    let names: string[];
    try {
      names = await readdir(dirname(path));
    } catch {
      return;
    }
    const paths = loaded(state.sourceId);
    for (const name of companions(basename(path), names)) {
      const file = join(dirname(path), name);
      if (paths.has(file)) continue;
      if (player.state.sourceId !== state.sourceId) return;
      try {
        await player.addAudioTrack(file, { select: false });
        paths.add(file);
      } catch (error) {
        if ((error as PluginError).code === "stale") return;
      }
    }
  };
  player.onChange((state) => void autoLoadFor(state));
  void autoLoadFor(player.state);
  const setAutoLoad = (on: boolean) => {
    autoLoad = on;
    persist();
    if (on) void autoLoadFor(player.state);
  };

  // ---------------------------------------------------------------- G03 / G04 tracks
  const tracks = () => player.state.audioTracks ?? [];
  const loadingTracks = () => player.state.audioTracks === null && player.state.phase === "loading";
  const noTracks = (): "Loading" | "No audio tracks" => (loadingTracks() ? "Loading" : "No audio tracks");
  const selectTrack = async (id: number) => {
    const list = tracks();
    const index = list.findIndex((t) => t.id === id);
    const chosen = list[index];
    if (!chosen || chosen.selected) return;
    try {
      await player.selectAudioTrack(id);
      quiet(plugin.osd.show("Audio track: {}", [trackLabel(trackRow(chosen, index))]));
    } catch (error) {
      // The source changed meanwhile: the menu follows it, nothing to say.
      if ((error as PluginError).code !== "stale") quiet(plugin.osd.show("Cannot switch the audio track: {}", [arg((error as Error).message)]));
    }
  };
  const cycleTrack = () => {
    const list = tracks();
    if (list.length === 1) return quiet(plugin.osd.show("Only one audio track"));
    const next = list[(list.findIndex((t) => t.selected) + 1) % list.length];
    return next ? selectTrack(next.id) : undefined;
  };

  // The track items are rebuilt when the list changes (one child scope); the rest of the
  // Audio Track ▸ submenu sits after them at order 512.
  let trackScope: Scope = plugin.scope();
  let trackKey: string | null = null;
  let rebuilding = Promise.resolve();
  const rebuildTracks = () =>
    (rebuilding = rebuilding.then(async () => {
      const list = player.state.audioTracks;
      const labels = list?.map((t, index) => trackLabel(trackRow(t, index))) ?? null;
      const key = JSON.stringify([labels, list?.map((t) => t.id), list === null ? noTracks() : null]);
      if (key === trackKey) return;
      trackKey = key;
      await trackScope.dispose();
      trackScope = plugin.scope();
      if (!list || list.length === 0) {
        trackScope.commands.add({
          id: "audio.track.none",
          path: [AUDIO, "Audio Track", loadingTracks() ? "Loading…" : "No Audio Tracks"],
          category: category.track,
          order: 511,
          group: "audio-tools",
          enabled: false,
          run: () => undefined,
        });
        return;
      }
      list.forEach((t: AudioTrack, index) =>
        trackScope.commands.add({
          id: `audio.track.${t.id}`,
          path: [AUDIO, "Audio Track", labels![index]!],
          category: category.track,
          home: HOME,
          order: 511,
          group: "audio-tools",
          radio: true,
          checked: () => tracks().some((x) => x.id === t.id && x.selected),
          run: () => selectTrack(t.id),
        }),
      );
    }));
  player.onChange(() => void rebuildTracks());
  void rebuildTracks();

  // ---------------------------------------------------------------- G16 output device
  // The device list is read at start and every 5 s (the player pushes no list changes);
  // the choice is saved here and handed to the player at start (it keeps it for one run).
  let devices: AudioDevice[] | null = null; // null: the player lists none (headless, ao=null)
  /** The chosen device is not connected: its return gets an OSD, its loss already had one or none. */
  let lost = false;
  let switching = 0;
  let output = player.state.audioOutput ?? { requested: null, active: null };
  const present = (id: string | null) => id === null || !!devices?.some((d) => d.id === id);
  /** A device's name; `null` for an unknown one (shown as "Selected device"). */
  const nameOf = (id: string) => devices?.find((d) => d.id === id)?.name ?? (id === device ? deviceName : null);
  /**
   * The menu and window entries: System Default, the devices, and the chosen one while it is
   * not connected. `name` is `null` for System Default and for an unknown device.
   */
  const deviceEntries = () => {
    if (!devices?.length) return null;
    const missing = device !== null && !present(device) ? nameOf(device) ?? SELECTED_DEVICE : null;
    // A device name is data (`{}` keeps it untranslated); "Unnamed device" and "Selected device" are words, also with " (2)".
    const titles = deviceTitles(devices, missing).map(({ name, n }) => {
      const base = !name ? text("Unnamed device") : name === SELECTED_DEVICE ? text("Selected device") : name;
      return n === null ? (typeof base === "string" ? text("{}", [base]) : base) : text("{} ({})", [base, n]);
    });
    return [
      { id: null, title: text("System Default"), name: null, connected: true },
      ...devices.map((d, i) => ({ id: d.id as string | null, title: titles[i]!, name: d.name as string | null, connected: true })),
      ...(missing === null ? [] : [{ id: device, title: titles.at(-1)!, name: device === null ? null : nameOf(device), connected: false }]),
    ];
  };
  const readDevices = async () => {
    let list: AudioDevice[] | null;
    try {
      list = await player.audioDevices();
    } catch {
      list = null;
    }
    devices = list && list.length > 0 ? list : null;
    // Keep the saved name current, so a disconnected device still shows what it is called.
    const found = device === null ? undefined : devices?.find((d) => d.id === device);
    if (found && found.name !== deviceName) {
      deviceName = found.name;
      persist();
    }
    if (device !== null && !present(device) && output.active !== device) lost = true;
    void rebuildDevices();
    plugin.update();
  };
  const chooseDevice = async (id: string | null) => {
    if (switching || (id === device && output.requested === id)) return;
    if (id !== null && !present(id)) return; // a disconnected entry is disabled
    const name = id === null ? null : nameOf(id);
    switching++;
    try {
      await player.setAudioDevice(id);
      device = id;
      deviceName = name;
      lost = false;
      persist();
      announce(id);
    } catch (error) {
      const message = arg((error as Error).message);
      if (name === null) quiet(plugin.osd.show("Cannot switch to System Default: {}", [message]));
      else quiet(plugin.osd.show("Cannot switch to {}: {}", [arg(name), message]));
    } finally {
      switching--;
      output = player.state.audioOutput ?? output;
      void rebuildDevices();
      plugin.update();
    }
  };
  /** "Audio output: …" for the device `id` (`null`: System Default). */
  const announce = (id: string | null) => {
    const name = id === null ? null : nameOf(id);
    if (id === null) quiet(plugin.osd.show("Audio output: System Default"));
    else if (name === null) quiet(plugin.osd.show("Audio output: Selected device"));
    else quiet(plugin.osd.show("Audio output: {}", [arg(name)]));
  };
  /** Unplugged: the player falls back to the default; plugged back: it returns to the choice. */
  const followOutput = (state: PlayerState) => {
    const next = state.audioOutput ?? { requested: null, active: null };
    const previous = output;
    output = next;
    if (switching || next.requested === null || next.requested !== previous.requested || next.active === previous.active) return;
    if (previous.active === next.requested) {
      lost = true;
      quiet(plugin.osd.show("Output device disconnected, using System Default"));
      void readDevices();
    } else if (next.active === next.requested && lost) {
      lost = false;
      const requested = next.requested;
      void readDevices().then(() => announce(requested));
    }
  };
  player.onChange(followOutput);

  let deviceScope: Scope = plugin.scope();
  let deviceKey: string | null = null;
  let deviceRebuild = Promise.resolve();
  const rebuildDevices = () =>
    (deviceRebuild = deviceRebuild.then(async () => {
      const entries = deviceEntries();
      const key = JSON.stringify(entries?.map((e) => [e.id, e.title, e.connected]) ?? null);
      if (key === deviceKey) return;
      deviceKey = key;
      await deviceScope.dispose();
      deviceScope = plugin.scope();
      const at = { order: 512, group: "audio-tools" } as const;
      if (!entries) {
        deviceScope.commands.add({ id: "audio.device.none", path: [AUDIO, "Output Device", "No Output Devices"], category: category.device, ...at, enabled: false, run: () => undefined });
        return;
      }
      entries.forEach((entry, index) =>
        deviceScope.commands.add({
          id: `audio.device.${index}`,
          path: [AUDIO, "Output Device", entry.title],
          category: category.device,
          home: HOME,
          ...at,
          radio: true,
          enabled: entry.connected,
          reason: entry.connected ? null : "Device not connected",
          busy: () => switching > 0,
          checked: () => device === entry.id,
          run: () => chooseDevice(entry.id),
        }),
      );
    }));
  // Hand the saved choice to the player first (it may not be plugged in: no OSD then);
  // activation does not wait for it.
  void (async () => {
    if (device !== null) {
      switching++;
      await player.setAudioDevice(device).catch(() => undefined);
      switching--;
      output = player.state.audioOutput ?? output;
    }
    await readDevices();
  })();
  const devicePoll = setInterval(() => void readDevices(), DEVICE_POLL_MS);
  devicePoll.unref?.();
  plugin.onDispose(() => clearInterval(devicePoll));

  // ---------------------------------------------------------------- window
  workbenchWindow.bind(() => {
    const list = tracks();
    return {
      status: describe(),
      sourceReady: ready(),
      isBusy: busy(),
      hasError: filter.status.state === "failed",
      errorDetail: filter.status.state === "failed" ? filter.status.error.message : "",
      tabIndex: tab,
      equalizerEnabled: settings.equalizer,
      presetIndex: presetOf(settings),
      gainValues: settings.gains,
      delayMs: settings.delayMs,
      normalizeEnabled: settings.normalize,
      clarityEnabled: settings.clarity.on,
      clarityDb: settings.clarity.db,
      syncKeepIndex: syncKeeps.findIndex((k) => k.id === syncKeep),
      trackRows: list.length > 0 ? list.map((t, index) => trackItem(trackRow(t, index))) : [loadingTracks() ? text("Loading") : text("No audio tracks")],
      trackIndex: list.length > 0 ? list.findIndex((t) => t.selected) : 0,
      tracksAvailable: list.length > 0,
      canLoad: !loading && loadReason() === null,
      channelIndex: channelModes.findIndex((m) => m.id === settings.channels),
      channelsAvailable: ready() && !mono(),
      monoTrack: mono(),
      autoLoad,
      ...deviceProps(),
    };
  });
  workbenchWindow.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    persist();
  });
  workbenchWindow.on("gainChanged", (index: number, value: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= settings.gains.length) return;
    const gains = [...settings.gains];
    gains[index] = tenth(value, -6, 6);
    edit({ ...settings, equalizer: true, gains });
  });
  workbenchWindow.on("equalizerChanged", (on: boolean) => edit({ ...settings, equalizer: on === true }));
  workbenchWindow.on("delayChanged", (value: number) => edit({ ...settings, delayMs: delay(value) }));
  workbenchWindow.on("delayReset", () => edit({ ...settings, delayMs: 0 }));
  workbenchWindow.on("normalizeChanged", (on: boolean) => edit({ ...settings, normalize: on === true }));
  workbenchWindow.on("clarityToggled", (on: boolean) => edit({ ...settings, clarity: { ...settings.clarity, on: on === true } }));
  workbenchWindow.on("clarityChanged", (value: number) => {
    if (settings.clarity.on) edit({ ...settings, clarity: { on: true, db: tenth(value, 0, 6) } });
  });
  workbenchWindow.on("clarityReset", () => edit({ ...settings, clarity: neutralClarity() }));
  workbenchWindow.on("syncKeepSelected", (index: number) => {
    const keep = Number.isInteger(index) ? syncKeeps[index] : undefined;
    if (keep) setKeep(keep.id);
  });
  workbenchWindow.on("presetSelected", (index: number) => preset(index));
  workbenchWindow.on("resetRequested", () => edit(neutral()));
  workbenchWindow.on("retryRequested", () => {
    if (ready() && !busy()) filter.retry();
  });
  workbenchWindow.on("trackSelected", (index: number) => {
    const chosen = Number.isInteger(index) ? tracks()[index] : undefined;
    if (chosen) void selectTrack(chosen.id);
  });
  workbenchWindow.on("loadRequested", () => loadExternal());
  workbenchWindow.on("channelSelected", (index: number) => {
    const mode = Number.isInteger(index) ? channelModes[index] : undefined;
    // The window gives no on-screen text; only the menu does.
    if (mode && !mono()) edit({ ...settings, channels: mode.id });
  });
  workbenchWindow.on("autoLoadChanged", (on: boolean) => setAutoLoad(on === true));
  workbenchWindow.on("deviceSelected", (index: number) => {
    const entry = Number.isInteger(index) ? deviceEntries()?.[index] : undefined;
    if (entry?.connected) void chooseDevice(entry.id);
  });
  /** Output device drop-down: names cut with an ellipsis by the control, the full one on hover. */
  function deviceProps() {
    const entries = deviceEntries();
    if (!entries) {
      const none = text("No output devices available");
      return { deviceRows: [none], deviceIndex: 0, devicesAvailable: false, deviceTip: none };
    }
    const index = Math.max(0, entries.findIndex((e) => e.id === device));
    const chosen = entries[index]!;
    /** A device's name as text (`name` cleans it for the list or keeps it whole for the tip). */
    const label = (entry: (typeof entries)[number], name: (text: string) => string) => {
      if (entry.id === null) return text("System Default");
      if (entry.name === null) return text("Selected device");
      const shown = name(entry.name);
      return shown ? text("{}", [shown]) : text("Unnamed device");
    };
    return {
      deviceRows: entries.map((e) => (e.connected ? label(e, cleanName) : text("{} (not connected)", [label(e, cleanName)]))),
      deviceIndex: index,
      devicesAvailable: switching === 0,
      // The tip has the full name, and why a disconnected device is not used.
      deviceTip: chosen.connected
        ? label(chosen, (name) => name)
        : text("{}: device not connected, using System Default", [label(chosen, (name) => name)]),
    };
  }

  // ---------------------------------------------------------------- menu Open ▸ 114, Audio ▸ 510–529
  plugin.commands.add({
    id: "audio.load",
    path: ["@open", "Load External Audio…"],
    category: category.audio,
    home: HOME,
    order: 114,
    group: "open",
    enabled: () => loadReason() === null,
    busy: () => loading,
    reason: loadReason,
    run: loadExternal,
  });
  const reason = (): "No audio track selected" | null => (ready() ? null : "No audio track selected");
  const editing = { enabled: ready, busy, reason };
  // Repeating keys must not be swallowed while a change is applying (the filter keeps the last value).
  const repeating = { enabled: ready, reason, repeat: true };
  plugin.commands.add({
    id: "audio.workbench",
    path: [AUDIO, "Audio Workbench…"],
    category: category.audio,
    order: 510,
    group: "audio-tools",
    run: () => workbenchWindow.show(),
  });
  plugin.commands.add({
    id: "audio.track.cycle",
    path: [AUDIO, "Audio Track", "Cycle Tracks"],
    category: category.track,
    home: HOME,
    order: 512,
    group: "audio-track-cycle",
    bindings: ["Alt+A"],
    enabled: () => tracks().length > 0,
    reason: () => (tracks().length > 0 ? null : noTracks()),
    run: cycleTrack,
  });
  for (const { id, label } of channelModes)
    plugin.commands.add({
      id: `audio.channels.${id}`,
      path: [AUDIO, "Audio Track", label],
      category: category.channels,
      home: HOME,
      order: 512,
      group: "audio-channels",
      radio: true,
      enabled: () => ready() && !mono(),
      busy,
      reason: () => reason() ?? (mono() ? "The current track is mono" : null),
      checked: () => settings.channels === id,
      run: () => setChannels(id),
    });
  plugin.commands.add({
    id: "audio.equalizer",
    path: [AUDIO, "Equalizer"],
    category: category.audio,
    home: HOME,
    order: 513,
    group: "audio-eq",
    bindings: ["Shift+E"],
    ...editing,
    checked: () => settings.equalizer,
    run: () => {
      if (edit({ ...settings, equalizer: !settings.equalizer })) quiet(plugin.osd.show(settings.equalizer ? "Equalizer: On" : "Equalizer: Off"));
    },
  });
  presets.forEach(({ label, osd }, index) =>
    plugin.commands.add({
      id: `audio.preset.${index}`,
      path: [AUDIO, "Preset", label],
      category: category.preset,
      home: HOME,
      order: 514,
      group: "audio-eq",
      radio: true,
      ...editing,
      checked: () => matches(settings, index),
      run: () => {
        if (ready() && preset(index)) quiet(plugin.osd.show(osd));
      },
    }),
  );
  // The step is SYNC_STEP_MS (50 ms).
  const syncSteps = [
    { id: "audio.sync.earlier", title: "50 ms Earlier", binding: ">", run: () => syncBy(-1) },
    { id: "audio.sync.later", title: "50 ms Later", binding: "<", run: () => syncBy(1) },
  ] as const;
  for (const { id, title, binding, run } of syncSteps)
    plugin.commands.add({ id, path: [AUDIO, "Audio Sync", title], category: category.sync, home: HOME, order: 515, group: "audio-eq", bindings: [binding], ...repeating, run });
  plugin.commands.add({
    id: "audio.sync.reset",
    path: ["@reset", "Audio Sync"],
    category: category.reset,
    home: HOME,
    order: 530,
    group: "reset-audio",
    bindings: ["?"],
    ...repeating,
    run: () => {
      if (edit({ ...settings, delayMs: 0 })) quiet(plugin.osd.show("Audio sync: {} s", [syncSeconds(0)]));
    },
  });
  syncKeeps.forEach(({ id, label }) =>
    plugin.commands.add({
      id: `audio.sync.keep.${id}`,
      path: [AUDIO, "Audio Sync", label],
      category: category.sync,
      home: HOME,
      order: 515,
      group: "audio-sync-keep",
      radio: true,
      checked: () => syncKeep === id,
      run: () => setKeep(id),
    }),
  );
  plugin.commands.add({
    id: "audio.normalize",
    path: [AUDIO, "Processing", "Normalize"],
    category: category.processing,
    home: HOME,
    order: 516,
    group: "audio-eq",
    bindings: ["Shift+N"],
    ...editing,
    checked: () => settings.normalize,
    run: () => {
      if (edit({ ...settings, normalize: !settings.normalize })) quiet(plugin.osd.show(settings.normalize ? "Normalize: On" : "Normalize: Off"));
    },
  });
  plugin.commands.add({
    id: "audio.clarity",
    path: [AUDIO, "Processing", "Crystalizer"],
    category: category.processing,
    home: HOME,
    order: 516,
    group: "audio-eq",
    bindings: ["Shift+C"],
    ...editing,
    checked: () => settings.clarity.on,
    run: () => {
      if (edit({ ...settings, clarity: { ...settings.clarity, on: !settings.clarity.on } }))
        quiet(plugin.osd.show(settings.clarity.on ? "Crystalizer: On" : "Crystalizer: Off"));
    },
  });
  plugin.commands.add({
    id: "audio.reset",
    path: ["@reset", "Audio Processing"],
    category: category.reset,
    order: 531,
    group: "reset-audio",
    ...editing,
    // The processing only: Audio Sync has its own item, channels are a radio choice.
    run: () => {
      if (edit(neutralProcessing(settings))) quiet(plugin.osd.show("Audio processing reset"));
    },
  });

  /** The window's status line, as a state its Slint file puts into words. */
  function describe() {
    if (!ready()) return player.state.phase === "loading" ? "loading" : "no-track";
    const status = filter.status;
    if (status.state === "applying") return "applying";
    if (status.state === "failed") return "failed";
    return touched ? (status.state === "applied" ? "applied" : "applying") : "ready";
  }
}

/** `settings.json` through `plugin.storage`; without a data directory, settings last for this run. */
function settingsStore(plugin: Plugin) {
  try {
    return plugin.storage.json<Record<string, unknown>>("settings", {
      defaults: {},
      validate: (value): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value),
    });
  } catch {
    let value: Record<string, unknown> = {};
    return {
      get value() {
        return value;
      },
      save(next: Record<string, unknown>) {
        value = next;
      },
    };
  }
}
