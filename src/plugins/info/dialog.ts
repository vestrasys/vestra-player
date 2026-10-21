import { rename, rm, stat, writeFile } from "node:fs/promises";
import { basename } from "node:path";
import type { PlaybackStats, PlayerState, Plugin } from "@vestra-player/sdk";
import { filePage, isUrl, message, pageText, playbackPage, remotePage, saveName, systemPage } from "./properties.ts";
import type { Row, SystemFacts } from "./properties.ts";
import { readFacts } from "./sysinfo.ts";

const TABS = 3;
const PROBE_ARGS = ["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json"];

type Prefs = { propertiesTab: number };
const isPrefs = (value: unknown): value is Prefs => typeof value === "object" && value !== null && !Array.isArray(value);
const reason = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** The 文件信息 failure in one line: a missing ffprobe, a timeout, or ffprobe's own last line. */
function probeError(error: unknown) {
  switch ((error as { code?: string }).code) {
    case "unavailable":
      return "无法读取文件信息（未找到 ffprobe）";
    case "timeout":
      return "无法读取文件信息：读取超时";
    default:
      return `无法读取文件信息：${reason(error).split("\n").at(-1)!.slice(0, 120) || "ffprobe 失败"}`;
  }
}

/**
 * I05 播放属性: a tool window with three pages. 播放信息 follows `player.stats()`
 * every second while the window is open; 文件信息 is read once per source with
 * ffprobe (`plugin.tools`, aborted when the source changes); 系统信息 once per
 * opening (`plugin.app.system()` and Node `os`). Closing the window (its 关闭 button or the
 * title bar, `onClose`) stops the polling. Copy and save take the current page's text.
 */
export function propertiesWindow(plugin: Plugin, say: (text: string) => void) {
  const { player } = plugin;
  let prefs: { value: Prefs; save(next: Prefs): void } = { value: { propertiesTab: 0 }, save: (next) => void (prefs.value = next) };
  try {
    prefs = plugin.storage.json<Prefs>("info", { defaults: { propertiesTab: 0 }, validate: isPrefs });
  } catch {
    // no data directory: the tab is remembered for this run only
  }
  const saved = Math.round(Number(prefs.value.propertiesTab));
  let tab = saved >= 0 && saved < TABS ? saved : 0;
  let status = "";
  let stats: PlaybackStats | null = null;
  let file: { sourceId: number; rows: Row[] } | null = null;
  let probe: AbortController | null = null;
  let facts: SystemFacts | null = null;
  let reading: AbortController | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  let sourceId = player.state.sourceId;

  const page = (): Row[] => {
    const state = player.state;
    if (tab === 0) return playbackPage(state, stats);
    if (tab === 2) return systemPage(plugin.app, facts);
    if (!state.source) return message("未打开媒体");
    if (isUrl(state.source)) return remotePage(state, stats);
    return file?.sourceId === state.sourceId ? file.rows : message("正在读取…");
  };

  const window = plugin.windows.create("ui/properties.slint");
  window.bind(() => ({ tabIndex: tab, rows: page() as unknown as Record<string, string | boolean>[], status, canRefresh: tab !== 0 }));

  async function readStats() {
    try {
      stats = await player.stats();
    } catch {
      // keep the last values; try again next second
    }
  }

  /** 文件信息 of a local source, unless already read (or `again`). The previous read is aborted. */
  async function readFile(again = false) {
    const state: PlayerState = player.state;
    if (!state.source || isUrl(state.source) || (!again && file?.sourceId === state.sourceId)) return;
    probe?.abort();
    const controller = (probe = new AbortController());
    file = null;
    plugin.update(); // 正在读取…
    let rows: Row[];
    try {
      const info = await stat(state.source).catch(() => {
        throw new Error("文件不存在");
      });
      const { stdout } = await plugin.tools.run("ffprobe", [...PROBE_ARGS, state.source], { timeoutMs: 10_000, signal: controller.signal });
      let parsed;
      try {
        parsed = JSON.parse(new TextDecoder().decode(stdout));
      } catch {
        throw new Error("ffprobe 输出无法解析");
      }
      rows = filePage(state.source, info, parsed);
    } catch (error) {
      if (controller.signal.aborted) return; // a newer read (or closing) took over
      rows = message(probeError(error));
    }
    if (probe !== controller) return;
    probe = null;
    file = { sourceId: state.sourceId, rows };
    plugin.update();
  }

  /** 系统信息, unless already read (or `again`). */
  async function readSystem(again = false) {
    if (reading || (facts && !again)) return;
    const controller = (reading = new AbortController());
    if (again) facts = null;
    plugin.update();
    try {
      const read = await readFacts(plugin.app);
      if (!controller.signal.aborted) facts = read; // closed meanwhile: read again next time
    } finally {
      if (reading === controller) reading = null;
      plugin.update();
    }
  }

  /** What the shown page needs. */
  function load() {
    if (tab === 1) void readFile();
    if (tab === 2) void readSystem();
  }

  function stop() {
    clearInterval(timer);
    timer = undefined;
    probe?.abort();
    probe = null;
    reading?.abort();
    reading = null;
  }

  window.on("tabChanged", (index: number) => {
    tab = Math.min(TABS - 1, Math.max(0, Math.round(Number(index))));
    status = "";
    if (prefs.value.propertiesTab !== tab) prefs.save({ propertiesTab: tab });
    load();
  });
  window.on("refresh", () => {
    status = "";
    if (tab === 1) void readFile(true);
    if (tab === 2) void readSystem(true);
  });
  window.on("copyPage", async () => {
    try {
      await plugin.clipboard.writeText(pageText(page()));
      status = "";
      say("已复制播放信息");
    } catch (error) {
      console.error(`[info] copy: ${reason(error)}`);
      status = "复制失败";
    }
  });
  window.on("savePage", async () => {
    const text = pageText(page()); // what the user saw when asking
    const chosen = await plugin.dialogs.saveFile({ title: "保存为", fileName: saveName(player.state.source) });
    if (!chosen) return;
    const temporary = `${chosen}.vestra-${process.pid}.tmp`;
    try {
      await writeFile(temporary, text);
      await rename(temporary, chosen);
      status = "";
      say(`已保存到 ${basename(chosen)}`);
    } catch (error) {
      await rm(temporary, { force: true });
      status = `保存失败：${reason(error)}`;
    }
  });
  window.on("closeWindow", async () => {
    stop();
    await window.close();
  });
  // Closed with the title bar: stop the 1-second stats polling and any read (reopening starts them again).
  window.onClose(stop);

  player.onChange((state) => {
    if (state.sourceId === sourceId) return;
    sourceId = state.sourceId;
    stats = null;
    if (!window.opened) return;
    probe?.abort();
    probe = null;
    void readStats();
    load();
  });
  plugin.onDispose(stop);

  /** Ctrl+F1: open, or bring the open window to the front. */
  return async function show() {
    if (!timer) {
      await readStats();
      timer = setInterval(() => void readStats().then(() => plugin.update()), 1000);
    }
    load();
    await window.show();
  };
}
