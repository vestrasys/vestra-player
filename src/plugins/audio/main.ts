import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Plugin, PlayerState } from "../../sdk/plugin/index.ts";
import {
  SYNC_STEP_MS,
  TABS,
  delay,
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
} from "./model.ts";
import type { Settings, SyncKeep } from "./model.ts";

/**
 * Audio workbench: one window and one owned audio chain. The equalizer, presets,
 * 规格化 and 晶化 are global and saved in `settings.json`; only 声音同步 follows
 * the source, as its keep mode says.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const filter = plugin.audio.filter("workbench");
  const window = plugin.windows.create("ui/workbench.slint", { id: "audio-workbench" });
  const file = settingsFile(plugin);
  const stored = parse(read(file));
  let settings = stored.settings;
  let syncKeep: SyncKeep = stored.syncKeep;
  let tab = stored.tab;
  let source: number | null = null;
  let track = "";
  // A change was requested, or saved settings are in effect: "已应用" then means something.
  let touched = nodes(settings).length > 0 || settings.delayMs !== 0;

  // ---------------------------------------------------------------- settings file
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  const save = () => {
    clearTimeout(saveTimer);
    saveTimer = undefined;
    if (!file) return;
    try {
      writeFileSync(`${file}.tmp`, JSON.stringify(serialize({ settings, syncKeep, tab, extra: stored.extra }), null, 2));
      renameSync(`${file}.tmp`, file);
    } catch (e) {
      console.error(`[audio] settings not saved: ${(e as Error).message}`);
    }
  };
  /** Slider drags write once they pause. */
  const saveSoon = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 300);
    saveTimer.unref?.();
  };
  plugin.onDispose(() => {
    if (saveTimer) save();
  });

  const ready = () => {
    const { phase, audioTracks } = player.state;
    return phase === "ready" && (audioTracks === null || audioTracks.some((t) => t.selected));
  };
  const busy = () => filter.status.state === "applying";
  const apply = () => filter.set({ nodes: nodes(settings), delayMs: settings.delayMs });
  /** Menu and window edits are ignored without a selected track, like the disabled controls. */
  const edit = (next: Settings) => {
    if (!ready()) return false;
    settings = next;
    touched = true;
    apply();
    saveSoon();
    return true;
  };
  /** Best-effort feedback: a missing OSD (headless) must not look like a processing failure. */
  const osd = (text: string) => void plugin.osd.show(text).catch(() => undefined);
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
    save();
  };

  // A new source resets only the delay, and only under 不保持. A track switch keeps everything;
  // re-setting the same value also drops a failure the previous track left behind.
  const follow = (state: PlayerState) => {
    const current = state.audioTracks?.filter((t) => t.selected).map((t) => t.id).join(",") ?? "?";
    if (state.sourceId === source && current === track) return;
    const newSource = source !== null && state.sourceId !== source;
    source = state.sourceId;
    track = current;
    if (newSource && syncKeep === "none" && settings.delayMs !== 0) settings = { ...settings, delayMs: 0 };
    apply(); // a never-applied neutral chain sends nothing
  };
  player.onChange(follow);
  follow(player.state);

  window.bind(() => ({
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
  }));
  window.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    saveSoon();
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

  // ---------------------------------------------------------------- menu 声音 ▶ 510–529
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

function settingsFile(plugin: Plugin) {
  try {
    return join(plugin.dataDirectory, "settings.json");
  } catch {
    return null; // no data directory: settings last for this run only
  }
}
/** A missing or broken file reads as the defaults. */
function read(file: string | null): unknown {
  if (!file) return null;
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}
