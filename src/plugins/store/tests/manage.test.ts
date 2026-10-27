/**
 * Finishing the plugin center (store 0.2.1): row logos (own package icon,
 * repository icon through ui.remoteImage, the store's icon otherwise),
 * reloading through packages.reload (built-ins too) and its refusals, the
 * warnings of a running plugin, development loads under Installed, and the
 * list read again after a refusal.
 */
import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import {
  actions,
  AVAILABLE,
  banners,
  BUILTIN,
  choose,
  controls,
  FakeNet,
  info,
  type Info,
  OFFICIAL,
  openStore,
  props,
  publish,
  rows,
  start,
  tab,
  until,
  yes,
} from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-manage-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const PNG = new URL("../assets/icon.png", import.meta.url);
const fromRepo = { kind: "repository", url: OFFICIAL } as const;
const one = (id: string, name: string, extra: { iconUrl?: string } = {}) => ({ id, name, ...extra, versions: [{ version: "1.0.0" }] });

type Host = Awaited<ReturnType<typeof start>>["host"];
type Tag = { text: string; tone: number };
type ShownRow = { key: string; name: string; badge: string; tip: string; logo: unknown; flags: Tag[] };
const row = (host: Host, name: string) => (rows(host) as unknown as ShownRow[]).find((r) => r.name === name);
const logoOf = (host: Host, name: string) => {
  const found = row(host, name);
  return found ? host.imagePath(found.logo as never) : null;
};
const reloads = (host: Host) => host.packageCalls.filter((c) => c.method === "reload").map((c) => c.id);

/** A package directory with a manifest and, when asked, its own icon. */
function packageDir(id: string, withIcon: boolean) {
  const directory = join(scratch, "packages", id);
  mkdirSync(directory, { recursive: true });
  writeFileSync(
    join(directory, "manifest.json"),
    JSON.stringify({ id, version: "1.0.0", name: id, ...(withIcon ? { icon: "assets/logo.png" } : {}) }),
  );
  if (withIcon) {
    mkdirSync(join(directory, "assets"), { recursive: true });
    copyFileSync(PNG, join(directory, "assets", "logo.png"));
  }
  return directory;
}

test("row logos: an installed plugin's own icon (offline too), else the store's; decoded at 64", async () => {
  const net = new FakeNet();
  net.offline = true;
  const own = info("x.own", { directory: packageDir("x.own", true) });
  const plain = info("x.plain", { directory: packageDir("x.plain", false) });
  const { host, stop } = await start({ net, packages: [BUILTIN, own, plain], dataDirectory: join(scratch, "data-offline") });
  try {
    await openStore(host);
    await until(host, () => logoOf(host, "x.own") !== null && logoOf(host, "x.plain") !== null, "logos");
    // A copy in the store's data folder (the player shows images only from the store's own folders).
    assert.match(String(logoOf(host, "x.own")), /data-offline[\\/]icons[\\/][0-9a-f]+\.png$/);
    assert.match(String(logoOf(host, "x.plain")), /assets[\\/]icon\.png$/, "no icon: the store's");
    assert.match(String(logoOf(host, "vestra.video")), /assets[\\/]icon\.png$/);
    for (const name of ["x.own", "x.plain", "vestra.video"]) {
      const path = logoOf(host, name);
      assert.equal(host.images.find((image) => image.path === path)?.maxSide, 64, name);
    }
    // One reference per file: a row written again keeps its image.
    const refs = () => new Set(host.images.map((image) => image.ref)).size;
    const before = refs();
    host.window("store").emit("search", "x.");
    await until(host, () => rows(host).length === 2, "filtered");
    assert.equal(refs(), before);
  } finally {
    await stop();
  }
});

test("row logos: repository icons through ui.remoteImage for the rows shown; the store's icon when it fails", async () => {
  const net = new FakeNet();
  const good = "https://plugins.test/assets/r.icon/icon.png";
  const broken = "https://plugins.test/assets/r.broken/icon.png";
  publish(net, OFFICIAL, [
    one("r.icon", "R Icon", { iconUrl: good }),
    one("r.broken", "R Broken", { iconUrl: broken }),
    one("r.none", "R None"),
  ]);
  const served = join(scratch, "served.png");
  copyFileSync(PNG, served);
  const { host, stop } = await start({ net, dataDirectory: join(scratch, "data-remote") });
  host.remoteImages.set(good, served);
  host.remoteImages.set(broken, null);
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 1, "installed");
    assert.equal(host.images.filter((image) => image.path.includes(".vestra-cache")).length, 0, "Available not shown yet: nothing fetched");
    await tab(host, AVAILABLE);
    await until(host, () => /\.vestra-cache/.test(String(logoOf(host, "R Icon"))), "remote logo");
    assert.equal(host.images.find((image) => image.path === logoOf(host, "R Icon"))?.maxSide, 64);
    assert.match(String(logoOf(host, "R Broken")), /assets[\\/]icon\.png$/);
    assert.match(String(logoOf(host, "R None")), /assets[\\/]icon\.png$/);
    // The details' icon is decoded at twice its 56px.
    await choose(host, "R None");
    await until(host, () => /assets[\\/]icon\.png$/.test(String(host.imagePath(props(host)["d-icon"]))), "details icon");
    const icon = host.imagePath(props(host)["d-icon"]);
    assert.ok(host.images.some((image) => image.path === icon && image.maxSide === 112));
  } finally {
    await stop();
  }
});

test("reload: busy, invalid (disabled meanwhile), missing (list read again) and a failed restart", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, stop } = await start({
    net,
    packages: [BUILTIN, info("a.run", { source: fromRepo }), info("b.run"), info("c.run"), info("d.run")],
  });
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 5, "rows");
    const window = host.window("store");
    const status = () => props(host)["status-text"];

    host.handle("packages.reload", () => {
      throw new HostError("busy", "the package is being installed or reloaded");
    });
    window.emit("rowAct", "\na.run", "reload");
    await until(host, () => status() === "「a.run」正在安装或重新加载，请稍后再试", "busy");
    assert.equal(props(host)["status-warn"], true);

    host.handle("packages.reload", () => {
      throw new HostError("invalid", "b.run is disabled");
    });
    // Disabled somewhere else, and the store has not heard yet.
    host.packages = host.packages.map((p) => (p.id === "b.run" ? { ...p, state: "disabled" } : p));
    window.emit("rowAct", "\nb.run", "reload");
    await until(host, () => status() === "「b.run」已停用，请启用它，而不是重新加载" && controls(host, "b.run").dim, "invalid, re-read");

    host.handle("packages.reload", () => {
      throw new HostError("missing", "no plugin c.run");
    });
    host.packages = host.packages.filter((p) => p.id !== "c.run");
    window.emit("rowAct", "\nc.run", "reload");
    await until(host, () => status() === "无法重新加载「c.run」：这个插件已不存在" && !row(host, "c.run"), "missing, re-read");

    // Accepted, then the new run fails.
    host.handle("packages.reload", (p) => {
      host.packageCalls.push({ method: "reload", id: String(p.id) });
      host.setPackages(host.packages.map((each) => (each.id === p.id ? { ...each, state: "starting", error: null } : each)));
      setTimeout(
        () => host.setPackages(host.packages.map((each) => (each.id === p.id ? { ...each, state: "failed", error: "boom" } : each))),
        20,
      );
    });
    window.emit("rowAct", "\nd.run", "reload");
    await until(host, () => status() === "无法重新加载「d.run」：boom" && controls(host, "d.run").badge === "失败", "failed restart");
    assert.deepEqual(reloads(host), ["d.run"]);
  } finally {
    await stop();
  }
});

test("warnings of a running plugin: a Warning tag with the reasons on hover, and notices in the details", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const held = {
    ...info("w.bar", { directory: packageDir("w.bar", false) }),
    warnings: [
      { code: "slot-occupied", slot: "bottom", owner: "vestra.video" },
      { code: "sequence-taken", owner: "x.gone" },
    ],
  } as Info;
  const { host, stop } = await start({ net, packages: [BUILTIN, held, info("w.calm")] });
  try {
    await openStore(host);
    await until(host, () => row(host, "w.bar")?.flags.length === 1, "tagged");
    assert.deepEqual(row(host, "w.bar")!.flags, [{ text: "警告", tone: 2 }]);
    assert.equal(
      row(host, "w.bar")!.tip,
      "它的底栏没有显示：底栏由「vestra.video」占用 · 它的上一个/下一个没有生效：由「x.gone」提供",
    );
    assert.deepEqual(row(host, "w.calm")!.flags, []);
    await choose(host, "w.bar");
    const notices = ["它的底栏没有显示：底栏由「vestra.video」占用", "它的上一个/下一个没有生效：由「x.gone」提供"];
    await until(host, () => notices.every((text) => banners(host).includes(text)), "notices");
    await choose(host, "w.calm");
    await until(host, () => !banners(host).includes(notices[0]!), "none for the other plugin");
    assert.ok(!banners(host).some((text) => notices.includes(text)));
  } finally {
    await stop();
  }
});

test("development loads: under Installed with a Dev tag; disable and enable for this run, never uninstall", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const dev = info("d.dev", { origin: "development", source: null, directory: packageDir("d.dev", false) });
  const { host, stop } = await start({ net, packages: [BUILTIN, dev] });
  try {
    await openStore(host);
    await until(host, () => row(host, "d.dev") !== undefined, "dev row");
    assert.deepEqual(row(host, "d.dev")!.flags, [{ text: "开发", tone: 1 }]);
    assert.equal(row(host, "d.dev")!.badge, "");
    assert.equal((row(host, "d.dev") as { meta?: string }).meta, "未知作者 · 开发加载");
    assert.equal(props(host)["installed-count"], 2);
    const shown = controls(host, "d.dev");
    assert.deepEqual(shown.primary, { id: "disable", text: "停用", enabled: true });
    assert.deepEqual(
      shown.menu.map((m) => m.id),
      ["reload", "openData"],
    );
    await choose(host, "d.dev");
    assert.ok(!actions(host).some((a) => a.id === "uninstall"), "a development load is not uninstalled from the store");
    assert.deepEqual(banners(host), ["本次运行以开发方式加载（--plugin）：停用只到播放器退出为止，也不能在这里卸载"], "not called sideloaded");

    host.window("store").emit("rowAct", "\nd.dev", "disable");
    await until(host, () => controls(host, "d.dev").dim && props(host)["status-text"] === "已停用「d.dev」，到播放器退出为止", "disabled");
    assert.equal(host.confirms.length, 0, "no question (user ruling)");
    assert.deepEqual(controls(host, "d.dev").primary, { id: "enable", text: "启用", enabled: true });
    assert.deepEqual(row(host, "d.dev")!.badge, "已停用");
    host.window("store").emit("rowAct", "\nd.dev", "enable");
    await until(host, () => !controls(host, "d.dev").dim && props(host)["status-text"] === "已启用「d.dev」", "enabled");
    assert.deepEqual(
      host.packageCalls.map((c) => [c.method, c.id, c.enabled]),
      [
        ["setEnabled", "d.dev", false],
        ["setEnabled", "d.dev", true],
      ],
    );
  } finally {
    await stop();
  }
});

test("a refused uninstall or a missing plugin: the store reads the list again", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, stop } = await start({ net, packages: [BUILTIN, info("u.gone"), info("u.busy")] });
  try {
    await openStore(host);
    await until(host, () => rows(host).length === 3, "rows");
    const window = host.window("store");

    // Uninstalled elsewhere; the store has not heard (no packages.changed).
    host.handle("packages.uninstall", () => {
      throw new HostError("missing", "no plugin u.gone");
    });
    host.packages = host.packages.filter((p) => p.id !== "u.gone");
    host.dialogAnswers.push(yes);
    window.emit("rowAct", "\nu.gone", "uninstall");
    await until(host, () => !row(host, "u.gone") && props(host)["status-text"] === "这个插件已不存在", "gone");

    // Refused while it was disabled elsewhere: the row follows the player's list.
    host.handle("packages.uninstall", () => {
      throw new HostError("busy", "busy");
    });
    host.packages = host.packages.map((p) => (p.id === "u.busy" ? { ...p, state: "disabled" } : p));
    host.dialogAnswers.push(yes);
    window.emit("rowAct", "\nu.busy", "uninstall");
    await until(host, () => controls(host, "u.busy").dim && props(host)["status-text"] === "正在安装或清理插件，请稍后再试", "re-read");
  } finally {
    await stop();
  }
});
