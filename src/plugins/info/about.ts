import type { AppInfo, Plugin } from "../../sdk/plugin/index.ts";

/** The protocol version in manifest.json (the 插件 API rows of I05 and I09). */
export const API_VERSION = "vestra.experimental/0.1";
const UNKNOWN = "未知";

/** `0.1.0（构建 289b26b）`, or the bare version without a build hash. */
export const vestraVersion = (app: AppInfo) => (app.build ? `${app.version}（构建 ${app.build}）` : app.version);

/**
 * The FFmpeg row from what libmpv reports: `ffmpeg-version` first, then any
 * `libav*` library versions (libmpv 0.41 reports none); 未知 when empty.
 */
export function ffmpegVersion(app: AppInfo) {
  const { ffmpeg, ...libraries } = app.components.libav;
  const parts = [ffmpeg, ...Object.entries(libraries).map(([name, version]) => `${name} ${version}`)].filter(Boolean);
  return parts.length ? parts.join(" · ") : UNKNOWN;
}

/** I09 component rows, `[label, value]`; a version that is not known shows 未知 (the row stays). */
export function components(app: AppInfo, node: string | null): [string, string][] {
  return [
    ["插件 API", API_VERSION],
    ["mpv", app.components.mpv || UNKNOWN],
    ["FFmpeg", ffmpegVersion(app)],
    ["Slint", app.components.slint || UNKNOWN],
    ["Node", node || UNKNOWN],
  ];
}

/** One row of the loaded-plugin list (`packages.list`). */
export type PluginRow = { id: string; version: string; origin: string; state: string };
const ORIGINS: Record<string, string> = { installed: "已安装", development: "开发加载", trial: "试载" };
const STATES: Record<string, string> = {
  starting: "启动中",
  running: "运行中",
  disabled: "已停用",
  failed: "失败",
  stopped: "已停止",
};
/** Rows for `packages.list()` results, sorted by id. */
export function pluginRows(plugins: { id: string; version: string | null; origin: string; state: string }[]): PluginRow[] {
  return [...plugins]
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .map((p) => ({
      id: p.id,
      version: p.version ?? UNKNOWN,
      origin: ORIGINS[p.origin] ?? p.origin,
      state: STATES[p.state] ?? p.state,
    }));
}

/** What the plugin list shows when `packages.list` failed (it is read again on the next opening). */
export const PLUGINS_UNKNOWN = "未知（无法读取插件列表）";

/** The whole window as plain text, for 复制信息. */
export function aboutText(app: AppInfo, node: string | null, plugins: PluginRow[] | null) {
  const lines = ["Vestra", `版本 ${vestraVersion(app)}`, ""];
  for (const [label, value] of components(app, node)) lines.push(`${label}：${value}`);
  lines.push("", "已加载插件");
  if (!plugins) lines.push(PLUGINS_UNKNOWN);
  else for (const p of plugins) lines.push(`${p.id}  ${p.version}  ${p.origin}  ${p.state}`);
  return `${lines.join("\n")}\n`;
}

/**
 * I09 关于 Vestra: a fixed 360×420 tool window with the logo, the version,
 * component versions (`plugin.app`) and the loaded plugins (`packages.list`,
 * read on opening and kept current by `packages.onChange`). F1 opens it, or
 * brings the open window to the front.
 */
export function aboutWindow(plugin: Plugin, say: (text: string) => void) {
  const node = process.versions.node ? `v${process.versions.node}` : null;
  let plugins: PluginRow[] | null = null;
  let status = "";
  /** Read the list now (each opening); a failure shows 未知 until the next read or change. */
  async function readPlugins() {
    try {
      plugins = pluginRows(await plugin.packages.list());
    } catch (error) {
      console.error(`[info] packages.list: ${error instanceof Error ? error.message : String(error)}`);
      plugins = null;
    }
    plugin.update();
  }
  // Starts, stops, failures, installs and uninstalls arrive here; the open window follows at once.
  plugin.packages.onChange((list) => void (plugins = pluginRows(list)));
  const window = plugin.windows.create("ui/about.slint");
  window.bind(() => ({
    version: `版本 ${vestraVersion(plugin.app)}`,
    components: components(plugin.app, node).map(([label, value]) => ({ label, value })),
    plugins: plugins ?? [],
    pluginsNote: plugins ? "" : PLUGINS_UNKNOWN,
    status,
  }));
  window.on("copyInfo", async () => {
    try {
      await plugin.clipboard.writeText(aboutText(plugin.app, node, plugins));
      status = "";
      say("已复制版本信息");
    } catch (error) {
      console.error(`[info] copy: ${error instanceof Error ? error.message : String(error)}`);
      status = "复制失败";
    }
  });
  window.on("closeWindow", () => window.close());
  // Nothing polls while the window is open; a title-bar close only clears the status line.
  window.onClose(() => void (status = ""));
  return async function show() {
    status = "";
    if (!window.opened) await readPlugins();
    await window.show();
  };
}
