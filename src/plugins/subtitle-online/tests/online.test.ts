// Orchestration against the SDK test host with a fake `fetch`; no real network or secret store.
import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import { deflateRawSync } from "node:zlib";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const SRT = "1\n00:00:01,000 --> 00:00:02,000\n你好\n";

type Call = { url: string; method: string; headers: Record<string, string>; body: string | null };
/** Replace `fetch` for one test: `routes` answers by URL prefix; every call is recorded. */
function fakeFetch(routes: Record<string, (call: Call) => Response | Promise<Response>>) {
  const calls: Call[] = [];
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: string | URL, init: RequestInit = {}) => {
    const url = String(input);
    const call = { url, method: init.method ?? "GET", headers: { ...(init.headers as Record<string, string>) }, body: (init.body as string) ?? null };
    calls.push(call);
    const route = Object.keys(routes).filter((prefix) => url.startsWith(prefix)).sort((a, b) => b.length - a.length)[0];
    if (!route) throw new TypeError("fetch failed");
    const answer = routes[route]!(call);
    return new Promise<Response>((resolve, reject) => {
      init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), { once: true });
      Promise.resolve(answer).then(resolve, reject);
    });
  }) as typeof fetch;
  return { calls, restore: () => void (globalThis.fetch = original) };
}
const jsonResponse = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
const OS = "https://api.opensubtitles.com/api/v1";
function osResult(fileId: number, language: string, release: string, extra: Record<string, unknown> = {}) {
  return {
    id: String(fileId), type: "subtitle",
    attributes: { language, release, download_count: 100 + fileId, fps: 23.976, url: `https://www.opensubtitles.com/s/${fileId}`, files: [{ file_id: fileId, file_name: `${release}.srt` }], ...extra },
  };
}

type SetupOptions = {
  key?: boolean;
  privacy?: boolean;
  settings?: Record<string, unknown>;
  /** The player's secret store refuses every call with this code. */
  store?: "unavailable" | "unsupported" | "failed";
  os?: "macos" | "windows" | "linux";
  /** Keys already in `secrets.json` (an older fallback). */
  file?: Record<string, string>;
  /** The interface language (default English). */
  locale?: "zh-Hans";
  /** The video opened as a `file://` URL instead of a path. */
  fileUrl?: boolean;
};
async function setup(options: SetupOptions = {}) {
  const folder = await mkdtemp(join(tmpdir(), "vestra-online-"));
  const data = join(folder, "data");
  const video = join(folder, "The.Movie.2019.1080p.BluRay.x264-GRP.mkv");
  await writeFile(video, Buffer.alloc(200 * 1024, 7));
  const { mkdir } = await import("node:fs/promises");
  await mkdir(data);
  if (options.file) await writeFile(join(data, "secrets.json"), JSON.stringify(options.file));
  await writeFile(join(data, "settings.json"), JSON.stringify({ privacyAccepted: options.privacy !== false, ...options.settings }));
  const host = new TestHost();
  if (options.key !== false) host.secrets.set("opensubtitles-api-key", "KEY");
  if (options.os) host.systemInfo = { ...host.systemInfo, os: options.os };
  const code = options.store;
  if (code)
    for (const method of ["secrets.get", "secrets.set", "secrets.delete"])
      host.handle(method, () => {
        throw new HostError(code, `${method}: ${code}`);
      });
  host.openElsewhere(options.fileUrl ? pathToFileURL(video).href : video);
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: null };
  const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data, ...(options.locale ? { locale: options.locale } : {}) });
  return { host, folder, data, video, stop, cleanup: () => rm(folder, { recursive: true, force: true }) };
}
/** Whether `folder`'s file system keeps POSIX modes: chmod a probe to 0600 and read it back (not on NTFS). */
async function modesEnforced(folder: string) {
  const probe = join(folder, "mode-probe");
  await writeFile(probe, "");
  await chmod(probe, 0o600);
  const kept = ((await stat(probe)).mode & 0o777) === 0o600;
  await rm(probe);
  return kept;
}
/** The window binds exactly the callbacks its Slint file declares. */
async function sameCallbacks(host: TestHost, id: string, file: string) {
  const slint = await readFile(new URL(`../ui/${file}.slint`, import.meta.url), "utf8");
  const callbacks = [...slint.matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
  assert.deepEqual([...host.window(id).callbacks].sort(), callbacks.sort(), file);
}
/** A window's status line as it shows. */
const shown = (host: TestHost, id: string) => host.window(id).props.status as string;
const zh = { locale: "zh-Hans" } as const;
const waitFor = async (check: () => boolean, what: string) => {
  const ok = () => {
    try {
      return check();
    } catch {
      return false; // e.g. a window that is not open yet
    }
  };
  // A time deadline, not a count of rounds: on a loaded machine each round is slower.
  for (const deadline = Date.now() + 15_000; !ok() && Date.now() < deadline; ) await pause(10);
  assert.ok(ok(), what);
};

test("menu: two items under Subtitles ▸ Online Subtitles without shortcuts; windows bind their Slint callbacks", async () => {
  const fetch = fakeFetch({});
  const t = await setup({ key: false });
  try {
    const declared = [...t.host.commands.values()].map(({ declaration: d }) => [d.path.join(" > "), d.order, d.group, d.bindings.join(",")]);
    assert.deepEqual(declared, [
      ["@subtitles > Online Subtitles > Search and Download Subtitles…", 360, "subtitle-online", ""],
      ["@subtitles > Online Subtitles > Online Subtitle Settings…", 361, "subtitle-online", ""],
    ]);
    await t.host.command("search").run();
    await t.host.settle();
    assert.equal(t.host.window("subtitle-online-settings").visible, true, "no key: the settings window opens instead");
    assert.match(shown(t.host, "subtitle-online-settings"), /^First enter an API key for OpenSubtitles/);
    assert.equal(fetch.calls.length, 0, "nothing is sent without a key");
    for (const [id, file] of [["subtitle-online-settings", "settings"]] as const) {
      const slint = await readFile(new URL(`../ui/${file}.slint`, import.meta.url), "utf8");
      const callbacks = [...slint.matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
      assert.deepEqual([...t.host.window(id).callbacks].sort(), callbacks.sort(), file);
    }
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("keys are saved in the player's secret store, never in settings.json", async () => {
  const fetch = fakeFetch({ [`${OS}/infos/formats`]: () => jsonResponse({ data: {} }) });
  const t = await setup({ key: false, ...zh });
  try {
    await t.host.command("settings").run();
    const settings = t.host.window("subtitle-online-settings");
    settings.emit("service-settings");
    await t.host.settle();
    const service = t.host.window("subtitle-online-service");
    assert.equal(service.props["service"], "opensubtitles");
    await sameCallbacks(t.host, "subtitle-online-service", "service");
    service.emit("test", "SECRET", "", "");
    await t.host.settle();
    await waitFor(() => shown(t.host, "subtitle-online-service") === "密钥有效（未登录，匿名下载次数较少）", "test result");
    assert.equal(fetch.calls[0]!.headers["Api-Key"], "SECRET");
    assert.equal(fetch.calls[0]!.headers["User-Agent"], "Vestra Player v0.1.0");
    service.emit("save", "SECRET", "me", "pw");
    await t.host.settle();
    await waitFor(() => shown(t.host, "subtitle-online-service").startsWith("已保存"), "saved");
    assert.equal(shown(t.host, "subtitle-online-service"), "已保存（macOS 钥匙串（vestra.subtitle-online））");
    assert.equal(service.props["location"], "macOS 钥匙串（vestra.subtitle-online）", "the help text names it");
    assert.deepEqual(Object.fromEntries(t.host.secrets), { "opensubtitles-api-key": "SECRET", "opensubtitles-password": "pw" });
    assert.equal(await stat(join(t.data, "secrets.json")).catch(() => null), null, "nothing in a file");
    service.emit("save", "SECRET", "me", "");
    await t.host.settle();
    await waitFor(() => !t.host.secrets.has("opensubtitles-password"), "an empty password deletes the secret");
    await t.stop();
    const saved = await readFile(join(t.data, "settings.json"), "utf8");
    assert.doesNotMatch(saved, /SECRET|pw/);
    assert.equal(JSON.parse(saved).username, "me");
  } finally {
    fetch.restore();
    await t.cleanup();
  }
});

/** Open the service settings for OpenSubtitles and save `key` / `password`; the service window. */
async function saveKey(t: Awaited<ReturnType<typeof setup>>, key: string, password: string) {
  await t.host.command("settings").run();
  t.host.window("subtitle-online-settings").emit("service-settings");
  await t.host.settle();
  const service = t.host.window("subtitle-online-service");
  service.emit("save", key, "", password);
  await t.host.settle();
  await waitFor(() => /^(已保存|无法保存)/.test(shown(t.host, "subtitle-online-service")), "save finished");
  return service;
}

for (const code of ["unavailable", "unsupported"] as const)
  test(`no secret store (${code}): keys go to secrets.json (0600) in the data directory`, async (context) => {
    const fetch = fakeFetch({});
    const t = await setup({ key: false, store: code, ...zh });
    try {
      await saveKey(t, "SECRET", "pw");
      const file = join(t.data, "secrets.json");
      assert.equal(shown(t.host, "subtitle-online-service"), `已保存（${file}（仅本人可读））`);
      assert.deepEqual(JSON.parse(await readFile(file, "utf8")), { "opensubtitles-api-key": "SECRET", "opensubtitles-password": "pw" });
      if (await modesEnforced(t.folder)) assert.equal((await stat(file)).mode & 0o777, 0o600);
      else context.diagnostic("this file system has no POSIX modes: the 0600 check is skipped");
      assert.equal(t.host.secrets.size, 0);
    } finally {
      await t.stop();
      fetch.restore();
      await t.cleanup();
    }
  });

test("a key left in secrets.json by the fallback is still read; saving to the store removes the copy", async () => {
  const fetch = fakeFetch({});
  const t = await setup({ key: false, file: { "opensubtitles-api-key": "OLD", "assrt-token": "TOKEN" }, ...zh });
  try {
    await t.host.command("settings").run();
    const settings = t.host.window("subtitle-online-settings");
    await waitFor(() => settings.props.services !== undefined, "settings bound");
    const states = (settings.props.services as { state: string }[]).map((s) => s.state);
    assert.deepEqual(states, ["已填写密钥", "已填写 token"]);
    await saveKey(t, "NEW", "");
    assert.equal(t.host.secrets.get("opensubtitles-api-key"), "NEW");
    assert.deepEqual(JSON.parse(await readFile(join(t.data, "secrets.json"), "utf8")), { "assrt-token": "TOKEN" });
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("the store's location is named after the system; a refused save is reported, not moved to a file", async () => {
  const fetch = fakeFetch({});
  const windows = await setup({ key: false, os: "windows", ...zh });
  try {
    await saveKey(windows, "SECRET", "");
    assert.equal(shown(windows.host, "subtitle-online-service"), "已保存（Windows 凭据管理器（vestra.subtitle-online））");
    assert.match(windows.host.window("subtitle-online-settings").props["location"] as string, /Windows 凭据管理器/);
  } finally {
    await windows.stop();
    await windows.cleanup();
  }
  const refused = await setup({ key: false, store: "failed", ...zh });
  try {
    await saveKey(refused, "SECRET", "");
    assert.equal(shown(refused.host, "subtitle-online-service"), "无法保存：secrets.set: failed");
    assert.equal(await stat(join(refused.data, "secrets.json")).catch(() => null), null);
  } finally {
    await refused.stop();
    fetch.restore();
    await refused.cleanup();
  }
});

test("search sends hash + keywords + languages once privacy is accepted; hash matches rank first", async () => {
  const fetch = fakeFetch({
    [`${OS}/subtitles`]: () =>
      jsonResponse({
        page: 1, total_pages: 2,
        data: [
          osResult(1, "en", "Movie.EN", { moviehash_match: false }),
          osResult(2, "zh-CN", "Movie.MT", { moviehash_match: true, machine_translated: true }),
          osResult(3, "en", "Movie.Hash.EN", { moviehash_match: true }),
          osResult(4, "zh-CN", "Movie.Hash.CN", { moviehash_match: true, hearing_impaired: true }),
        ],
      }),
  });
  const t = await setup({ privacy: false, ...zh });
  try {
    await t.host.command("search").run();
    await t.host.settle();
    const window = t.host.window("subtitle-download");
    assert.equal(window.props["privacy-pending"], true);
    assert.equal(shown(t.host, "subtitle-download"), "搜索会把文件名关键字与文件哈希发送给所选服务。");
    assert.equal(fetch.calls.length, 0, "nothing is sent before the user agrees");
    window.emit("accept-privacy");
    await waitFor(() => shown(t.host, "subtitle-download") === "找到 4 条字幕" && window.props["busy"] === false, "results");
    const url = new URL(fetch.calls[0]!.url);
    assert.equal(url.searchParams.get("languages"), "en,zh-cn,zh-tw");
    assert.match(url.searchParams.get("moviehash")!, /^[0-9a-f]{16}$/);
    assert.equal(url.searchParams.get("query"), "the movie 2019");
    assert.deepEqual([...url.searchParams.keys()], ["languages", "moviehash", "query"], "sorted parameters");
    assert.equal(window.props["title-text"], "The Movie 2019");
    const rows = window.props["rows"] as { release: string; hearing: boolean; machine: boolean; hash: boolean; language: string; service: string }[];
    assert.deepEqual(rows.map((r) => [r.release, r.hearing, r.machine, r.hash]), [
      ["Movie.Hash.CN", true, false, true],
      ["Movie.Hash.EN", false, false, true],
      ["Movie.MT", false, true, true],
      ["Movie.EN", false, false, false],
    ]);
    assert.deepEqual([rows[0]!.language, rows[0]!.service], ["简体中文", "opensubtitles"], "the language named");
    // ui/download.slint names them (ResultRow): the same catalog texts.
    assert.deepEqual(["{} [HI]", "{} [MT]", "Hash"].map((text) => t.host.translate(text, ["Movie"])), [
      "Movie [HI]", "Movie [机翻]", "哈希",
    ]);
    assert.equal(window.props["can-more"], true);
    window.emit("more");
    await waitFor(() => fetch.calls.length === 2, "page 2");
    assert.equal(new URL(fetch.calls[1]!.url).searchParams.get("page"), "2");
    await t.stop();
    assert.equal(JSON.parse(await readFile(join(t.data, "settings.json"), "utf8")).privacyAccepted, true);
  } finally {
    fetch.restore();
    await t.cleanup();
  }
});

/** A search that returns one OpenSubtitles result, and its download. */
function searchAndDownload(file: Buffer | string = SRT, name = "Movie.CN.srt") {
  return fakeFetch({
    [`${OS}/subtitles`]: () => jsonResponse({ page: 1, total_pages: 1, data: [osResult(7, "zh-CN", "Movie.CN", { moviehash_match: true }), osResult(8, "en", "Movie.EN")] }),
    [`${OS}/download`]: (call) => jsonResponse({ link: `https://dl.example/${JSON.parse(call.body!).file_id}`, file_name: name, remaining: 4 }),
    "https://dl.example/": () => new Response(typeof file === "string" ? file : new Uint8Array(file)),
  });
}

test("double-click downloads, loads and selects the subtitle, and saves <video name>.zh.srt beside the video", async () => {
  const fetch = searchAndDownload();
  const t = await setup(zh);
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 2 && window.props["busy"] === false, "results");
    window.emit("open-row", 0);
    // The copy beside the video is written after the load, by the plugin itself: wait for the operation to end.
    await waitFor(() => t.host.subtitleLoads.length === 1 && window.props["busy"] === false, "loaded");
    assert.deepEqual(t.host.subtitleLoads.map((l) => [l.format, l.title, l.select, l.bytes.toString()]), [["srt", "简体中文 · Movie.CN", "primary", SRT]]);
    assert.equal(JSON.parse(fetch.calls.find((c) => c.url.endsWith("/download"))!.body!).file_id, 7);
    assert.equal(t.host.osd.at(-1), "打开字幕：简体中文 · Movie.CN");
    const video = join(t.folder, "The.Movie.2019.1080p.BluRay.x264-GRP.zh.srt");
    assert.equal(shown(t.host, "subtitle-download"), `打开字幕：简体中文 · Movie.CN。已保存到 ${video}`);
    const files = await readdir(t.folder);
    assert.ok(files.includes("The.Movie.2019.1080p.BluRay.x264-GRP.zh.srt"), files.join());
    window.emit("open-row", 0);
    await waitFor(() => t.host.subtitleLoads.length === 2 && window.props["busy"] === false, "loaded again");
    assert.ok((await readdir(t.folder)).includes("The.Movie.2019.1080p.BluRay.x264-GRP.zh (1).srt"), "never overwrites");
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("a source opened as a file:// URL: hash and title from the file, the copy saved beside the video", async () => {
  const fetch = searchAndDownload();
  const t = await setup({ fileUrl: true, ...zh });
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 2 && window.props["busy"] === false, "results");
    const url = new URL(fetch.calls[0]!.url);
    assert.match(url.searchParams.get("moviehash") ?? "", /^[0-9a-f]{16}$/, "the file was read for its hash");
    assert.equal(url.searchParams.get("query"), "the movie 2019");
    window.emit("open-row", 0);
    await waitFor(() => t.host.subtitleLoads.length === 1 && window.props["busy"] === false, "loaded");
    assert.ok((await readdir(t.folder)).includes("The.Movie.2019.1080p.BluRay.x264-GRP.zh.srt"), "beside the video");
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("Add adds the checked results without selecting them; Save Only only saves", async () => {
  const fetch = searchAndDownload();
  const t = await setup({ settings: { saveLocation: "data" }, ...zh });
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 2 && window.props["busy"] === false, "results");
    window.emit("toggle-row", 0, true);
    window.emit("toggle-row", 1, true);
    await t.host.settle();
    assert.equal(window.props["has-checked"], true);
    window.emit("add-checked");
    await waitFor(() => t.host.osd.at(-1) === "已添加 2 条字幕" && window.props["busy"] === false, "added");
    assert.deepEqual(t.host.subtitleLoads.map((l) => l.select), ["none", "none"]);
    assert.equal(t.host.subtitles.selection.target, null);
    window.emit("save-checked");
    await waitFor(() => shown(t.host, "subtitle-download") === `已保存 2 个字幕到 ${join(t.data, "downloads")}`, "saved");
    assert.equal(t.host.subtitleLoads.length, 2, "Save Only does not load");
    assert.equal((await readdir(join(t.data, "downloads"))).length, 4, "two from Add, two from Save Only");
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("quota, bad key and network failures are explained; other services' results still show", async () => {
  let mode: "quota" | "auth" | "offline" = "quota";
  const fetch = fakeFetch({
    [`${OS}/subtitles`]: () => (mode === "auth" ? jsonResponse({ message: "nope" }, 401) : jsonResponse({ data: [osResult(7, "zh-CN", "Movie.CN")] })),
    [`${OS}/download`]: () => jsonResponse({ remaining: 0, message: "quota", reset_time: "23 小时" }, 406),
  });
  const t = await setup(zh);
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 1 && window.props["busy"] === false, "results");
    window.emit("open-row", 0);
    await waitFor(() => /重置/.test(shown(t.host, "subtitle-download")), "quota");
    assert.equal(t.host.osd.at(-1), "今日下载次数已用完（OpenSubtitles）。登录账号可获得更多次数");
    assert.equal(shown(t.host, "subtitle-download"), "今日下载次数已用完（OpenSubtitles）。登录账号可获得更多次数。将于 23 小时 重置");
    assert.equal(t.host.subtitleLoads.length, 0);
    mode = "auth";
    window.emit("search-again");
    await waitFor(() => shown(t.host, "subtitle-download") === "API 密钥无效，请在在线字幕设置中检查", "auth");
    fetch.restore();
    const offline = fakeFetch({});
    window.emit("search-title", "Movie");
    await waitFor(() => shown(t.host, "subtitle-download") === "无法连接 OpenSubtitles", "offline");
    offline.restore();
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

test("a source change during the download aborts it; nothing is loaded into the new source", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const fetch = fakeFetch({
    [`${OS}/subtitles`]: () => jsonResponse({ data: [osResult(7, "zh-CN", "Movie.CN")] }),
    [`${OS}/download`]: async () => {
      await gate;
      return jsonResponse({ link: "https://dl.example/7", file_name: "a.srt" });
    },
    "https://dl.example/": () => new Response(SRT),
  });
  const t = await setup();
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 1 && window.props["busy"] === false, "results");
    window.emit("open-row", 0);
    await waitFor(() => fetch.calls.some((c) => c.url.endsWith("/download")), "download started");
    t.host.openElsewhere("/other.mkv");
    t.host.subtitles.selection.stamp = { generation: 2, revision: 1 };
    await t.host.settle();
    release();
    await pause(50);
    await t.host.settle();
    assert.equal(t.host.subtitleLoads.length, 0);
    assert.equal(shown(t.host, "subtitle-download"), "The media changed; no subtitle was loaded.");
    assert.deepEqual(window.props["rows"], [], "results of the old source are dropped");
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});

/** A zip with stored and deflated entries. */
function zip(files: Record<string, string>) {
  const locals: Buffer[] = [], centrals: Buffer[] = [];
  let offset = 0;
  for (const [index, [name, text]] of Object.entries(files).entries()) {
    const raw = Buffer.from(text), deflate = index % 2 === 1;
    const data = deflate ? deflateRawSync(raw) : raw;
    const nameBytes = Buffer.from(name);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(0x800, 6);
    local.writeUInt16LE(deflate ? 8 : 0, 8);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(0x800, 8);
    central.writeUInt16LE(deflate ? 8 : 0, 10);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(raw.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBytes, data);
    centrals.push(central, nameBytes);
    offset += 30 + nameBytes.length + data.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

test("a zip with several subtitle files asks which one to load; rar is refused with a reason", async () => {
  const archive = zip({ "readme.txt": "hi", "a.chs.srt": SRT, "b.cht.ass": "[Script Info]\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,繁體\n" });
  let body: Buffer = archive;
  const routed = fakeFetch({
    [`${OS}/subtitles`]: () => jsonResponse({ data: [osResult(7, "zh-TW", "Pack")] }),
    [`${OS}/download`]: () => jsonResponse({ link: "https://dl.example/7", file_name: "pack.zip" }),
    "https://dl.example/": () => new Response(new Uint8Array(body)),
  });
  const t = await setup();
  try {
    await t.host.command("search").run();
    const window = t.host.window("subtitle-download");
    await waitFor(() => (window.props["rows"] as unknown[])?.length === 1 && window.props["busy"] === false, "results");
    window.emit("open-row", 0);
    await waitFor(() => t.host.window("subtitle-online-choose").visible === true, "chooser");
    const chooser = t.host.window("subtitle-online-choose");
    assert.deepEqual(chooser.props["names"], ["a.chs.srt", "b.cht.ass"]);
    await sameCallbacks(t.host, "subtitle-online-choose", "choose");
    await sameCallbacks(t.host, "subtitle-download", "download");
    chooser.emit("choose", 1);
    await waitFor(() => t.host.subtitleLoads.length === 1, "loaded");
    assert.equal(t.host.subtitleLoads[0]!.format, "ass");
    assert.match(t.host.subtitleLoads[0]!.bytes.toString(), /繁體/);
    // The load is recorded before the operation ends; a click while busy is ignored.
    await waitFor(() => window.props["busy"] === false, "first download finished");
    body = Buffer.from("Rar!\u001a\u0007\u0000");
    window.emit("open-row", 0);
    await waitFor(() => /rar/.test(shown(t.host, "subtitle-download")), "rar refused");
    assert.equal(shown(t.host, "subtitle-download"), "This archive format (rar, 7z) is not supported yet; download a zip or a single subtitle file.");
    assert.equal(t.host.subtitleLoads.length, 1);
  } finally {
    await t.stop();
    routed.restore();
    await t.cleanup();
  }
});

test("automatic search: a hash match in the first language is loaded when a video without subtitles opens", async () => {
  const fetch = searchAndDownload();
  const t = await setup({ settings: { autoSearch: "no-subtitles", afterSearch: "load-best" }, ...zh });
  try {
    assert.equal(fetch.calls.length, 0, "the source open at start is left alone");
    t.host.openElsewhere(t.video);
    t.host.subtitles.selection.stamp = { generation: 2, revision: 1 };
    await waitFor(() => t.host.osd.at(-1)?.startsWith("已自动载入在线字幕") === true, "auto loaded");
    assert.equal(t.host.osd.at(-1), "已自动载入在线字幕：简体中文 · Movie.CN");
    assert.equal(t.host.subtitleLoads[0]!.select, "primary");
    // A video that already has a text track is not searched.
    const before = fetch.calls.length;
    t.host.subtitles.tracks.push({ target: { kind: "embedded", id: 1 }, title: "x", language: null, styleSupported: true, styleReason: null, codec: "subrip", text: true, streamIndex: 2 });
    t.host.openElsewhere(t.video);
    t.host.subtitles.selection.stamp = { generation: 3, revision: 1 };
    await pause(100);
    await t.host.settle();
    assert.equal(fetch.calls.length, before);
  } finally {
    await t.stop();
    fetch.restore();
    await t.cleanup();
  }
});
