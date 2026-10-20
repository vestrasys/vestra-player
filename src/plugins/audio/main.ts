import { readdir } from "node:fs/promises";
import { basename, dirname, isAbsolute, join } from "node:path";
import type { AudioDevice, AudioTrack, Plugin, PlayerState, PluginError, Scope } from "../../sdk/plugin/index.ts";
import {
  DEVICE_POLL_MS,
  NO_DEVICES,
  SYNC_STEP_MS,
  SYSTEM_DEFAULT,
  TABS,
  channelModes,
  companions,
  delay,
  deviceLabel,
  deviceTitles,
  matches,
  neutral,
  neutralClarity,
  nodes,
  parse,
  presetOf,
  presets,
  serialize,
  syncKeeps,
  syncText,
  tenth,
  trackLabel,
} from "./model.ts";
import type { ChannelMode, Settings, SyncKeep } from "./model.ts";

/**
 * Audio workbench: one window and one owned audio chain, plus the track menu.
 * The equalizer, presets, 规格化, 晶化 and 声道 are global and saved in
 * `settings.json`; only 声音同步 follows the source, as its keep mode says.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const filter = plugin.audio.filter("workbench");
  const window = plugin.windows.create("ui/workbench.slint", { id: "audio-workbench" });
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
  // A change was requested, or saved settings are in effect: "已应用" then means something.
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
  /** Best-effort feedback: a missing OSD (headless) must not look like a processing failure. */
  const osd = (text: string) => void plugin.osd.show([...text].slice(0, 200).join("")).catch(() => undefined);
  /** Choosing a preset (like dragging a band) turns the equalizer on. */
  const preset = (index: number) => {
    const chosen = presets[index];
    if (chosen) edit({ ...settings, equalizer: true, gains: [...chosen.gains] });
    return chosen;
  };
  const syncBy = (steps: number) => {
    if (edit({ ...settings, delayMs: delay(settings.delayMs + steps * SYNC_STEP_MS) })) osd(syncText(settings.delayMs));
  };
  const setKeep = (keep: SyncKeep) => {
    syncKeep = keep;
    persist();
  };
  const setChannels = (mode: ChannelMode) => {
    if (mono() || !edit({ ...settings, channels: mode })) return;
    const mixed = mode !== "stereo" && channelCount !== null && channelCount > 2 ? "（已混为立体声）" : "";
    osd(`声道: ${channelModes.find((m) => m.id === mode)!.label}${mixed}`);
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

  // A new source resets only the delay, and only under 不保持. A track switch keeps everything;
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
  const loadReason = () => {
    const { phase, kind } = player.state;
    if (phase === "loading") return "正在载入";
    if (phase !== "ready") return "没有正在播放的文件";
    return kind === "stream" ? "生成的画面不能载入音频" : null;
  };
  const loadExternal = async () => {
    if (loading || loadReason()) return;
    loading = true;
    plugin.update();
    try {
      const seen = player.state;
      const path = await plugin.dialogs.openFile({ title: "载入外部音频" });
      if (!path) return;
      const paths = loaded(seen.sourceId);
      if (paths.has(path)) return osd("该音频已载入");
      try {
        await player.addAudioTrack(path, { ifCurrent: seen });
        paths.add(path);
        osd(`已载入音频: ${basename(path)}`);
      } catch (error) {
        osd((error as PluginError).code === "stale" ? "来源已改变，未载入音频" : `无法载入音频: ${(error as Error).message}`);
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
  const noTracks = () => (player.state.audioTracks === null && player.state.phase === "loading" ? "正在载入" : "没有音轨");
  const selectTrack = async (id: number) => {
    const list = tracks();
    const index = list.findIndex((t) => t.id === id);
    const chosen = list[index];
    if (!chosen || chosen.selected) return;
    try {
      await player.selectAudioTrack(id);
      osd(`音轨: ${trackLabel(chosen, index)}`);
    } catch (error) {
      // The source changed meanwhile: the menu follows it, nothing to say.
      if ((error as PluginError).code !== "stale") osd(`无法切换音轨: ${(error as Error).message}`);
    }
  };
  const cycleTrack = () => {
    const list = tracks();
    if (list.length === 1) return osd("只有一条音轨");
    const next = list[(list.findIndex((t) => t.selected) + 1) % list.length];
    return next ? selectTrack(next.id) : undefined;
  };

  // The track items are rebuilt when the list changes (one child scope); the rest of the
  // 音轨 ▶ submenu sits after them at order 512.
  let trackScope: Scope = plugin.scope();
  let trackKey: string | null = null;
  let rebuilding = Promise.resolve();
  const rebuildTracks = () =>
    (rebuilding = rebuilding.then(async () => {
      const list = player.state.audioTracks;
      const labels = list?.map(trackLabel) ?? null;
      const key = JSON.stringify([labels, list?.map((t) => t.id), list === null ? noTracks() : null]);
      if (key === trackKey) return;
      trackKey = key;
      await trackScope.dispose();
      trackScope = plugin.scope();
      if (!list || list.length === 0) {
        trackScope.commands.add({ id: "audio.track.none", path: ["声音", "音轨", noTracks() === "正在载入" ? "正在载入…" : "没有音轨"], order: 511, group: "audio-tools", enabled: false, run: () => undefined });
        return;
      }
      list.forEach((t: AudioTrack, index) =>
        trackScope.commands.add({
          id: `audio.track.${t.id}`,
          path: ["声音", "音轨", labels![index]!],
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
  const nameOf = (id: string | null) =>
    id === null ? SYSTEM_DEFAULT : (devices?.find((d) => d.id === id)?.name ?? (id === device ? deviceName : null) ?? "所选设备");
  /** The menu and window entries: 系统默认, the devices, and the chosen one while it is not connected. */
  const deviceEntries = () => {
    if (!devices?.length) return null;
    const missing = device !== null && !present(device) ? nameOf(device) : null;
    const titles = deviceTitles(devices, missing);
    return [
      { id: null, title: SYSTEM_DEFAULT, name: SYSTEM_DEFAULT, connected: true },
      ...devices.map((d, i) => ({ id: d.id as string | null, title: titles[i]!, name: d.name, connected: true })),
      ...(missing === null ? [] : [{ id: device, title: titles.at(-1)!, name: missing, connected: false }]),
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
    const name = nameOf(id);
    switching++;
    try {
      await player.setAudioDevice(id);
      device = id;
      deviceName = id === null ? null : name;
      lost = false;
      persist();
      osd(`音频输出: ${name}`);
    } catch (error) {
      osd(`无法切换到 ${name}: ${(error as Error).message}`);
    } finally {
      switching--;
      output = player.state.audioOutput ?? output;
      void rebuildDevices();
      plugin.update();
    }
  };
  /** Unplugged: the player falls back to the default; plugged back: it returns to the choice. */
  const followOutput = (state: PlayerState) => {
    const next = state.audioOutput ?? { requested: null, active: null };
    const previous = output;
    output = next;
    if (switching || next.requested === null || next.requested !== previous.requested || next.active === previous.active) return;
    if (previous.active === next.requested) {
      lost = true;
      osd("输出设备已断开，改用系统默认");
      void readDevices();
    } else if (next.active === next.requested && lost) {
      lost = false;
      void readDevices().then(() => osd(`音频输出: ${nameOf(next.requested)}`));
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
        deviceScope.commands.add({ id: "audio.device.none", path: ["声音", "输出设备", NO_DEVICES], ...at, enabled: false, run: () => undefined });
        return;
      }
      entries.forEach((entry, index) =>
        deviceScope.commands.add({
          id: `audio.device.${index}`,
          path: ["声音", "输出设备", entry.title],
          ...at,
          radio: true,
          enabled: entry.connected,
          reason: entry.connected ? null : "设备未连接",
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
  window.bind(() => {
    const list = tracks();
    return {
      statusText: describe(),
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
      trackNames: list.length > 0 ? list.map(trackLabel) : [noTracks()],
      trackIndex: list.length > 0 ? list.findIndex((t) => t.selected) : 0,
      tracksAvailable: list.length > 0,
      canLoad: !loading && loadReason() === null,
      channelIndex: channelModes.findIndex((m) => m.id === settings.channels),
      channelsAvailable: ready() && !mono(),
      channelTip: mono() ? "当前音轨是单声道" : "",
      autoLoad,
      ...deviceProps(),
    };
  });
  window.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    persist();
  });
  window.on("gainChanged", (index: number, value: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= settings.gains.length) return;
    const gains = [...settings.gains];
    gains[index] = tenth(value, -6, 6);
    edit({ ...settings, equalizer: true, gains });
  });
  window.on("equalizerChanged", (on: boolean) => edit({ ...settings, equalizer: on === true }));
  window.on("delayChanged", (value: number) => edit({ ...settings, delayMs: delay(value) }));
  window.on("delayReset", () => edit({ ...settings, delayMs: 0 }));
  window.on("normalizeChanged", (on: boolean) => edit({ ...settings, normalize: on === true }));
  window.on("clarityToggled", (on: boolean) => edit({ ...settings, clarity: { ...settings.clarity, on: on === true } }));
  window.on("clarityChanged", (value: number) => {
    if (settings.clarity.on) edit({ ...settings, clarity: { on: true, db: tenth(value, 0, 6) } });
  });
  window.on("clarityReset", () => edit({ ...settings, clarity: neutralClarity() }));
  window.on("syncKeepSelected", (index: number) => {
    const keep = Number.isInteger(index) ? syncKeeps[index] : undefined;
    if (keep) setKeep(keep.id);
  });
  window.on("presetSelected", (index: number) => preset(index));
  window.on("resetRequested", () => edit(neutral()));
  window.on("retryRequested", () => {
    if (ready() && !busy()) filter.retry();
  });
  window.on("trackSelected", (index: number) => {
    const chosen = Number.isInteger(index) ? tracks()[index] : undefined;
    if (chosen) void selectTrack(chosen.id);
  });
  window.on("loadRequested", () => loadExternal());
  window.on("channelSelected", (index: number) => {
    const mode = Number.isInteger(index) ? channelModes[index] : undefined;
    // The window gives no on-screen text; only the menu does.
    if (mode && !mono()) edit({ ...settings, channels: mode.id });
  });
  window.on("autoLoadChanged", (on: boolean) => setAutoLoad(on === true));
  window.on("deviceSelected", (index: number) => {
    const entry = Number.isInteger(index) ? deviceEntries()?.[index] : undefined;
    if (entry?.connected) void chooseDevice(entry.id);
  });
  /** 输出设备 drop-down: names cut with an ellipsis by the control, the full one on hover. */
  function deviceProps() {
    const entries = deviceEntries();
    if (!entries) return { deviceNames: [NO_DEVICES], deviceIndex: 0, devicesAvailable: false, devicesTip: NO_DEVICES };
    const index = Math.max(0, entries.findIndex((e) => e.id === device));
    const chosen = entries[index]!;
    return {
      deviceNames: entries.map((e) => (e.connected ? deviceLabel(e.name) : `${deviceLabel(e.name)}（未连接）`)),
      deviceIndex: index,
      devicesAvailable: switching === 0,
      devicesTip: chosen.connected ? chosen.name : `${chosen.name}：设备未连接，正在使用系统默认`,
    };
  }

  // ---------------------------------------------------------------- menu 打开 ▶ 114, 声音 ▶ 510–529
  plugin.commands.add({
    id: "audio.load",
    path: ["打开", "载入外部音频…"],
    order: 114,
    group: "open",
    enabled: () => loadReason() === null,
    busy: () => loading,
    reason: loadReason,
    run: loadExternal,
  });
  const reason = () => (ready() ? null : "没有选中的音轨");
  const editing = { enabled: ready, busy, reason };
  // Repeating keys must not be swallowed while a change is applying (the filter keeps the last value).
  const repeating = { enabled: ready, reason, repeat: true };
  plugin.commands.add({
    id: "audio.workbench",
    path: ["声音", "音频工作台…"],
    order: 510,
    group: "audio-tools",
    run: () => window.show(),
  });
  plugin.commands.add({
    id: "audio.track.cycle",
    path: ["声音", "音轨", "按序切换"],
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
      path: ["声音", "音轨", label],
      order: 512,
      group: "audio-channels",
      radio: true,
      enabled: () => ready() && !mono(),
      busy,
      reason: () => reason() ?? (mono() ? "当前音轨是单声道" : null),
      checked: () => settings.channels === id,
      run: () => setChannels(id),
    });
  plugin.commands.add({
    id: "audio.equalizer",
    path: ["声音", "均衡器"],
    order: 513,
    group: "audio-eq",
    bindings: ["Shift+E"],
    ...editing,
    checked: () => settings.equalizer,
    run: () => {
      if (edit({ ...settings, equalizer: !settings.equalizer })) osd(`均衡器: ${settings.equalizer ? "开" : "关"}`);
    },
  });
  presets.forEach(({ label }, index) =>
    plugin.commands.add({
      id: `audio.preset.${index}`,
      path: ["声音", "预设", label],
      order: 514,
      group: "audio-eq",
      radio: true,
      ...editing,
      checked: () => matches(settings, index),
      run: () => {
        if (ready() && preset(index)) osd(`均衡器预设: ${label}`);
      },
    }),
  );
  const sync = (id: string, title: string, binding: string, run: () => void) =>
    plugin.commands.add({ id, path: ["声音", "声音同步", title], order: 515, group: "audio-eq", bindings: [binding], ...repeating, run });
  sync("audio.sync.earlier", `超前 ${SYNC_STEP_MS} 毫秒`, ">", () => syncBy(-1));
  sync("audio.sync.later", `滞后 ${SYNC_STEP_MS} 毫秒`, "<", () => syncBy(1));
  sync("audio.sync.reset", "复位", "?", () => {
    if (edit({ ...settings, delayMs: 0 })) osd(syncText(0));
  });
  syncKeeps.forEach(({ id, label }) =>
    plugin.commands.add({
      id: `audio.sync.keep.${id}`,
      path: ["声音", "声音同步", label],
      order: 515,
      group: "audio-sync-keep",
      radio: true,
      checked: () => syncKeep === id,
      run: () => setKeep(id),
    }),
  );
  plugin.commands.add({
    id: "audio.normalize",
    path: ["声音", "声音处理", "规格化"],
    order: 516,
    group: "audio-eq",
    bindings: ["Shift+N"],
    ...editing,
    checked: () => settings.normalize,
    run: () => {
      if (edit({ ...settings, normalize: !settings.normalize })) osd(`规格化: ${settings.normalize ? "开" : "关"}`);
    },
  });
  plugin.commands.add({
    id: "audio.clarity",
    path: ["声音", "声音处理", "晶化"],
    order: 516,
    group: "audio-eq",
    bindings: ["Shift+C"],
    ...editing,
    checked: () => settings.clarity.on,
    run: () => {
      if (edit({ ...settings, clarity: { ...settings.clarity, on: !settings.clarity.on } }))
        osd(`晶化: ${settings.clarity.on ? "开" : "关"}`);
    },
  });
  plugin.commands.add({
    id: "audio.reset",
    path: ["声音", "复位"],
    order: 517,
    group: "audio-eq",
    ...editing,
    run: () => {
      if (edit(neutral())) osd("音频处理已复位");
    },
  });

  function describe() {
    if (!ready()) return player.state.phase === "loading" ? "正在载入音频…" : "没有选中的音轨";
    const status = filter.status;
    if (status.state === "applying") return "正在应用…";
    if (status.state === "failed") return "音频处理失败";
    return touched ? (status.state === "applied" ? "已应用" : "正在应用…") : "就绪";
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
