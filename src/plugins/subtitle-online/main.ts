import { access, mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";
import type { Plugin, PlayerState } from "../../sdk/plugin/index.ts";
import { convert } from "./formats.ts";
import { archiveKind, keywords, movieHash, unzipSubtitles, urlTitle } from "./media.ts";
import { DEFAULTS, fileLanguage, isSettingsFile, parseLanguages, parseSettings, rank, row, trackTitle } from "./model.ts";
import type { Settings } from "./model.ts";
import { Secrets } from "./secrets.ts";
import { Assrt, OpenSubtitles, SERVICE_NAMES, ServiceError } from "./services.ts";
import type { Credentials, Query, Result, Service, ServiceId } from "./services.ts";

const PRIVACY = "搜索会把文件名关键字与文件哈希发送给所选服务。";
const NO_SERVICE = "请先在「服务」中为 OpenSubtitles 填写 API 密钥（在 opensubtitles.com 免费申请），或启用并填写射手网(伪) token。";
const NO_MEDIA = "没有可载入字幕的媒体";
const SOURCE_CHANGED = "媒体已切换，未载入字幕。";
const HELP: Record<ServiceId, string> = {
  opensubtitles:
    "在 opensubtitles.com 注册账号后，到「API consumers」页面免费申请 API 密钥。用户名和密码可选，登录后每日下载次数更多。",
  assrt: "在 assrt.net 注册后，在用户中心取得 API token。服务有调用频率限制。",
};
const isRemote = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source) && !/^file:\/\//i.test(source);
const localPath = (source: string) => (/^file:\/\//i.test(source) ? decodeURIComponent(new URL(source).pathname) : source);

/** One short Chinese sentence for a failure. */
function errorText(e: unknown) {
  if (e instanceof ServiceError) return e.message;
  const error = e as { code?: string; name?: string; message?: string };
  if (error?.name === "AbortError" || error?.code === "aborted") return "已取消。";
  switch (error?.code) {
    case undefined:
      return error?.message ?? String(e); // our own messages (formats.ts) are Chinese
    case "stale":
      return SOURCE_CHANGED;
    case "invalid":
      return "播放器拒绝了该字幕（文件过大或内容无效）。";
    case "unavailable":
    case "unsupported":
      return "当前媒体不能载入字幕。";
    default:
      return "字幕载入失败。";
  }
}
const aborted = (e: unknown) =>
  (e instanceof ServiceError && e.kind === "aborted") || (e as { name?: string })?.name === "AbortError";

/** 在线字幕 (D16): search OpenSubtitles / assrt, download, load as this plugin's tracks. */
export default async function activate(plugin: Plugin) {
  const { player, subtitles } = plugin;
  const say = (text: string) => void plugin.osd.show(text).catch(() => {});
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
  const secrets = new Secrets(directory, process.env.VESTRA_SUBTITLE_ONLINE_KEYCHAIN === "off" ? false : undefined);
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
  const serviceState: Partial<Record<ServiceId, string>> = {};

  // ------------------------------------------------------------ download window state
  const download = plugin.windows.create("ui/download.slint", { id: "subtitle-download" });
  const settingsWindow = plugin.windows.create("ui/settings.slint", { id: "subtitle-online-settings" });
  const serviceWindow = plugin.windows.create("ui/service.slint", { id: "subtitle-online-service" });
  const chooser = plugin.windows.create("ui/choose.slint", { id: "subtitle-online-choose" });
  let results: Result[] = [];
  const checked = new Set<string>();
  let selected = -1;
  let more = false;
  let status = "";
  let settingsStatus = "";
  let serviceStatus = "";
  let busy = false;
  let title = "";
  /** The last search, for 再次搜索 and 更多. */
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
    privacyText: `${PRIVACY}同意后不再提示。`,
    canLoad: canLoad(),
    canMore: more && last !== null,
    hasChecked: results.some((r) => checked.has(r.key)),
    statusText: status || (!canLoad() ? `${NO_MEDIA}，可按标题搜索。` : ""),
  }));
  download.set({ titleText: "" });
  settingsWindow.bind(() => ({
    services: settings.services.map((s) => ({
      name: SERVICE_NAMES[s.id],
      enabled: s.enabled,
      state: serviceState[s.id] ?? (configured(s.id) ? (s.id === "opensubtitles" ? "已填写密钥" : "已填写 token") : s.id === "opensubtitles" ? "未填写密钥" : "未填写 token"),
    })),
    autoIndex: settings.autoSearch === "no-subtitles" ? 1 : 0,
    afterIndex: settings.afterSearch === "load-best" ? 1 : 0,
    exactOnly: settings.exactOnly,
    saveIndex: settings.saveLocation === "data" ? 1 : 0,
    noteText: `${PRIVACY}密钥与密码存放在：${secrets.location}。`,
    statusText: settingsStatus,
    busy,
  }));
  serviceWindow.bind(() => ({
    serviceName: SERVICE_NAMES[editing],
    hasAccount: editing === "opensubtitles",
    keyLabel: editing === "opensubtitles" ? "API 密钥" : "token",
    helpText: `${HELP[editing]}存放在：${secrets.location}。`,
    statusText: serviceStatus,
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
        status = errorText(e);
        if (e instanceof ServiceError && e.kind === "quota") {
          say(e.message);
          if (e.reset) status = `${e.message}。将于 ${e.reset} 重置`;
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
  function abortOperation(text: string | null) {
    if (!operation) return;
    operation.controller.abort();
    operation = null;
    busy = false;
    if (text !== null) status = text;
    plugin.update();
  }
  // Closing the download window while a request runs aborts it (the window has no close event).
  const watch = setInterval(() => {
    if (operation?.window && !download.opened && !chooser.opened) abortOperation("");
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
      status = NO_SERVICE;
      settingsStatus = NO_SERVICE;
      void showSettings();
      return [];
    }
    if (!settings.privacyAccepted) {
      privacyPending = () => void search(query, append, fromWindow);
      status = PRIVACY;
      plugin.update();
      return [];
    }
    if (!query.title && !query.hash) {
      status = "请输入标题后再搜索。";
      plugin.update();
      return [];
    }
    const source = player.state.sourceId;
    return (
      (await begin(fromWindow, async (signal) => {
        status = "正在搜索…";
        plugin.update();
        const outcomes = await Promise.allSettled(active().map((s) => s.search(query, signal)));
        if (signal.aborted) return [];
        const errors: string[] = [];
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
        const head = results.length ? `找到 ${results.length} 条字幕` : errors.length ? "" : "没有找到字幕，可修改标题后再搜索";
        status = [head, ...errors].filter(Boolean).join("；");
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
      else if (kind) throw new Error("此压缩格式暂不支持（rar、7z），请下载 zip 或单个字幕文件。");
      else files.push(file);
    }
    if (!files.length) throw new Error("压缩包里没有字幕文件。");
    let index = 0;
    if (files.length > 1) {
      const picked = await choose(files.map((f) => f.name), signal);
      if (picked === null || !files[picked]) {
        const error = new Error("已取消。");
        error.name = "AbortError";
        throw error;
      }
      index = picked;
    }
    return { file: files[index]!, remaining: got.remaining };
  }
  /** Where a downloaded file goes: `<视频名>.<语言>.<扩展名>` beside the video, else the data folder. */
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
    if (player.state.sourceId !== source || !canLoad()) throw new Error(player.state.source ? SOURCE_CHANGED : NO_MEDIA);
    const { file } = await fetchFile(result, signal);
    const fps = result.fps ?? (await player.stats().then((s) => s.video?.fps ?? null, () => null));
    const converted = convert(file.bytes, { fallback: "gb18030", fps });
    for (const [i, part] of converted.tracks.entries()) {
      const s = await subtitles.refresh();
      if (s.sourceId !== source || player.state.sourceId !== source) throw new Error(SOURCE_CHANGED);
      const name = part.language ? `${trackTitle(result)} · ${part.language}`.slice(0, 200) : trackTitle(result);
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
      status = "正在下载字幕…";
      plugin.update();
      const { saved, notes } = await loadResult(result, "primary", signal, source);
      const text = `打开字幕：${trackTitle(result)}`;
      status = [text, ...notes, saved.fallback ? "字幕已载入，但无法保存到视频所在文件夹" : saved.path ? `已保存到 ${saved.path}` : ""]
        .filter(Boolean)
        .join("。");
      say(saved.fallback ? "字幕已载入，但无法保存到视频所在文件夹" : text);
    });
  }
  async function addChecked() {
    const picks = checkedResults();
    const source = last?.source ?? player.state.sourceId;
    await begin(true, async (signal) => {
      let added = 0;
      for (const result of picks) {
        status = `正在下载字幕（${added + 1}/${picks.length}）…`;
        plugin.update();
        await loadResult(result, "none", signal, source);
        added++;
      }
      status = `已添加 ${added} 条字幕`;
      say(status);
    });
  }
  async function saveChecked() {
    const picks = checkedResults();
    await begin(true, async (signal) => {
      const paths: string[] = [];
      for (const result of picks) {
        status = `正在下载字幕（${paths.length + 1}/${picks.length}）…`;
        plugin.update();
        const { file } = await fetchFile(result, signal);
        const { path } = await saveCopy(result, file);
        if (path) paths.push(path);
      }
      status = paths.length ? `已保存 ${paths.length} 个字幕到 ${dirname(paths[0]!)}` : "没有可保存的位置。";
    });
  }
  async function saveAs() {
    const result = selectedResult();
    if (!result) return;
    await begin(true, async (signal) => {
      status = "正在下载字幕…";
      plugin.update();
      const { file } = await fetchFile(result, signal);
      const path = await plugin.dialogs.saveFile({ title: "另存字幕", fileName: file.name });
      if (!path || signal.aborted) return void (status = "未保存。");
      await writeFile(path, file.bytes);
      status = `已保存到 ${path}`;
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
    if (!canLoad()) return void (status = NO_MEDIA);
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
  download.on("cancel", () => abortOperation("已取消。"));

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
    serviceStatus = "";
    serviceWindow.set({
      keyText: editing === "opensubtitles" ? credentials.apiKey : credentials.token,
      userText: credentials.username,
      passwordText: credentials.password,
    });
    await serviceWindow.show();
  });
  async function testService(id: ServiceId, c: Credentials) {
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
    settingsStatus = `正在测试 ${SERVICE_NAMES[id]}…`;
    plugin.update();
    const text = await testService(id, credentials);
    serviceState[id] = text;
    settingsStatus = `${SERVICE_NAMES[id]}：${text}`;
    plugin.update();
  });
  settingsWindow.on("changeLanguages", (text: string) => {
    const languages = parseLanguages(String(text));
    if (!languages) return void (settingsStatus = "首选语言写法如 zh-CN, zh-TW, en（最多 5 个）");
    settingsStatus = "";
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
      serviceStatus = `已保存（${secrets.location}）`;
    } catch (e) {
      serviceStatus = `无法保存：${(e as Error).message}`;
    }
    plugin.update();
  });
  serviceWindow.on("test", async (key: string, user: string, password: string) => {
    serviceStatus = "正在测试…";
    plugin.update();
    serviceStatus = await testService(editing, entered(key, user, password));
    plugin.update();
  });

  async function showSettings() {
    settingsWindow.set({ languagesText: settings.languages.join(", "), excludeText: settings.exclude, selectedService });
    await settingsWindow.show();
  }
  async function showDownload() {
    if (!active().length) {
      settingsStatus = NO_SERVICE;
      return showSettings();
    }
    status = "";
    await download.show();
    if (canLoad()) await searchCurrent();
    else {
      results = [];
      status = `${NO_MEDIA}，可按标题搜索。`;
      plugin.update();
    }
  }

  // ------------------------------------------------------------ menu
  plugin.commands.add({
    id: "search", path: ["字幕", "在线字幕", "搜索与下载字幕…"], order: 360, group: "subtitle-online",
    run: showDownload,
  });
  plugin.commands.add({
    id: "settings", path: ["字幕", "在线字幕", "在线字幕设置…"], order: 361, group: "subtitle-online",
    run: showSettings,
  });

  // ------------------------------------------------------------ source changes, automatic search
  let seenSource = player.state.sourceId;
  let autoDone = player.state.sourceId; // the source open at start is not searched automatically
  player.onChange((p) => {
    if (p.sourceId !== seenSource) {
      seenSource = p.sourceId;
      // Requests for the old source must not load into the new one.
      abortOperation(operation ? SOURCE_CHANGED : null);
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
  /** 打开没有字幕的视频时: search in the background; act only on a hash match in the first language. */
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
      say(`已自动载入在线字幕：${trackTitle(best)}`);
    });
  }

  plugin.onError((e) => {
    status = errorText(e);
    plugin.update();
  });
  plugin.onDispose(() => {
    clearInterval(watch);
    operation?.controller.abort();
  });
  await loadCredentials();
}
