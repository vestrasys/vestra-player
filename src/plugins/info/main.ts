import type { PlaybackStats, PlayerState, Plugin } from "../../sdk/plugin/index.ts";

/** "m:ss", or "h:mm:ss" from one hour. */
function clock(seconds: number | null) {
  if (seconds === null || !Number.isFinite(seconds)) return "--:--";
  const total = Math.max(0, Math.floor(seconds));
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

const or = (value: unknown) => (value === null || value === undefined ? "-" : String(value));

/** The overlay text: everything the old host diagnostic line showed, and the stream facts. */
export function describe(state: PlayerState, stats: PlaybackStats) {
  const { video, audio } = stats;
  return [
    state.source ?? "未打开媒体",
    `${clock(state.position)} / ${clock(state.duration)}`,
    `视频 ${or(video?.codec)} ${or(video?.width)}×${or(video?.height)} ${video?.fps ? video.fps.toFixed(2) : "-"} fps`,
    `音频 ${or(audio?.codec)} ${or(audio?.sampleRate)} Hz ${or(audio?.channels)} 声道`,
    `${stats.backend} · session ${stats.session} · 帧 ${stats.framesShown} · 丢帧 ${or(stats.framesDropped)}`,
  ].join("\n");
}

/** Playback information: Tab shows a read-only overlay on the video, refreshed every second. */
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();
  let shown = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  const render = async () => {
    const stats = await plugin.player.stats();
    if (shown) panel.set(describe(plugin.player.state, stats));
  };
  const refresh = () => void render().catch(() => {});
  plugin.commands.add({
    id: "info.toggle",
    path: ["视图", "播放信息"],
    order: 503,
    bindings: ["Tab"],
    checked: () => shown,
    run: () => {
      shown = !shown;
      clearInterval(timer);
      if (!shown) return panel.set(null);
      refresh();
      timer = setInterval(refresh, 1000);
    },
  });
  plugin.onDispose(() => clearInterval(timer));
}
