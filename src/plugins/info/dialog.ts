import { rename, rm, stat, writeFile } from "node:fs/promises";
import { basename } from "node:path";
import type { I18n, PlaybackStats, PlayerState, Plugin, Scope, TextArg } from "@vestra-player/sdk";
import { filePage, isUrl, message, pageText, playbackPage, remotePage, saveName, systemPage, widestLabel } from "./properties.ts";
import type { Row, SystemFacts } from "./properties.ts";
import { readFacts } from "./sysinfo.ts";

const TABS = 3;
const PROBE_ARGS = ["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json"];

type Prefs = { propertiesTab: number };
const isPrefs = (value: unknown): value is Prefs => typeof value === "object" && value !== null && !Array.isArray(value);
const reason = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** The File page failure in one line: a missing ffprobe, a timeout, our own reason, or ffprobe's own last line. */
function probeError(i18n: I18n, error: unknown) {
  const { text } = i18n;
  switch ((error as { code?: string }).code) {
    case "unavailable":
      return text("Cannot read file information (ffprobe not found)");
    case "timeout":
      return text("Cannot read file information: timed out");
  }
  const own = (error as { text?: TextArg }).text;
  const last = reason(error).split("\n").at(-1)!.slice(0, 120);
  return text("Cannot read file information: {}", [own ?? (last || text("ffprobe failed"))]);
}

/**
 * I05 Playback Properties: a tool window with three pages. Playback follows
 * `player.stats()` every second while the window is open; File is read once per
 * source with ffprobe (`plugin.tools`, aborted when the source changes); System
 * once per opening (`plugin.app.system()` and Node `os`). Closing the window (its
 * Close button or the title bar, `onClose`) stops the polling. Copy and save take
 * the current page's text. The window's binding and the source listener live in a
 * child scope while it is open (lazy activation, player 0.5.21): a closed window
 * reads no player state, so nothing keeps the plugin running while idle.
 */
export function propertiesWindow(plugin: Plugin) {
  const { player } = plugin;
  let prefs: { value: Prefs; save(next: Prefs): void } = { value: { propertiesTab: 0 }, save: (next) => void (prefs.value = next) };
  try {
    prefs = plugin.storage.json<Prefs>("info", { defaults: { propertiesTab: 0 }, validate: isPrefs });
  } catch {
    // no data directory: the tab is remembered for this run only
  }
  const saved = Math.round(Number(prefs.value.propertiesTab));
  let tab = saved >= 0 && saved < TABS ? saved : 0;
  /** `""`, `copy-failed` or `save-failed` (properties.slint words it), with the reason. */
  let status = "";
  let statusDetail = "";
  let stats: PlaybackStats | null = null;
  let file: { sourceId: number; rows: Row[] } | null = null;
  let probe: AbortController | null = null;
  let facts: SystemFacts | null = null;
  let reading: AbortController | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  /** What follows the player while the window is open; `null` while closed. */
  let open: Scope | null = null;
  let sourceId = player.state.sourceId;

  const page = (): Row[] => {
    const state = player.state;
    if (tab === 0) return playbackPage(plugin.i18n, state, stats);
    if (tab === 2) return systemPage(plugin.i18n, plugin.app, facts);
    if (!state.source) return message(plugin.i18n.text("No media open"));
    if (isUrl(state.source)) return remotePage(plugin.i18n, state, stats);
    return file?.sourceId === state.sourceId ? file.rows : message(plugin.i18n.text("Reading…"));
  };

  const propertiesToolWindow = plugin.windows.create("ui/properties.slint");

  async function readStats() {
    try {
      stats = await player.stats();
    } catch {
      // keep the last values; try again next second
    }
  }

  /** The File page of a local source, unless already read (or `again`). The previous read is aborted. */
  async function readFile(again = false) {
    const state: PlayerState = player.state;
    if (!state.source || isUrl(state.source) || (!again && file?.sourceId === state.sourceId)) return;
    probe?.abort();
    const controller = (probe = new AbortController());
    file = null;
    plugin.update(); // Reading…
    let rows: Row[];
    try {
      const info = await stat(state.source).catch(() => {
        throw plugin.i18n.error("File not found");
      });
      const { stdout } = await plugin.tools.run("ffprobe", [...PROBE_ARGS, state.source], { timeoutMs: 10_000, signal: controller.signal });
      let parsed;
      try {
        parsed = JSON.parse(new TextDecoder().decode(stdout));
      } catch {
        throw plugin.i18n.error("Cannot parse the ffprobe output");
      }
      rows = filePage(plugin.i18n, state.source, info, parsed, plugin.i18n.locale);
    } catch (error) {
      if (controller.signal.aborted) return; // a newer read (or closing) took over
      rows = message(probeError(plugin.i18n, error));
    }
    if (probe !== controller) return;
    probe = null;
    file = { sourceId: state.sourceId, rows };
    plugin.update();
  }

  /** The System page, unless already read (or `again`). */
  async function readSystem(again = false) {
    if (reading || (facts && !again)) return;
    const controller = (reading = new AbortController());
    if (again) facts = null;
    plugin.update();
    try {
      const read = await readFacts(plugin.i18n, plugin.app);
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

  /** A source change: the stats and the File page belong to the old one. */
  function sourceChanged(state: PlayerState) {
    if (state.sourceId === sourceId) return false;
    sourceId = state.sourceId;
    stats = null;
    return true;
  }

  /** Opening: bind the window and follow source changes until it closes; the source is read again now. */
  function follow() {
    if (open) return;
    const scope = (open = plugin.scope());
    sourceChanged(player.state);
    scope.add(propertiesToolWindow.bind(() => {
      const rows = page();
      // The label column's width: the host lays out this one label, not every row's.
      return { tabIndex: tab, rows: rows as unknown as Record<string, string | boolean>[], labelSample: widestLabel(plugin.i18n, rows), status, statusDetail, canRefresh: tab !== 0 };
    }));
    scope.player.onChange((state) => {
      if (!sourceChanged(state) || !propertiesToolWindow.opened) return;
      probe?.abort();
      probe = null;
      void readStats();
      load();
    });
  }

  /** Closing: stop polling and reading, and let go of the player's state (the SDK's update pass after a handler then drops what the binding held). */
  async function stop() {
    const following = open;
    open = null;
    clearInterval(timer);
    timer = undefined;
    probe?.abort();
    probe = null;
    reading?.abort();
    reading = null;
    await following?.dispose();
  }

  propertiesToolWindow.on("tabChanged", (index: number) => {
    tab = Math.min(TABS - 1, Math.max(0, Math.round(Number(index))));
    status = "";
    if (prefs.value.propertiesTab !== tab) prefs.save({ propertiesTab: tab });
    load();
  });
  propertiesToolWindow.on("refresh", () => {
    status = "";
    if (tab === 1) void readFile(true);
    if (tab === 2) void readSystem(true);
  });
  propertiesToolWindow.on("copyPage", async () => {
    try {
      await plugin.clipboard.writeText(pageText(plugin.i18n, page()));
      status = "";
      void plugin.osd.show("Playback info copied").catch(() => {});
    } catch (error) {
      console.error(`[info] copy: ${reason(error)}`);
      status = "copy-failed";
    }
  });
  propertiesToolWindow.on("savePage", async () => {
    const text = pageText(plugin.i18n, page()); // what the user saw when asking
    const chosen = await plugin.dialogs.saveFile({ title: "Save As", fileName: saveName(plugin.i18n, player.state.source) });
    if (!chosen) return;
    const temporary = `${chosen}.vestra-${process.pid}.tmp`;
    try {
      await writeFile(temporary, text);
      await rename(temporary, chosen);
      status = "";
      void plugin.osd.show("Saved to {}", [basename(chosen)]).catch(() => {});
    } catch (error) {
      await rm(temporary, { force: true });
      status = "save-failed";
      statusDetail = reason(error); // the system's message
    }
  });
  propertiesToolWindow.on("closeWindow", async () => {
    // Stop at once, so that no show() lands between stopping and closing.
    const stopping = stop();
    await propertiesToolWindow.close();
    await stopping;
  });
  // Closed with the title bar: stop the 1-second stats polling and any read (reopening starts them again).
  propertiesToolWindow.onClose(stop);

  plugin.onDispose(stop);

  /** Ctrl+F1: open, or bring the open window to the front. */
  return async function show() {
    follow();
    if (!timer) {
      // Claimed before the first read: an overlapping show() (two quick Ctrl+F1) starts no second interval.
      timer = setInterval(() => void readStats().then(() => plugin.update()), 1000);
      await readStats();
    }
    load();
    try {
      await propertiesToolWindow.show();
    } catch (error) {
      if (!propertiesToolWindow.opened) await stop(); // it did not open: nothing to follow or poll
      throw error;
    }
  };
}
