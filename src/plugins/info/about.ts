import type { AppInfo, PackageSource, Plugin } from "@vestra-player/sdk";

/** The protocol version in manifest.json (the Plugin API rows of I05 and I09). */
export const API_VERSION = "vestra/1";
// i18n: needs r2-api (UNKNOWN is a word inside plain text: properties rows, the clipboard)
const UNKNOWN = "Unknown";

/** `0.1.0 (build 289b26b)`, or the bare version without a build hash. */
// i18n: needs r2-api (a word inside a properties-row value and the clipboard text)
export const vestraVersion = (app: AppInfo) => (app.build ? `${app.version} (build ${app.build})` : app.version);

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

/** I09 component rows as plain text, `[label, value]`; a version that is not known shows Unknown. */
// i18n: needs r2-api (the copied text)
export function components(app: AppInfo, node: string | null): [string, string][] {
  const v = versions(app, node);
  return [
    ["Plugin API", v.api],
    ["mpv", v.mpv || UNKNOWN],
    ["FFmpeg", v.ffmpeg || UNKNOWN],
    ["Slint", v.slint || UNKNOWN],
    ["Node", v.node || UNKNOWN],
  ];
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

// i18n: needs r2-api (the copied text names origins and states in words; the window translates them in about.slint)
const ORIGIN_WORDS: Record<string, string> = {
  bundled: "Built-in", repository: "Repository", link: "Link", file: "Local file", workshop: "Workshop", installed: "Installed", development: "Development",
};
const STATE_WORDS: Record<string, string> = { starting: "Starting", running: "Running", disabled: "Disabled", failed: "Failed", stopped: "Stopped" };

/** What the copied text says when `packages.list` failed (it is read again on the next opening). */
export const PLUGINS_UNKNOWN = "Unknown (cannot read the plugin list)";

/** The whole window as plain text, for Copy Info. */
// i18n: needs r2-api (the copied text: fixed words and data composed in TypeScript)
export function aboutText(app: AppInfo, node: string | null, plugins: PluginRow[] | null) {
  const lines = ["Vestra Player", `Version ${vestraVersion(app)}`, ""];
  for (const [label, value] of components(app, node)) lines.push(`${label}: ${value}`);
  lines.push("", "Loaded plugins");
  if (!plugins) lines.push(PLUGINS_UNKNOWN);
  else for (const p of plugins) lines.push(`${p.id}  ${p.version || UNKNOWN}  ${ORIGIN_WORDS[p.origin] ?? p.origin}  ${STATE_WORDS[p.state] ?? p.state}`);
  return `${lines.join("\n")}\n`;
}

/**
 * I09 About Vestra Player: a fixed 360×420 tool window with the logo, the version,
 * component versions (`plugin.app`) and the loaded plugins (`packages.list`,
 * read on opening and kept current by `packages.onChange`). F1 opens it, or
 * brings the open window to the front.
 */
export function aboutWindow(plugin: Plugin) {
  const node = process.versions.node ? `v${process.versions.node}` : null;
  let plugins: PluginRow[] | null = null;
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
  // Starts, stops, failures, installs and uninstalls arrive here; the open window follows at once.
  plugin.packages.onChange((list) => void (plugins = pluginRows(list)));
  const window = plugin.windows.create("ui/about.slint");
  window.bind(() => ({
    version: plugin.app.version,
    build: plugin.app.build ?? "",
    ...versions(plugin.app, node),
    plugins: plugins ?? [],
    pluginsUnknown: !plugins,
    status,
  }));
  window.on("copyInfo", async () => {
    try {
      await plugin.clipboard.writeText(aboutText(plugin.app, node, plugins));
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
    if (!window.opened) await readPlugins();
    await window.show();
  };
}
