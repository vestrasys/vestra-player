import type { AppInfo, I18n, PackageSource, Plugin, SystemInfo, TextArg } from "@vestra-player/sdk";

/** The protocol version in manifest.json (the Plugin API rows of I05 and I09). */
export const API_VERSION = "vestra/1";
/** `0.1.0 (build 289b26b)`, or the bare version without a build hash. */
export const vestraVersion = (i18n: I18n, app: AppInfo): TextArg => (app.build ? i18n.text("{} (build {})", [app.version, app.build]) : app.version);

/**
 * The FFmpeg version from what libmpv reports: `ffmpeg-version` first, then
 * any `libav*` library versions (libmpv 0.41 reports none); `""` when empty.
 */
export function ffmpegVersion(app: AppInfo) {
  const { ffmpeg, ...libraries } = app.components.libav;
  return [ffmpeg, ...Object.entries(libraries).map(([name, version]) => `${name} ${version}`)].filter(Boolean).join(" · ");
}

/** I09 component versions; `""` = not known (the window shows Unknown, the row stays). */
export function versions(app: AppInfo, node: string | null) {
  return {
    api: API_VERSION,
    mpv: app.components.mpv || "",
    ffmpeg: ffmpegVersion(app),
    slint: app.components.slint || "",
    node: node || "",
  };
}

/** I09 component rows of the copied text, `[label, value]`; a version that is not known shows Unknown. */
export function components(i18n: I18n, app: AppInfo, node: string | null): [TextArg, TextArg][] {
  const { text } = i18n;
  const v = versions(app, node);
  const unknown = text("Unknown");
  return [
    [text("Plugin API"), v.api],
    ["mpv", v.mpv || unknown],
    ["FFmpeg", v.ffmpeg || unknown],
    ["Slint", v.slint || unknown],
    ["Node", v.node || unknown],
  ];
}

/**
 * The Rendering row: the renderer drawing now (`"gpu"` / `"software"`, a code
 * about.slint words) and the player's reason (`""` = none shown). `mode` is
 * `""` when the player does not report a renderer (before 0.5.12): the row is
 * left out rather than guessed.
 */
export type RenderingRow = { mode: "" | "gpu" | "software"; reason: string };
export const NO_RENDERING: RenderingRow = { mode: "", reason: "" };
/**
 * The row from `plugin.app.system()` (`null`: the read failed). The reason is
 * the player's English message, shown as it is (it may point to Windows'
 * graphics settings): always under software rendering, and under GPU rendering
 * unless it only says nothing ruled the GPU out (`default`) or that it is
 * starting (`pending`). Players before 0.5.13 report no reason: the mode alone.
 */
export function renderingRow(info: Pick<SystemInfo, "rendering" | "renderingReason"> | null): RenderingRow {
  const mode = info?.rendering;
  if (mode !== "gpu" && mode !== "software") return NO_RENDERING;
  const reason = info?.renderingReason;
  const quiet = mode === "gpu" && (reason?.code === "default" || reason?.code === "pending");
  return { mode, reason: !reason || quiet ? "" : reason.message };
}

/** `Software — <reason>`, `GPU`: the Rendering row's value in the interface language (as about.slint words it). */
export function renderingValue(i18n: I18n, row: RenderingRow) {
  const { text } = i18n;
  const mode = row.mode === "gpu" ? text("GPU") : text("Software");
  return row.reason ? text("{} — {}", [mode, row.reason]) : mode;
}

/**
 * One row of the loaded-plugin list (`packages.list`): `origin` and `state`
 * are codes the window names (`about.slint`); `version` is `""` when unknown.
 */
export type PluginRow = { id: string; version: string; origin: string; state: string };
type Listed = { id: string; version: string | null; origin: string; state: string; builtin?: boolean; source?: PackageSource | null };
/** 「Source」 as 插件管理 / 商城 name it: the package source's kind, `bundled` for a built-in, else `installed` / `development`. */
function origin(p: Listed) {
  if (p.builtin) return "bundled";
  if (p.source) return p.source.kind;
  return p.origin;
}
/** Rows for `packages.list()` results, sorted by id. */
export function pluginRows(plugins: Listed[]): PluginRow[] {
  return [...plugins]
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .map((p) => ({ id: p.id, version: p.version ?? "", origin: origin(p), state: p.state }));
}

/** The origin and state words of the copied text, as about.slint words the codes. */
const ORIGIN_WORDS = {
  bundled: "Built-in", repository: "Repository", link: "Link", file: "Local file", workshop: "Workshop", installed: "Installed", development: "Development",
} as const;
const STATE_WORDS = { starting: "Starting", running: "Running", disabled: "Disabled", failed: "Failed", stopped: "Stopped" } as const;

/** The whole window as plain text in the interface language, for Copy Info (it leaves the player: resolved here). */
export function aboutText(i18n: I18n, app: AppInfo, node: string | null, plugins: PluginRow[] | null, rendering: RenderingRow = NO_RENDERING) {
  const { text, list, resolve } = i18n;
  const lines: TextArg[] = ["Vestra Player", app.build ? text("Version {} (build {})", [app.version, app.build]) : text("Version {}", [app.version]), ""];
  for (const [label, value] of components(i18n, app, node)) lines.push(text("{}: {}", [label, value]));
  if (rendering.mode) lines.push(text("{}: {}", [text("Rendering"), renderingValue(i18n, rendering)]));
  lines.push("", text("Loaded plugins"));
  if (!plugins) lines.push(text("Unknown (cannot read the plugin list)"));
  else
    for (const p of plugins) {
      const origin = Object.hasOwn(ORIGIN_WORDS, p.origin) ? text(ORIGIN_WORDS[p.origin as keyof typeof ORIGIN_WORDS]) : p.origin;
      const state = Object.hasOwn(STATE_WORDS, p.state) ? text(STATE_WORDS[p.state as keyof typeof STATE_WORDS]) : p.state;
      lines.push(text("{}  {}  {}  {}", [p.id, p.version || text("Unknown"), origin, state]));
    }
  return `${resolve(list(lines, "lines"))}\n`;
}

/**
 * I09 About Vestra Player: a 360-wide tool window with the logo, the version,
 * component versions (`plugin.app`), the renderer and why (`app.system()`,
 * read on opening) and the loaded plugins (`packages.list`, read on opening
 * and kept current by `packages.onChange`). F1 opens it, or brings the open
 * window to the front.
 */
export function aboutWindow(plugin: Plugin) {
  const node = process.versions.node ? `v${process.versions.node}` : null;
  let plugins: PluginRow[] | null = null;
  let rendering = NO_RENDERING;
  /** `""`, or `copy-failed` (about.slint words it). */
  let status = "";
  /** Read the list now (each opening); a failure shows Unknown until the next read or change. */
  async function readPlugins() {
    try {
      plugins = pluginRows(await plugin.packages.list());
    } catch (error) {
      console.error(`[info] packages.list: ${error instanceof Error ? error.message : String(error)}`);
      plugins = null;
    }
    plugin.update();
  }
  /** The renderer now (each opening); a failed read leaves the row out, as an older player does. */
  async function readRendering() {
    try {
      rendering = renderingRow(await plugin.app.system());
    } catch (error) {
      console.error(`[info] app.system: ${error instanceof Error ? error.message : String(error)}`);
      rendering = NO_RENDERING;
    }
    plugin.update();
  }
  // Starts, stops, failures, installs and uninstalls arrive here; the open window follows at once.
  plugin.packages.onChange((list) => void (plugins = pluginRows(list)));
  const window = plugin.windows.create("ui/about.slint");
  window.bind(() => ({
    version: plugin.app.version,
    build: plugin.app.build ?? "",
    ...versions(plugin.app, node),
    plugins: plugins ?? [],
    pluginsUnknown: !plugins,
    rendering: rendering.mode,
    renderingReason: rendering.reason,
    status,
  }));
  window.on("copyInfo", async () => {
    try {
      await plugin.clipboard.writeText(aboutText(plugin.i18n, plugin.app, node, plugins, rendering));
      status = "";
      void plugin.osd.show("Version info copied").catch(() => {});
    } catch (error) {
      console.error(`[info] copy: ${error instanceof Error ? error.message : String(error)}`);
      status = "copy-failed";
    }
  });
  window.on("closeWindow", () => window.close());
  // Nothing polls while the window is open; a title-bar close only clears the status line.
  window.onClose(() => void (status = ""));
  return async function show() {
    status = "";
    if (!window.opened) await Promise.all([readPlugins(), readRendering()]);
    await window.show();
  };
}
