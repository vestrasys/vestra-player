import type { PlaybackStats, PlayerState, Plugin } from "../../sdk/plugin/index.ts";

/** "mm:ss", or "h:mm:ss" from one hour. */
function clock(seconds: number | null) {
  if (seconds === null || !Number.isFinite(seconds)) return "--:--";
  const total = Math.max(0, Math.floor(seconds));
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

const or = (value: unknown) => (value === null || value === undefined || value === "" ? "-" : String(value));

/** The file name of a path, or the last path segment of a URL (its host when it has none). */
export function fileName(source: string | null) {
  if (!source) return "未打开媒体";
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(source)) {
    try {
      const url = new URL(source);
      const last = url.pathname.split("/").filter(Boolean).at(-1);
      return last ? decodeURIComponent(last) : url.hostname || source;
    } catch {
      return source;
    }
  }
  return source.split(/[\\/]/).filter(Boolean).at(-1) ?? source;
}

/** The overlay text (ui-alignment §9.2): one fact per line, Chinese labels. */
export function describe(state: PlayerState, stats: PlaybackStats) {
  const { video, audio } = stats;
  const size = video?.width && video?.height ? `${video.width}×${video.height}` : "-";
  return [
    `文件：${fileName(state.source)}`,
    `时间：${clock(state.position)} / ${clock(state.duration)}`,
    `视频：${or(video?.codec)} ${size}`,
    `帧率：${video?.fps ? `${video.fps.toFixed(2)} fps` : "-"}`,
    `音频：${or(audio?.codec)} ${audio?.sampleRate ? `${audio.sampleRate} Hz` : "-"} ${audio?.channels ? `${audio.channels} 声道` : "-"}`,
    `后端：${or(stats.backend)}（会话 ${stats.session}）`,
    `已显示帧：${stats.framesShown}`,
    `丢帧：${or(stats.framesDropped)}`,
  ].join("\n");
}

/** Playback information: Tab shows a read-only overlay on the video, refreshed every second. */
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();
  let shown = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  const render = async () => {
    const stats = await plugin.player.stats();
    if (shown) panel.set(describe(plugin.player.state, stats)); // may have been switched off meanwhile
  };
  const refresh = () => void render().catch(() => {}); // keep the last text; try again next second
  plugin.commands.add({
    id: "info.toggle",
    path: ["播放信息"],
    order: 810,
    group: "windows",
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
