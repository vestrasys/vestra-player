import { access, mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";
import type { Plugin, PlayerState, TextArg } from "@vestra-player/sdk";
import { convert } from "./formats.ts";
import { archiveKind, keywords, movieHash, unzipSubtitles, urlTitle } from "./media.ts";
import { DEFAULTS, fileLanguage, isSettingsFile, parseLanguages, parseSettings, rank, row, trackTitle } from "./model.ts";
import type { Settings } from "./model.ts";
import { Secrets } from "./secrets.ts";
import { Failure, NONE, STATUS, forSlint, line, lineForSlint, list, raw, sentences, serviceResult } from "./messages.ts";
import type { Line, Status, StatusId } from "./messages.ts";
import { Assrt, OpenSubtitles, ServiceError } from "./services.ts";
import type { Credentials, Query, Result, Service, ServiceId } from "./services.ts";

const isRemote = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source) && !/^file:\/\//i.test(source);
const localPath = (source: string) => (/^file:\/\//i.test(source) ? decodeURIComponent(new URL(source).pathname) : source);

/** The line for a failure. */
function errorText(e: unknown): Line {
  if (e instanceof ServiceError || e instanceof Failure) return e.status;
  const error = e as { code?: string; name?: string; message?: string };
  if (error?.name === "AbortError" || error?.code === "aborted") return line("cancelled");
  switch (error?.code) {
    case undefined:
      return raw(error?.message ?? String(e)); // not ours: the system's own words
    case "stale":
      return line("sourceChanged");
    case "invalid":
      return line("invalid");
    case "unavailable":
    case "unsupported":
      return line("unsupported");
    default:
      return line("failed");
  }
}
const one = (l: Line): Status => sentences(l);
const aborted = (e: unknown) =>
  (e instanceof ServiceError && e.kind === "aborted") || (e as { name?: string })?.name === "AbortError";

/** Online Subtitles (D16): search OpenSubtitles / assrt, download, load as this plugin's tracks. */
export default async function activate(plugin: Plugin) {
  const { player, subtitles } = plugin;
  const { text } = plugin.i18n;
  const say = (id: StatusId, args: TextArg[] = []) => void plugin.osd.show(STATUS[id], args).catch(() => {});
  let directory: string | null = null;
  try {
    directory = plugin.dataDirectory;
  } catch {
    // No data directory: settings last for this run; downloads are only loaded.
  }

  // ------------------------------------------------------------ settings and keys
  const store = (() => {
    try {
      return plugin.storage.json<Record<string, unknown>>("settings", { defaults: { ...DEFAULTS }, validate: isSettingsFile });
    } catch {
      let value: Record<string, unknown> = { ...DEFAULTS };
      return { get value() { return value; }, save(next: Record<string, unknown>) { value = next; } };
    }
  })();
  let settings = parseSettings(store.value);
  const saveSettings = (patch: Partial<Settings>) => {
    settings = { ...settings, ...patch };
    store.save(settings);
    plugin.update();
  };
  const { os } = await plugin.app.system().catch(() => ({ os: "other" as const }));
  const secrets = new Secrets(directory, plugin.secrets, os);
  let credentials: Credentials = { apiKey: "", username: settings.username, password: "", token: "" };
  async function loadCredentials() {
    const [apiKey, password, token] = await Promise.all([
      secrets.get("opensubtitles-api-key"), secrets.get("opensubtitles-password"), secrets.get("assrt-token"),
    ]);
    credentials = { apiKey, password, token, username: settings.username };
  }
  const services: Record<ServiceId, Service> = {
    opensubtitles: new OpenSubtitles(() => credentials),
    assrt: new Assrt(() => credentials),
  };
  const configured = (id: ServiceId, c = credentials) => (id === "opensubtitles" ? !!c.apiKey : !!c.token);
  /** Enabled services that have a key, in the user's order. Nothing else is ever asked. */
  const active = () => settings.services.filter((s) => s.enabled && configured(s.id)).map((s) => services[s.id]);
  const serviceState: Partial<Record<ServiceId, Line>> = {};

  // ------------------------------------------------------------ download window state
  const download = plugin.windows.create("ui/download.slint", { id: "subtitle-download" });
  const settingsWindow = plugin.windows.create("ui/settings.slint", { id: "subtitle-online-settings" });
  const serviceWindow = plugin.windows.create("ui/service.slint", { id: "subtitle-online-service" });
  const chooser = plugin.windows.create("ui/choose.slint", { id: "subtitle-online-choose" });
  let results: Result[] = [];
  const checked = new Set<string>();
  let selected = -1;
  let more = false;
  let status: Status = NONE;
  let settingsStatus: Status = NONE;
  let serviceStatus: Status = NONE;
  let busy = false;
  let title = "";
  /** The last search, for Search Again and More. */
  let last: { query: Query; source: number } | null = null;
  let privacyPending: (() => void) | null = null;
  /** The running window operation; closing the window or a new source aborts it. */
  let operation: { controller: AbortController; window: boolean } | null = null;
  let editing: ServiceId = "opensubtitles";

  const canLoad = () => {
    const p = player.state;
    return !!p.source && p.kind === "media" && p.phase === "ready";
  };
  download.bind(() => ({
    rows: results.map((r) => row(r, checked.has(r.key))),
    selectedIndex: selected,
    busy,
    privacyPending: privacyPending !== null,
    canLoad: canLoad(),
    canMore: more && last !== null,
    hasChecked: results.some((r) => checked.has(r.key)),
    status: forSlint(status.parts.length || canLoad() ? status : one(line("noMediaSearchTitle"))),
  }));
  download.set({ titleText: "" });
  settingsWindow.bind(() => ({
    services: settings.services.map((s) => ({
      service: s.id,
      enabled: s.enabled,
      state: lineForSlint(
        serviceState[s.id] ??
          line(configured(s.id) ? (s.id === "opensubtitles" ? "keySet" : "tokenSet") : s.id === "opensubtitles" ? "keyMissing" : "tokenMissing"),
      ),
    })),
    autoIndex: settings.autoSearch === "no-subtitles" ? 1 : 0,
    afterIndex: settings.afterSearch === "load-best" ? 1 : 0,
    exactOnly: settings.exactOnly,
    saveIndex: settings.saveLocation === "data" ? 1 : 0,
    location: secrets.location,
    status: forSlint(settingsStatus),
    busy,
  }));
  serviceWindow.bind(() => ({
    service: editing,
    location: secrets.location,
    status: forSlint(serviceStatus),
    busy,
  }));

  /** Run one window operation at a time; a new one replaces the old. */
  async function begin<T>(fromWindow: boolean, work: (signal: AbortSignal) => Promise<T>): Promise<T | undefined> {
    operation?.controller.abort();
    const mine = { controller: new AbortController(), window: fromWindow };
    operation = mine;
    busy = true;
    plugin.update();
    try {
      return await work(mine.controller.signal);
    } catch (e) {
      if (!aborted(e)) {
        status = one(errorText(e));
        if (e instanceof ServiceError && e.kind === "quota") {
          if (e.status.id !== "raw" && e.status.id !== "") say(e.status.id, e.status.args);
          if (e.reset) status = sentences(e.status, line("resetAt", [e.reset]));
        }
      }
      return undefined;
    } finally {
      if (operation === mine) {
        operation = null;
        busy = false;
      }
      plugin.update();
    }
  }
  function abortOperation(next: Status | null) {
    if (!operation) return;
    operation.controller.abort();
    operation = null;
    busy = false;
    if (next !== null) status = next;
    plugin.update();
  }
  // Closing the download window while a request runs aborts it (the window has no close event).
  const watch = setInterval(() => {
    if (operation?.window && !download.opened && !chooser.opened) abortOperation(NONE);
  }, 500);
  watch.unref();

  // ------------------------------------------------------------ search
  /** Keywords and hash for the current source. */
  async function mediaQuery(p: PlayerState) {
    if (!p.source) return { title: "", hash: null as string | null };
    if (isRemote(p.source)) return { title: urlTitle(p.source), hash: null };
    const path = localPath(p.source);
    return { title: keywords(basename(path)), hash: (await movieHash(path))?.hash ?? null };
  }
  async function search(query: Query, append: boolean, fromWindow = true) {
    if (!active().length) {
      status = one(line("noService"));
      settingsStatus = one(line("noService"));
      void showSettings();
      return [];
    }
    if (!settings.privacyAccepted) {
      privacyPending = () => void search(query, append, fromWindow);
      status = one(line("privacy"));
      plugin.update();
      return [];
    }
    if (!query.title && !query.hash) {
      status = one(line("enterTitle"));
      plugin.update();
      return [];
    }
    const source = player.state.sourceId;
    return (
      (await begin(fromWindow, async (signal) => {
        status = one(line("searching"));
        plugin.update();
        const outcomes = await Promise.allSettled(active().map((s) => s.search(query, signal)));
        if (signal.aborted) return [];
        const errors: Line[] = [];
        const found: Result[] = [];
        more = false;
        for (const outcome of outcomes) {
          if (outcome.status === "fulfilled") {
            found.push(...outcome.value.results);
            more ||= outcome.value.more;
          } else if (!aborted(outcome.reason)) errors.push(errorText(outcome.reason));
        }
        last = { query, source };
        results = rank(append ? [...results, ...found] : found, settings);
        if (!append) {
          checked.clear();
          selected = results.length ? 0 : -1;
        }
        const head = results.length ? [line("found", [results.length])] : errors.length ? [] : [line("notFound")];
        status = list(...head, ...errors);
        return results;
      })) ?? []
    );
  }
  async function searchCurrent(fromWindow = true) {
    const p = player.state;
    const media = await mediaQuery(p);
    title = media.title;
    download.set({ titleText: title });
    return search({ title: media.title, hash: media.hash, languages: settings.languages, page: 1 }, false, fromWindow);
  }

  // ------------------------------------------------------------ download, load, save
  /** Ask which file of an archive to use; `null` when cancelled. */
  let choosing: ((index: number | null) => void) | null = null;
  chooser.on("choose", (index: number) => {
    choosing?.(Number.isInteger(index) ? index : null);
    void chooser.close();
  });
  chooser.on("dismiss", () => {
    choosing?.(null);
    void chooser.close();
  });
  async function choose(names: string[], signal: AbortSignal) {
    chooser.set({ names, selectedIndex: 0 });
    await chooser.show();
    return new Promise<number | null>((resolve) => {
      choosing = (index) => {
        choosing = null;
        resolve(index);
      };
      signal.addEventListener("abort", () => choosing?.(null), { once: true });
    });
  }
  /** The subtitle file of a result: archives are opened, several files are offered to choose from. */
  async function fetchFile(result: Result, signal: AbortSignal) {
    const got = await services[result.service].download(result, signal);
    const files: { name: string; bytes: Buffer }[] = [];
    for (const file of got.files) {
      const kind = archiveKind(file.bytes);
      if (kind === "zip") files.push(...unzipSubtitles(file.bytes));
      else if (kind) throw new Failure("archiveUnsupported");
      else files.push(file);
    }
    if (!files.length) throw new Failure("archiveEmpty");
    let index = 0;
    if (files.length > 1) {
      const picked = await choose(files.map((f) => f.name), signal);
      if (picked === null || !files[picked]) {
        const error = new Error("Cancelled");
        error.name = "AbortError";
        throw error;
      }
      index = picked;
    }
    return { file: files[index]!, remaining: got.remaining };
  }
  /** Where a downloaded file goes: `<video name>.<language>.<extension>` beside the video, else the data folder. */
  async function saveCopy(result: Result, file: { name: string; bytes: Buffer }, folderOverride?: string) {
    const p = player.state;
    const ext = extname(file.name).toLowerCase() || ".srt";
    const local = p.source && !isRemote(p.source) ? localPath(p.source) : null;
    const stem = local ? `${basename(local, extname(local))}.${fileLanguage(result.language)}` : basename(file.name, extname(file.name));
    const unique = async (folder: string) => {
      for (let n = 0; n < 1000; n++) {
        const path = join(folder, `${stem}${n ? ` (${n})` : ""}${ext}`);
        if (await access(path).then(() => false, () => true)) return path;
      }
      throw new Error("too many files");
    };
    const write = async (folder: string) => {
      await mkdir(folder, { recursive: true });
      const path = await unique(folder);
      await writeFile(path, file.bytes, { flag: "wx" });
      return path;
    };
    const besideVideo = !folderOverride && settings.saveLocation === "video" && local;
    if (besideVideo) {
      try {
        return { path: await write(dirname(local)), fallback: false };
      } catch {
        // not writable: the data folder below
      }
    }
    if (!directory && !folderOverride) return { path: null, fallback: !!besideVideo };
    return { path: await write(folderOverride ?? join(directory!, "downloads")), fallback: !!besideVideo };
  }
  /** Download one result and load it; the source must still be the one searched for. */
  async function loadResult(result: Result, select: "primary" | "none", signal: AbortSignal, source: number) {
    if (player.state.sourceId !== source || !canLoad()) throw new Failure(player.state.source ? "sourceChanged" : "noMedia");
    const { file } = await fetchFile(result, signal);
    const fps = result.fps ?? (await player.stats().then((s) => s.video?.fps ?? null, () => null));
    const converted = convert(file.bytes, { fallback: "gb18030", fps });
    for (const [i, part] of converted.tracks.entries()) {
      const s = await subtitles.refresh();
      if (s.sourceId !== source || player.state.sourceId !== source) throw new Failure("sourceChanged");
      const title = trackTitle(text, result);
      const name = part.language ? text("{} · {}", [title, part.language.slice(0, 20)]) : title;
      await subtitles.load(part.bytes, { format: part.format, title: name, select: i === 0 ? select : "none", ifCurrent: s, signal });
    }
    // The track uses the downloaded bytes; saving a copy is a convenience.
    const saved = await saveCopy(result, file).catch(() => ({ path: null, fallback: true }));
    return { saved, notes: converted.notes };
  }

  const selectedResult = () => results[selected] ?? null;
  const checkedResults = () => results.filter((r) => checked.has(r.key));
  async function openResult(result: Result) {
    const source = last?.source ?? player.state.sourceId;
    await begin(true, async (signal) => {
      status = one(line("downloading"));
      plugin.update();
      const { saved, notes } = await loadResult(result, "primary", signal, source);
      const where = saved.fallback ? [line("notSavedBeside")] : saved.path ? [line("savedTo", [saved.path])] : [];
      const title = trackTitle(text, result);
      status = sentences(line("opened", [title]), ...notes.map((note) => line(note)), ...where);
      if (saved.fallback) say("notSavedBeside");
      else say("opened", [title]);
    });
  }
  async function addChecked() {
    const picks = checkedResults();
    const source = last?.source ?? player.state.sourceId;
    await begin(true, async (signal) => {
      let added = 0;
      for (const result of picks) {
        status = one(line("downloadingOf", [added + 1, picks.length]));
        plugin.update();
        await loadResult(result, "none", signal, source);
        added++;
      }
      status = one(line("added", [added]));
      say("added", [added]);
    });
  }
  async function saveChecked() {
    const picks = checkedResults();
    await begin(true, async (signal) => {
      const paths: string[] = [];
      for (const result of picks) {
        status = one(line("downloadingOf", [paths.length + 1, picks.length]));
        plugin.update();
        const { file } = await fetchFile(result, signal);
        const { path } = await saveCopy(result, file);
        if (path) paths.push(path);
      }
      status = one(paths.length ? line("savedCount", [paths.length, dirname(paths[0]!)]) : line("nowhereToSave"));
    });
  }
  async function saveAs() {
    const result = selectedResult();
    if (!result) return;
    await begin(true, async (signal) => {
      status = one(line("downloading"));
      plugin.update();
      const { file } = await fetchFile(result, signal);
      const path = await plugin.dialogs.saveFile({ title: "Save Subtitle As", fileName: file.name });
      if (!path || signal.aborted) return void (status = one(line("notSaved")));
      await writeFile(path, file.bytes);
      status = one(line("savedTo", [path]));
    });
  }

  // ------------------------------------------------------------ window callbacks
  download.on("searchTitle", (text: string) => {
    title = String(text).trim();
    void search({ title, hash: null, languages: settings.languages, page: 1 }, false);
  });
  download.on("searchAgain", () => {
    if (last) void search({ ...last.query, page: 1 }, false);
    else void searchCurrent();
  });
  download.on("acceptPrivacy", () => {
    saveSettings({ privacyAccepted: true });
    const next = privacyPending;
    privacyPending = null;
    next?.();
  });
  download.on("toggleRow", (index: number, on: boolean) => {
    const result = results[index];
    if (!result) return;
    if (on === true) checked.add(result.key);
    else checked.delete(result.key);
  });
  download.on("selectRow", (index: number) => {
    if (results[index]) selected = index;
  });
  download.on("openRow", (index: number) => {
    const result = results[index];
    if (!result || busy) return;
    selected = index;
    if (!canLoad()) return void (status = one(line("noMedia")));
    void openResult(result);
  });
  download.on("openSelected", () => {
    const result = selectedResult();
    if (result && !busy) void openResult(result);
  });
  download.on("addChecked", () => void (busy || addChecked()));
  download.on("saveChecked", () => void (busy || saveChecked()));
  download.on("saveAs", () => void (busy || saveAs()));
  download.on("more", () => {
    if (last && more && !busy) void search({ ...last.query, page: last.query.page + 1 }, true);
  });
  download.on("openSettings", () => void showSettings());
  download.on("cancel", () => abortOperation(one(line("cancelled"))));

  settingsWindow.on("toggleService", (index: number, on: boolean) => {
    const services = settings.services.map((s, i) => (i === index ? { ...s, enabled: on === true } : s));
    saveSettings({ services });
  });
  let selectedService = 0;
  settingsWindow.on("selectService", (index: number) => {
    if (settings.services[index]) selectedService = index;
  });
  settingsWindow.on("moveService", (direction: number) => {
    const index = selectedService;
    const to = index + Math.sign(direction);
    if (!settings.services[index] || !settings.services[to]) return;
    const services = [...settings.services];
    [services[index], services[to]] = [services[to]!, services[index]!];
    selectedService = to;
    saveSettings({ services });
    settingsWindow.set({ selectedService: to });
  });
  const currentService = (): ServiceId => settings.services[selectedService]?.id ?? "opensubtitles";
  settingsWindow.on("serviceSettings", async () => {
    editing = currentService();
    serviceStatus = NONE;
    serviceWindow.set({
      keyText: editing === "opensubtitles" ? credentials.apiKey : credentials.token,
      userText: credentials.username,
      passwordText: credentials.password,
    });
    await serviceWindow.show();
  });
  async function testService(id: ServiceId, c: Credentials): Promise<Line> {
    const service = id === "opensubtitles" ? new OpenSubtitles(() => c) : new Assrt(() => c);
    const controller = new AbortController();
    try {
      return await service.test(controller.signal);
    } catch (e) {
      return errorText(e);
    }
  }
  settingsWindow.on("testService", async () => {
    const id = currentService();
    settingsStatus = one(line(`testing.${id}`));
    plugin.update();
    const result = await testService(id, credentials);
    serviceState[id] = result;
    settingsStatus = serviceResult(id, result);
    plugin.update();
  });
  settingsWindow.on("changeLanguages", (text: string) => {
    const languages = parseLanguages(String(text));
    if (!languages) return void (settingsStatus = one(line("languagesHint")));
    settingsStatus = NONE;
    saveSettings({ languages });
  });
  settingsWindow.on("changeAuto", (index: number) => saveSettings({ autoSearch: index === 1 ? "no-subtitles" : "off" }));
  settingsWindow.on("changeAfter", (index: number) => saveSettings({ afterSearch: index === 1 ? "load-best" : "show" }));
  settingsWindow.on("changeExact", (on: boolean) => saveSettings({ exactOnly: on === true }));
  settingsWindow.on("changeExclude", (text: string) => saveSettings({ exclude: String(text).slice(0, 200) }));
  settingsWindow.on("changeSave", (index: number) => saveSettings({ saveLocation: index === 1 ? "data" : "video" }));

  const entered = (key: string, user: string, password: string): Credentials =>
    editing === "opensubtitles"
      ? { ...credentials, apiKey: String(key).trim(), username: String(user).trim(), password: String(password) }
      : { ...credentials, token: String(key).trim() };
  serviceWindow.on("save", async (key: string, user: string, password: string) => {
    const next = entered(key, user, password);
    try {
      if (editing === "opensubtitles") {
        await secrets.set("opensubtitles-api-key", next.apiKey);
        await secrets.set("opensubtitles-password", next.password);
        saveSettings({ username: next.username });
      } else await secrets.set("assrt-token", next.token);
      credentials = next;
      delete serviceState[editing];
      serviceStatus = one(line("saved", [secrets.location.kind, secrets.location.value]));
    } catch (e) {
      serviceStatus = one(line("saveFailed", [(e as Error).message]));
    }
    plugin.update();
  });
  serviceWindow.on("test", async (key: string, user: string, password: string) => {
    serviceStatus = one(line("testing"));
    plugin.update();
    serviceStatus = one(await testService(editing, entered(key, user, password)));
    plugin.update();
  });

  async function showSettings() {
    settingsWindow.set({ languagesText: settings.languages.join(", "), excludeText: settings.exclude, selectedService });
    await settingsWindow.show();
  }
  async function showDownload() {
    if (!active().length) {
      settingsStatus = one(line("noService"));
      return showSettings();
    }
    status = NONE;
    await download.show();
    if (canLoad()) await searchCurrent();
    else {
      results = [];
      status = one(line("noMediaSearchTitle"));
      plugin.update();
    }
  }

  // ------------------------------------------------------------ menu
  plugin.commands.add({
    id: "search", path: ["@subtitles", "Online Subtitles", "Search and Download Subtitles…"], order: 360, group: "subtitle-online",
    run: showDownload,
  });
  plugin.commands.add({
    id: "settings", path: ["@subtitles", "Online Subtitles", "Online Subtitle Settings…"], order: 361, group: "subtitle-online",
    run: showSettings,
  });

  // ------------------------------------------------------------ source changes, automatic search
  let seenSource = player.state.sourceId;
  let autoDone = player.state.sourceId; // the source open at start is not searched automatically
  player.onChange((p) => {
    if (p.sourceId !== seenSource) {
      seenSource = p.sourceId;
      // Requests for the old source must not load into the new one.
      abortOperation(operation ? one(line("sourceChanged")) : null);
      results = [];
      checked.clear();
      selected = -1;
      last = null;
      more = false;
    }
    if (p.sourceId !== autoDone && p.phase === "ready") {
      autoDone = p.sourceId;
      void autoSearch(p);
    }
  });
  /** "When opening a video without subtitles": search in the background; act only on a hash match in the first language. */
  async function autoSearch(p: PlayerState) {
    if (settings.autoSearch !== "no-subtitles" || !settings.privacyAccepted || !active().length) return;
    if (p.kind !== "media" || !p.source || isRemote(p.source)) return;
    const state = await subtitles.refresh().catch(() => null);
    if (!state || state.sourceId !== p.sourceId || state.tracks.some((t) => t.text)) return;
    const found = await searchCurrent(false);
    const best = found[0];
    const first = settings.languages[0]?.toLowerCase();
    if (!best?.hashMatch || best.language.toLowerCase() !== first || player.state.sourceId !== p.sourceId) return;
    if (settings.afterSearch === "show") return void download.show();
    await begin(false, async (signal) => {
      await loadResult(best, "primary", signal, p.sourceId);
      say("autoLoaded", [trackTitle(text, best)]);
    });
  }

  plugin.onError((e) => {
    status = one(errorText(e));
    plugin.update();
  });
  plugin.onDispose(() => {
    clearInterval(watch);
    operation?.controller.abort();
  });
  await loadCredentials();
}
