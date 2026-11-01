import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test, { afterEach, beforeEach } from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { compareVersions, deps, downloadUrl, LATEST_URL, parseLatest, platformKey, releasePage, timing, UpdateError } from "../updates.ts";

const directory = new URL("..", import.meta.url);
const tick = () => new Promise((done) => setTimeout(done, 5));
async function until(predicate: () => boolean, what = "condition") {
  for (let i = 0; i < 1000; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail(`timed out waiting for ${what}`);
}

type Route = { status?: number; body?: string | Buffer; headers?: Record<string, string> } | "offline";
/** A fetch that answers from a route table; unknown addresses are 404. Nothing leaves the machine. */
class FakeNet {
  routes = new Map<string, Route>();
  requests: string[] = [];
  count = (url: string) => this.requests.filter((r) => r === url).length;
  fetch = async (url: string, init: RequestInit): Promise<Response> => {
    this.requests.push(url);
    assert.equal(init.redirect, "manual", "redirects are followed by hand (https only)");
    const route = this.routes.get(url) ?? { status: 404 };
    if (route === "offline") throw new TypeError("fetch failed", { cause: { code: "ENOTFOUND", message: "getaddrinfo ENOTFOUND github.com" } });
    const status = route.status ?? 200;
    const body = route.body === undefined || status >= 300 ? null : Buffer.from(route.body);
    return new Response(body, { status, headers: { ...(body ? { "content-length": String(body.length) } : {}), ...route.headers } });
  };
}

const INSTALLER = Buffer.from("MZ fake Vestra Player setup program ".repeat(64));
const SHA = createHash("sha256").update(INSTALLER).digest("hex");
const FILE = "VestraPlayer-windows-x64-setup.exe";
function latest(version: string, extra: Record<string, unknown> = {}) {
  return JSON.stringify({
    schema: 1,
    version,
    published: "2026-11-05T10:00:00Z",
    notes: `https://github.com/vestrasys/vestra-player/releases/tag/v${version}`,
    downloads: {
      "windows-x64": { file: FILE, size: INSTALLER.length, sha256: SHA },
      "macos-arm64": { file: "VestraPlayer-macos-arm64.dmg", size: 10, sha256: "0".repeat(64) },
    },
    somethingNew: { ignored: true },
    ...extra,
  });
}

let net: FakeNet;
let now: number;
let downloads: string;
const saved = { ...deps };
const savedTiming = { ...timing };
beforeEach(async () => {
  net = new FakeNet();
  now = Date.parse("2026-11-06T08:00:00Z");
  downloads = await mkdtemp(join(tmpdir(), "vestra-info-downloads-"));
  Object.assign(deps, { fetch: net.fetch, now: () => now, downloads: () => downloads });
  // The automatic check stays out of the way unless a test shortens it.
  Object.assign(timing, { firstCheck: 1_000_000_000 });
});
afterEach(async () => {
  Object.assign(deps, saved);
  Object.assign(timing, savedTiming);
  await rm(downloads, { recursive: true, force: true });
});

async function start(options: { version?: string; dataDirectory?: string; locale?: "zh-Hans"; os?: "windows" | "macos"; arch?: string } = {}) {
  const host = new TestHost();
  host.app = { version: options.version ?? "0.6.2", build: "abc1234", components: { mpv: "mpv v0.41.0", libav: { ffmpeg: "9.0.2" }, slint: "1.16.1" } };
  host.systemInfo = { ...host.systemInfo, os: options.os ?? "windows", arch: options.arch ?? "x86_64" };
  host.packages = [];
  const started = await startPlugin(activate, {
    directory,
    host,
    ...(options.locale ? { locale: options.locale } : {}),
    ...(options.dataDirectory ? { dataDirectory: options.dataDirectory } : {}),
  });
  return { host, plugin: started.plugin, stop: () => started.stop() };
}
/** Check for Updates… and wait for the result in the About window. */
async function manualCheck(host: TestHost) {
  await host.command("info.update").run();
  const window = host.window("about");
  await until(() => window.props["update-state"] !== "checking", "the check");
  await host.settle();
  return window;
}

test("J10: top-level Check for Updates… right after About (881, windows); Options ▸ General ▸ Updates, on by default", async () => {
  const { host, stop } = await start();
  try {
    assert.deepEqual(host.command("info.update").declaration, {
      title: "Check for Updates…", path: ["Check for Updates…"], order: 881, group: "windows", radio: false, bindings: [], focus: null, repeat: false,
    });
    assert.deepEqual(host.conflicts, []);
    const page = host.settingsPage("info.updates");
    assert.deepEqual([page.category, page.title, page.order, page.file], ["general", "Updates", 20, "ui/updates.slint"]);
    assert.equal(page.props["auto-check"], true);
    // Nothing keeps the plugin running for the check (its timer does not count; lazy activation).
    assert.deepEqual(host.keepsRunning, []);
    assert.equal(net.requests.length, 0, "nothing goes online at activation");
  } finally {
    await stop();
  }
});

test("J10: Check for Updates… with a newer release: version, date, Download and Release Notes in About", async () => {
  net.routes.set(LATEST_URL, { body: latest("0.6.3") });
  const { host, stop } = await start();
  try {
    const window = await manualCheck(host);
    assert.equal(window.visible, true);
    assert.equal(window.props["update-state"], "available");
    assert.equal(window.props["latest-version"], "0.6.3");
    assert.equal(window.props["latest-date"], "2026-11-05");
    assert.equal(window.props["update-button"], "download");
    assert.equal(window.props["has-notes"], true);
    assert.deepEqual(host.osd, [], "a manual check shows it in the window, not on the video");
    window.emit("release-notes");
    await until(() => host.system.urls.length === 1);
    assert.deepEqual(host.system.urls, ["https://github.com/vestrasys/vestra-player/releases/tag/v0.6.3"]);
  } finally {
    await stop();
  }
});

test("J10: the same or an older release is up to date", async () => {
  for (const [version, running] of [["0.6.2", "0.6.2"], ["0.6.1", "0.6.2"], ["0.6.2", "0.6.2-rc.1"]] as const) {
    net.routes.set(LATEST_URL, { body: latest(version) });
    const { host, stop } = await start({ version: running });
    try {
      const window = await manualCheck(host);
      const expected = version === "0.6.2" && running === "0.6.2-rc.1" ? "available" : "current";
      assert.equal(window.props["update-state"], expected, `${version} vs ${running}`);
      if (expected === "current") assert.equal(window.props["update-button"], "check");
    } finally {
      await stop();
    }
  }
});

test("J10: a manual check says why it failed: offline, 404, rate limits, malformed JSON, unknown schema", async () => {
  const cases: [Route, string, string][] = [
    ["offline", "offline", ""],
    [{ status: 404 }, "not-found", "404"],
    [{ status: 403 }, "rate-limited", "403"],
    [{ status: 429 }, "rate-limited", "429"],
    [{ status: 500 }, "http", "500"],
    [{ body: "{ not json" }, "invalid", ""],
    [{ body: JSON.stringify({ schema: 1, downloads: {} }) }, "invalid", ""],
    [{ body: latest("0.7.0", { schema: 2 }) }, "schema", ""],
  ];
  for (const [route, code, detail] of cases) {
    net.routes.set(LATEST_URL, route);
    const { host, stop } = await start();
    try {
      const window = await manualCheck(host);
      assert.equal(window.props["update-state"], "failed", code);
      assert.equal(window.props["update-error"], code);
      assert.equal(window.props["update-detail"], detail);
      assert.equal(window.props["update-button"], "check", "Check Now tries again");
      assert.deepEqual(host.osd, []);
    } finally {
      await stop();
    }
  }
});

test("J10: redirects are followed over https only", async () => {
  const objects = "https://objects.githubusercontent.com/latest.json";
  net.routes.set(LATEST_URL, { status: 302, headers: { location: objects } });
  net.routes.set(objects, { body: latest("0.6.3") });
  const { host, stop } = await start();
  try {
    assert.equal((await manualCheck(host)).props["update-state"], "available");
    net.routes.set(LATEST_URL, { status: 302, headers: { location: "http://example.com/latest.json" } });
    const window = await manualCheck(host);
    assert.equal(window.props["update-state"], "failed");
    assert.equal(net.count("http://example.com/latest.json"), 0, "never fetched over http");
  } finally {
    await stop();
  }
});

test("J10: no installer for this platform: Open Release Page instead of Download", async () => {
  net.routes.set(LATEST_URL, { body: latest("0.6.3") });
  const { host, stop } = await start({ os: "macos", arch: "x86_64" });
  try {
    const window = await manualCheck(host);
    assert.equal(window.props["update-state"], "available");
    assert.equal(window.props["update-button"], "release-page");
    window.emit("update-action");
    await until(() => host.system.urls.length === 1);
    assert.equal(host.system.urls[0], "https://github.com/vestrasys/vestra-player/releases/tag/v0.6.3");
  } finally {
    await stop();
  }
});

test("J10 automatic check: 30 s after activation, one OSD notice per new version, quiet failures, at most once a day", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-info-data-"));
  try {
    Object.assign(timing, { firstCheck: 20 });
    net.routes.set(LATEST_URL, { body: latest("0.6.3") });
    let run = await start({ dataDirectory: data });
    await until(() => run.host.osd.length === 1, "the notice");
    assert.deepEqual(run.host.osd, ["Vestra Player 0.6.3 is available. Open About Vestra Player… to download it."]);
    assert.equal(net.count(LATEST_URL), 1);
    // The About window shows the marker without going online again.
    await run.host.command("info.about").run();
    await run.host.settle();
    assert.equal(run.host.window("about").props["update-state"], "available");
    await run.stop();

    // Next start within the day: no request.
    now += 60 * 60 * 1000;
    run = await start({ dataDirectory: data });
    await new Promise((done) => setTimeout(done, 100));
    assert.equal(net.count(LATEST_URL), 1, "throttled to once a day");
    await run.stop();

    // A day later the same version: checked again, no second notice.
    now += timing.interval;
    run = await start({ dataDirectory: data });
    await until(() => net.count(LATEST_URL) === 2, "the daily check");
    await new Promise((done) => setTimeout(done, 50));
    assert.deepEqual(run.host.osd, [], "one notice per version");
    await run.stop();

    // A day later, offline: quiet (logged only), the window keeps the last known release.
    now += timing.interval;
    net.routes.set(LATEST_URL, "offline");
    run = await start({ dataDirectory: data });
    await until(() => net.count(LATEST_URL) === 3, "the offline check");
    await new Promise((done) => setTimeout(done, 50));
    assert.deepEqual(run.host.osd, []);
    await run.host.command("info.about").run();
    await run.host.settle();
    assert.equal(run.host.window("about").props["update-state"], "available");
    assert.equal(run.host.window("about").props["update-error"], "");
    await run.stop();

    // A day later, a newer version again: one more notice.
    now += timing.interval;
    net.routes.set(LATEST_URL, { body: latest("0.6.4") });
    run = await start({ dataDirectory: data });
    await until(() => run.host.osd.length === 1, "the second notice");
    assert.deepEqual(run.host.osd, ["Vestra Player 0.6.4 is available. Open About Vestra Player… to download it."]);
    await run.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("J10: turning the automatic check off in Options stops it; Check for Updates… still works", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-info-data-"));
  try {
    net.routes.set(LATEST_URL, { body: latest("0.6.3") });
    let run = await start({ dataDirectory: data });
    run.host.settingsPage("info.updates").emit("toggle-auto", false);
    await run.host.settle();
    assert.equal(run.host.settingsPage("info.updates").props["auto-check"], false);
    await run.stop();
    Object.assign(timing, { firstCheck: 10 });
    run = await start({ dataDirectory: data });
    assert.equal(run.host.settingsPage("info.updates").props["auto-check"], false, "kept in updates.json");
    await new Promise((done) => setTimeout(done, 100));
    assert.equal(net.count(LATEST_URL), 0, "no automatic check");
    assert.equal((await manualCheck(run.host)).props["update-state"], "available");
    await run.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("J10 download: into Downloads with progress, size and sha256 checked, then Run Installer opens it", async () => {
  net.routes.set(LATEST_URL, { body: latest("0.6.3") });
  net.routes.set(downloadUrl(FILE), { status: 302, headers: { location: "https://objects.githubusercontent.com/setup.exe" } });
  net.routes.set("https://objects.githubusercontent.com/setup.exe", { body: INSTALLER });
  const { host, stop } = await start();
  try {
    const window = await manualCheck(host);
    window.emit("update-action");
    await until(() => window.props["update-state"] === "downloaded", "the download");
    await host.settle();
    const path = join(downloads, FILE);
    assert.deepEqual(await readFile(path), INSTALLER);
    assert.deepEqual(await readdir(downloads), [FILE], "no part file left");
    assert.equal(window.props["installer-path"], path);
    assert.equal(window.props.received, INSTALLER.length);
    assert.equal(window.props.total, INSTALLER.length);
    assert.equal(window.props["update-button"], "run");
    assert.equal(host.command("info.update").state.busy, false);
    window.emit("update-action");
    await until(() => host.system.opened.length === 1, "the installer");
    assert.deepEqual(host.system.opened, [path], "started by the system, never run by the plugin itself");
    // A file that changed after the download is not started.
    await writeFile(path, "tampered");
    window.emit("update-action");
    await until(() => window.props["update-state"] === "download-failed");
    assert.equal(window.props["update-error"], "sha256");
    assert.equal(host.system.opened.length, 1);
  } finally {
    await stop();
  }
});

test("J10 download: a size or checksum mismatch is rejected and nothing is kept", async () => {
  const wrong = Buffer.from(INSTALLER);
  wrong[0] = 0x4e;
  const cases: [Buffer, string][] = [
    [INSTALLER.subarray(0, INSTALLER.length - 1), "size"],
    [Buffer.concat([INSTALLER, Buffer.from("x")]), "size"],
    [wrong, "sha256"],
  ];
  for (const [body, code] of cases) {
    net.routes.set(LATEST_URL, { body: latest("0.6.3") });
    net.routes.set(downloadUrl(FILE), { body });
    const { host, stop } = await start();
    try {
      const window = await manualCheck(host);
      window.emit("update-action");
      await until(() => window.props["update-state"] === "download-failed", code);
      assert.equal(window.props["update-error"], code);
      assert.equal(window.props["update-button"], "download", "Download tries again");
      assert.deepEqual(await readdir(downloads), [], `${code}: no file left`);
      assert.deepEqual(host.system.opened, []);
    } finally {
      await stop();
    }
  }
});

test("J10 download: the command is busy while it runs (keeps the plugin running); Cancel stops it", async () => {
  net.routes.set(LATEST_URL, { body: latest("0.6.3") });
  let release!: () => void;
  const gate = new Promise<void>((done) => (release = done));
  const fetch = net.fetch;
  deps.fetch = async (url, init) => {
    if (url !== downloadUrl(FILE)) return fetch(url, init);
    const stream = new ReadableStream<Uint8Array>({
      async start(controller) {
        controller.enqueue(new Uint8Array(INSTALLER.subarray(0, 100)));
        await gate;
        controller.close();
      },
    });
    init.signal?.addEventListener("abort", () => release());
    return new Response(stream, { status: 200 });
  };
  const { host, stop } = await start();
  try {
    const window = await manualCheck(host);
    window.emit("update-action");
    await until(() => window.props["update-state"] === "downloading" && window.props.received === 100, "progress");
    await host.settle();
    assert.equal(host.command("info.update").state.busy, true);
    assert.equal(window.props["update-button"], "cancel");
    window.emit("update-action");
    await until(() => window.props["update-state"] === "available", "cancelled");
    await host.settle();
    assert.equal(host.command("info.update").state.busy, false);
    assert.deepEqual(await readdir(downloads), []);
  } finally {
    release();
    await stop();
  }
});

test("J10 pure rules: versions, platforms, latest.json", () => {
  assert.ok(compareVersions("0.6.3", "0.6.2")! > 0);
  assert.equal(compareVersions("0.6.2", "v0.6.2"), 0);
  assert.ok(compareVersions("0.10.0", "0.9.9")! > 0);
  assert.ok(compareVersions("1.0.0-rc.1", "1.0.0")! < 0);
  assert.ok(compareVersions("1.0.0-rc.2", "1.0.0-rc.10")! < 0);
  assert.ok(compareVersions("1.0.0-alpha", "1.0.0-alpha.1")! < 0);
  assert.equal(compareVersions("1.0.0+build.5", "1.0.0"), 0);
  assert.equal(compareVersions("latest", "1.0.0"), null);
  assert.equal(platformKey("windows", "x86_64"), "windows-x64");
  assert.equal(platformKey("macos", "aarch64"), "macos-arm64");
  assert.equal(platformKey("darwin", "arm64"), "macos-arm64");
  assert.equal(platformKey("other", "x86_64"), null);

  const ok = parseLatest(JSON.parse(latest("0.6.3")), "windows-x64");
  assert.deepEqual(ok, {
    version: "0.6.3", published: "2026-11-05", notesUrl: "https://github.com/vestrasys/vestra-player/releases/tag/v0.6.3",
    installer: { file: FILE, size: INSTALLER.length, sha256: SHA },
  });
  assert.equal(parseLatest(JSON.parse(latest("0.6.3")), "linux-x64").installer, null, "missing platform");
  assert.equal(parseLatest({ schema: "1.2", version: "0.6.3" }, "windows-x64").notesUrl, releasePage("0.6.3"), "a minor schema is read; notes default to the tag page");
  const code = (json: unknown) => {
    try {
      parseLatest(json, "windows-x64");
      return "ok";
    } catch (error) {
      return (error as UpdateError).code;
    }
  };
  assert.equal(code({ schema: 2, version: "0.7.0" }), "schema");
  assert.equal(code({ schema: 0, version: "0.7.0" }), "schema");
  assert.equal(code([]), "invalid");
  assert.equal(code({ version: "0.7.0" }), "invalid");
  assert.equal(code({ schema: 1, version: "seven" }), "invalid");
  const entry = (patch: Record<string, unknown>) => ({ schema: 1, version: "0.7.0", downloads: { "windows-x64": { file: FILE, size: 10, sha256: SHA, ...patch } } });
  assert.equal(code(entry({})), "ok");
  assert.equal(code(entry({ file: "../evil.exe" })), "invalid");
  assert.equal(code(entry({ file: "C:\\evil.exe" })), "invalid");
  assert.equal(code(entry({ file: "sub/evil.exe" })), "invalid");
  assert.equal(code(entry({ size: -1 })), "invalid");
  assert.equal(code(entry({ size: 2 * 1024 * 1024 * 1024 })), "invalid");
  assert.equal(code(entry({ sha256: "abc" })), "invalid");
  // Another platform's broken entry does not matter.
  assert.equal(code({ schema: 1, version: "0.7.0", downloads: { "windows-x64": { file: FILE, size: 10, sha256: SHA }, "macos-arm64": 5 } }), "ok");
});

test("J10 in Simplified Chinese: menu, settings page and notice", async () => {
  Object.assign(timing, { firstCheck: 20 });
  net.routes.set(LATEST_URL, { body: latest("0.6.3") });
  const { host, stop } = await start({ locale: "zh-Hans" });
  try {
    assert.deepEqual(host.command("info.update").display.path, ["检查更新…"]);
    assert.equal(host.settingsPage("info.updates").title, "更新");
    await until(() => host.osd.length === 1);
    assert.deepEqual(host.osd, ["Vestra Player 0.6.3 已发布。打开“关于 Vestra Player…”即可下载。"]);
    assert.equal(host.translate("Version {} is available (released {}).", ["0.6.3", "2026-11-05"]), "新版本 0.6.3 可用（发布于 2026-11-05）。");
    assert.equal(host.translate("Run Installer"), "运行安装程序");
  } finally {
    await stop();
  }
});

test("J10: the new texts are machine translated in every language and marked for review", async () => {
  const pot = await readFile(new URL("../locales/messages.pot", import.meta.url), "utf8");
  const ids = ["Check for Updates…", "Updates", "Check Now", "Download", "Run Installer", "Release Notes", "Downloading version {0}: {1} of {2} MB", "the file's checksum does not match the release"];
  for (const id of ids) assert.ok(pot.includes(`msgid "${id}"`), id);
  for (const lang of ["zh-Hans", "ja", "ko", "de", "fr", "es"]) {
    const po = await readFile(new URL(`../locales/${lang}.po`, import.meta.url), "utf8");
    for (const id of ids) {
      const escaped = id.replace(/[(){}.]/g, "\\$&");
      assert.match(po, new RegExp(`#, x-machine\nmsgid "${escaped}"\nmsgstr "[^"]+"`), `${lang}: ${id}`);
    }
  }
});

test("J10: an About window opened before any check offers Check Now; a refused release page is only logged", async () => {
  const { host, stop } = await start();
  try {
    await host.command("info.about").run();
    await host.settle();
    const window = host.window("about");
    assert.equal(window.props["update-state"], "");
    assert.equal(window.props["update-button"], "check");
    assert.equal(window.props["has-notes"], false);
    host.handle("system.openUrl", () => {
      throw new HostError("failed", "refused");
    });
    window.emit("release-notes");
    await host.settle();
    net.routes.set(LATEST_URL, { body: latest("0.6.3") });
    window.emit("update-action");
    await until(() => window.props["update-state"] === "available");
  } finally {
    await stop();
  }
});
