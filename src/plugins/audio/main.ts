import type { Plugin, PlayerState } from "../../sdk/plugin/index.ts";
import { clamp, matches, neutral, nodes, presetOf, presets } from "./model.ts";
import type { Settings } from "./model.ts";

/** Audio workbench: one window and one owned audio chain for the selected track. */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const filter = plugin.audio.filter("workbench");
  const window = plugin.windows.create("ui/workbench.slint", { id: "audio-workbench" });
  let settings = neutral();
  let track = "";
  let edited = false; // ever changed: a track change then really resets something
  let touched = false; // a change was requested for the current track

  const ready = () => {
    const { phase, audioTracks } = player.state;
    return phase === "ready" && (audioTracks === null || audioTracks.some((t) => t.selected));
  };
  const busy = () => filter.status.state === "applying";
  const apply = () => filter.set({ nodes: nodes(settings), delayMs: settings.delayMs });
  /** Menu and window edits are ignored without a selected track, like the disabled controls. */
  const edit = (next: Settings) => {
    if (!ready()) return;
    settings = next;
    edited = touched = true;
    apply();
  };
  /** Best-effort feedback: a missing OSD (headless) must not look like a processing failure. */
  const osd = (text: string) => void plugin.osd.show(text).catch(() => undefined);
  /** Choosing a preset (like dragging a band) turns the equalizer on. */
  const preset = (index: number) => {
    const chosen = presets[index];
    if (chosen) edit({ ...settings, equalizer: true, gains: [...chosen.gains] });
    return chosen;
  };

  // Settings belong to one loaded source and one selected track: either change resets to neutral.
  const follow = (state: PlayerState) => {
    const current = `${state.sourceId}:${state.audioTracks?.filter((t) => t.selected).map((t) => t.id).join(",") ?? "?"}`;
    if (current === track) return;
    track = current;
    settings = neutral();
    touched = edited;
    apply(); // also drops a failure left by the previous track; never-applied neutral sends nothing
  };
  player.onChange(follow);
  follow(player.state);

  window.bind(() => ({
    statusText: describe(),
    sourceReady: ready(),
    isBusy: busy(),
    hasError: filter.status.state === "failed",
    errorDetail: filter.status.state === "failed" ? filter.status.error.message : "",
    equalizerEnabled: settings.equalizer,
    presetIndex: presetOf(settings),
    gainValues: settings.gains,
    delayMs: settings.delayMs,
    normalizeEnabled: settings.normalize,
    clarityDb: settings.clarity,
  }));
  window.on("gainChanged", (index: number, value: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= settings.gains.length) return;
    const gains = [...settings.gains];
    gains[index] = Math.round(clamp(value, -6, 6) * 10) / 10;
    edit({ ...settings, equalizer: true, gains });
  });
  window.on("equalizerChanged", (on: boolean) => edit({ ...settings, equalizer: on === true }));
  window.on("delayChanged", (value: number) =>
    edit({ ...settings, delayMs: Math.round(clamp(value, -2000, 2000) / 10) * 10 }),
  );
  window.on("normalizeChanged", (on: boolean) => edit({ ...settings, normalize: on === true }));
  window.on("clarityChanged", (value: number) =>
    edit({ ...settings, clarity: Math.round(clamp(value, 0, 6) * 10) / 10 }),
  );
  window.on("presetSelected", (index: number) => preset(index));
  window.on("resetRequested", () => edit(neutral()));
  window.on("retryRequested", () => {
    if (ready() && !busy()) filter.retry();
  });

  const editing = { enabled: ready, busy, reason: () => (ready() ? null : "没有选中的音轨") };
  // 声音 ▶ 510–529 (round allocation), after playback's volume items.
  const group = "domain";
  plugin.commands.add({
    id: "audio.workbench",
    path: ["声音", "音频工作台…"],
    order: 510,
    group,
    run: () => window.show(),
  });
  plugin.commands.add({
    id: "audio.equalizer",
    path: ["声音", "均衡器"],
    order: 511,
    group,
    bindings: ["Shift+E"],
    ...editing,
    checked: () => settings.equalizer,
    run: () => {
      if (!ready()) return;
      edit({ ...settings, equalizer: !settings.equalizer });
      osd(`均衡器: ${settings.equalizer ? "开" : "关"}`);
    },
  });
  presets.forEach(({ label }, index) =>
    plugin.commands.add({
      id: `audio.preset.${index}`,
      path: ["声音", "预设", label],
      order: 512 + index,
      group,
      radio: true,
      ...editing,
      checked: () => matches(settings, index),
      run: () => {
        if (ready() && preset(index)) osd(`均衡器预设: ${label}`);
      },
    }),
  );
  plugin.commands.add({
    id: "audio.reset",
    path: ["声音", "复位"],
    order: 516,
    group,
    ...editing,
    run: () => {
      if (!ready()) return;
      edit(neutral());
      osd("音频处理已复位");
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
