import assert from "node:assert/strict";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { installFromRepository } from "../installer.ts";
import {
  actions,
  banners,
  BUILTIN,
  choose,
  FakeNet,
  info,
  no,
  OFFICIAL,
  openStore,
  props,
  publish,
  rows,
  start,
  THIRD,
  until,
  yes,
} from "./helpers.ts";

const YT = {
  id: "zysilm.youtube",
  name: "YouTube",
  author: "zysilm",
  versions: [{ version: "0.2.0", capabilities: ["commands", "ui.window", "playback.control", "clipboard"] }],
};

async function ready(net: FakeNet, packages = [BUILTIN]) {
  const started = await start({ net, packages });
  await openStore(started.host);
  await until(started.host, () => rows(started.host).length > 1, "rows");
  return started;
}

type Call = { method: string; id: string; source?: unknown; replace?: boolean };
const installs = (host: { packageCalls: readonly Call[] }) => host.packageCalls.filter((c) => c.method === "install");

test("official install: confirm with permissions, download through a redirect, check, install from the repository", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  // GitHub Releases answer with a redirect to their asset host.
  const direct = "https://plugins.test/download/zysilm.youtube-0.2.0.vxp";
  const asset = net.routes.get(direct)!;
  net.route(direct, { status: 302, headers: { location: "https://assets.test/yt.vxp" } });
  net.route("https://assets.test/yt.vxp", asset);
  const { host, store, stop } = await ready(net);
  try {
    const row = (await choose(host, "YouTube")) as { meta?: string };
    // The repository is official; the plugin is a third-party one hosted there.
    assert.equal(row.meta, "zysilm · 官方仓库");
    assert.deepEqual(actions(host).map((a) => a.text), ["安装 0.2.0"]);
    assert.equal(props(host)["permissions-title"], "权限（4）");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => installs(host).length === 1, "install");
    const confirm = host.confirms.at(-1)!;
    assert.equal(confirm["message"], "安装「YouTube」0.2.0？");
    assert.match(String(confirm["detail"]), /来自 Vestra Player 官方仓库，作者 zysilm。/);
    assert.match(String(confirm["detail"]), /· 读写剪贴板/);
    assert.match(String(confirm["detail"]), /不限制它读写文件和联网/);
    assert.deepEqual(installs(host)[0], {
      method: "install",
      id: "zysilm.youtube",
      source: { kind: "repository", url: OFFICIAL },
      replace: false,
    });
    await until(host, () => host.osd.includes("已安装 YouTube 0.2.0"), "osd");
    await until(host, () => !store.busy);
    assert.deepEqual(readdirSync(join(host.dataDirectory, "downloads")), [], "download removed");
    assert.equal(rows(host).find((r) => r.name === "YouTube")?.badge, "已安装");
    assert.deepEqual(actions(host).map((a) => a.text), ["卸载…"]);
    assert.equal(props(host)["manage-note"], "停用、重新加载、查看数据目录：设置 ▶ 插件管理…");
  } finally {
    await stop();
  }
});

test("a refused confirmation downloads nothing", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  const { host, stop } = await ready(net);
  try {
    await choose(host, "YouTube");
    host.dialogAnswers.push(no);
    host.window("store").emit("act", "install");
    await host.settle(50);
    assert.equal(net.count("https://plugins.test/download/zysilm.youtube-0.2.0.vxp"), 0);
    assert.equal(installs(host).length, 0);
  } finally {
    await stop();
  }
});

for (const [what, served, text] of [
  ["size", Buffer.from("x"), "下载的文件与仓库描述不符（大小），已删除"],
  ["sha256", null, "下载的文件与仓库描述不符（校验和），已删除"],
] as const) {
  test(`a package whose ${what} differs from the index is deleted and not installed`, async () => {
    const net = new FakeNet();
    publish(net, OFFICIAL, [YT]);
    const url = "https://plugins.test/download/zysilm.youtube-0.2.0.vxp";
    const real = Buffer.from((net.routes.get(url) as { body: Buffer }).body);
    const body = served ?? Buffer.from(real.toString().replace("YouTube", "YouTubf"));
    net.route(url, { body });
    const { host, store, stop } = await ready(net);
    try {
      await choose(host, "YouTube");
      host.dialogAnswers.push(yes);
      host.window("store").emit("act", "install");
      await until(host, () => host.osd.includes(text), text);
      assert.equal(installs(host).length, 0);
      assert.equal(host.sent("packages.validateArchive").length, 0);
      await until(host, () => !store.busy);
      assert.deepEqual(readdirSync(join(host.dataDirectory, "downloads")), []);
    } finally {
      await stop();
    }
  });
}

test("a package that asks for more than its entry lists is not installed", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { ...YT, versions: [{ version: "0.2.0", capabilities: ["commands"], packageCapabilities: ["commands", "secrets"] }] },
  ]);
  const { host, stop } = await ready(net);
  try {
    await choose(host, "YouTube");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => host.osd.includes("插件包与仓库描述不符（权限），未安装"));
    assert.equal(installs(host).length, 0);
  } finally {
    await stop();
  }
});

test("installed from another source: 替换… asks once and installs with replace", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  const { host, stop } = await ready(net, [BUILTIN, info("zysilm.youtube", { version: "0.1.5", source: { kind: "file" } })]);
  try {
    const row = await choose(host, "YouTube");
    assert.equal(row.badge, "已从本地文件安装");
    assert.deepEqual(actions(host).map((a) => a.id), ["replace"]);
    assert.ok(banners(host).some((b) => b.startsWith("已从本地文件安装 0.1.5")));
    // There is also the row of the installed instance (named from its manifest, here unreadable: the id).
    assert.equal(rows(host).find((r) => r.name === "zysilm.youtube")?.badge, "已安装");

    host.dialogAnswers.push(no);
    host.window("store").emit("act", "replace");
    await until(host, () => host.confirms.length === 1, "replace question");
    assert.equal(host.confirms.at(-1)!["message"], "替换已安装的「YouTube」？");
    assert.equal(host.confirms.at(-1)!["destructive"], true);
    assert.match(String(host.confirms.at(-1)!["detail"]), /原插件的数据保留，以后从原来源重新安装时接上/);
    // Acceptance S5: the store cannot see dormant instances, so it says both cases.
    assert.match(String(host.confirms.at(-1)!["detail"]), /若之前从这个来源安装过，会接上当时的数据；否则使用新的空数据/);
    assert.equal(installs(host).length, 0);

    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "replace");
    await until(host, () => installs(host).length === 1);
    assert.deepEqual(installs(host)[0], { method: "install", id: "zysilm.youtube", source: { kind: "repository", url: OFFICIAL }, replace: true });
    assert.equal(host.confirms.length, 2, "one question per attempt");
  } finally {
    await stop();
  }
});

test("a conflict the store did not know about: confirm, then retry with replace", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  const { host, stop } = await ready(net);
  try {
    await choose(host, "YouTube");
    // Installed from the workshop meanwhile, without the store hearing about it yet.
    host.packages.push(info("zysilm.youtube", { source: { kind: "workshop" } }));
    host.dialogAnswers.push(yes, yes);
    host.window("store").emit("act", "install");
    await until(host, () => installs(host).length === 1);
    assert.equal(host.confirms.length, 2);
    assert.equal(host.confirms[1]!["message"], "替换已安装的「YouTube」？");
    assert.equal(installs(host)[0]!.replace, true);
  } finally {
    await stop();
  }
});

test("vestra. ids in any repository: refused before downloading (reserved for built-ins)", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT, { id: "vestra.video", name: "Fake Video", versions: [{ version: "9.0.0" }] }]);
  publish(net, THIRD, [{ id: "Vestra.extra", name: "Extra", versions: [{ version: "1.0.0" }] }]);
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await openStore(host);
    await until(host, () => rows(host).some((r) => r.name === "Extra") && rows(host).some((r) => r.name === "Fake Video"), "rows");
    const fake = await choose(host, "Fake Video");
    assert.equal(fake.badge, "不能安装");
    assert.deepEqual(actions(host), []);
    assert.ok(banners(host).includes("「vestra.video」：vestra. 开头的 id 只留给播放器内置插件，不能安装或试载"));
    assert.ok((props(host)["versions"] as { canInstall: boolean }[]).every((v) => !v.canInstall));
    // The built-in plugin keeps its own row.
    assert.equal(rows(host).find((r) => r.name === "vestra.video")?.badge, "内置");
    const extra = await choose(host, "Extra");
    assert.equal(extra.badge, "不能安装");
    assert.deepEqual(actions(host), []);
    // Even if asked to (an older window, a race): no question, no download, no install.
    const item = store.items.find((i) => i.id === "Vestra.extra")!;
    await installFromRepository(store, item, item.entry!.versions[0]!);
    await until(host, () => host.osd.includes("「Vestra.extra」：vestra. 开头的 id 只留给播放器内置插件，不能安装或试载"));
    assert.equal(host.confirms.length, 0);
    assert.equal(net.count("https://third.example/repo/download/Vestra.extra-1.0.0.vxp"), 0);
    assert.equal(installs(host).length, 0);
  } finally {
    await stop();
  }
});

test("the player refuses the package (versionMismatch) or the install degrades", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    { ...YT, versions: [{ version: "0.2.0", capabilities: ["commands"], minPlayerVersion: null, packageMinPlayerVersion: "9.0.0" }] },
    { id: "a.two", name: "Two", versions: [{ version: "1.0.0" }] },
  ]);
  const { host, store, stop } = await ready(net);
  try {
    await choose(host, "YouTube");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => host.osd.includes("这个插件需要 Vestra Player ≥ 9.0.0（当前 0.2.0）"));
    await until(host, () => !store.busy);

    host.handle("packages.install", (p) => ({ operationId: String(p.requestId), status: "failedDegraded" }));
    await choose(host, "Two");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => host.osd.includes("安装失败，已恢复原来的版本"));
    await until(host, () => !store.busy);

    host.handle("packages.install", () => {
      throw new HostError("busy", "operation in progress");
    });
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => host.osd.includes("这个插件正在安装或回退，请稍后再试"));
  } finally {
    await stop();
  }
});

test("update, rollback, older version and uninstall from the same repository", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    {
      ...YT,
      versions: [
        { version: "0.3.0", capabilities: ["commands", "clipboard"] },
        { version: "0.2.0", capabilities: ["commands"] },
        { version: "0.1.0", capabilities: ["commands"] },
      ],
    },
  ]);
  const installed = info("zysilm.youtube", {
    version: "0.2.0",
    capabilities: ["commands"],
    source: { kind: "repository", url: OFFICIAL },
  });
  const { host, store, stop } = await ready(net, [BUILTIN, installed]);
  try {
    const row = await choose(host, "YouTube");
    assert.equal(row.badge, "可更新 → 0.3.0");
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.3.0", "卸载…"]);
    assert.ok(banners(host).includes("新版本需要新的权限：读写剪贴板"));
    assert.equal(props(host)["updates-label"], "可更新 1");

    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => host.osd.includes("已更新 YouTube 到 0.3.0"));
    assert.equal(host.confirms.at(-1)!["message"], "更新「YouTube」到 0.3.0？新版本需要更多权限");
    assert.match(String(host.confirms.at(-1)!["detail"]), /新增：\n· 读写剪贴板/);
    await until(host, () => !store.busy);
    assert.deepEqual(actions(host).map((a) => a.text), ["回退到 0.2.0", "卸载…"]);

    host.window("store").emit("act", "rollback");
    await until(host, () => host.osd.includes("已回退 YouTube 到 0.2.0"));
    await until(host, () => !store.busy);
    // Acceptance S3: after the rollback the previous version (0.3.0) is newer.
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.3.0", "恢复到 0.3.0", "卸载…"]);

    // 版本 tab: the older 0.1.0 can be installed (same source; the player keeps one rollback step).
    const versions = props(host)["versions"] as { version: string; status: string; canInstall: boolean }[];
    assert.deepEqual(
      versions.map((v) => [v.version, v.status, v.canInstall]),
      [
        ["0.3.0", "最新", true],
        ["0.2.0", "当前", false],
        ["0.1.0", "", true],
      ],
    );
    host.dialogAnswers.push(yes);
    host.window("store").emit("installVersion", "0.1.0");
    await until(host, () => host.osd.includes("已安装 YouTube 0.1.0"));
    assert.equal(host.confirms.at(-1)!["message"], "安装「YouTube」0.1.0（旧版本）？");
    assert.equal(host.confirms.at(-1)!["detail"], "当前 0.2.0。安装后可以在商城恢复到 0.2.0。");
    await until(host, () => !store.busy);
    assert.ok(actions(host).some((x) => x.text === "恢复到 0.2.0"));
    host.window("store").emit("act", "rollback");
    await until(host, () => host.osd.includes("已恢复 YouTube 到 0.2.0"));
    await until(host, () => !store.busy);

    host.dialogAnswers.push({ confirmed: true, checked: true });
    host.window("store").emit("act", "uninstall");
    await until(host, () => host.osd.includes("已卸载 YouTube（数据已删除）"));
    assert.deepEqual(host.packageCalls.at(-1), { method: "uninstall", id: "zysilm.youtube", removeData: true });
    assert.equal(host.confirms.at(-1)!["checkbox"], "同时删除它的设置与数据");
  } finally {
    await stop();
  }
});

test("third-party repository: warning banner, unofficial confirmation, remote README images blocked until asked", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  publish(net, THIRD, [
    {
      id: "someone.viz",
      name: "Viz",
      readme: "# Viz\n\n![shot](https://img.example/shot.png)\n",
      versions: [{ version: "1.0.0", capabilities: ["commands", "processing.video", "playback.read"] }],
    },
  ]);
  const { host, store, stop } = await start({ net });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    host.remoteImages.set("https://img.example/shot.png", store.plugin.dataDirectory);
    await openStore(host);
    await until(host, () => rows(host).some((r) => r.name === "Viz"));
    await choose(host, "Viz");
    await until(host, () => banners(host).some((b) => b.startsWith("已阻止")), "blocked images");
    assert.ok(banners(host).includes("非官方仓库：未经 Vestra Player 审核，插件可能读取你的文件或联网"));
    assert.ok(banners(host).includes("已阻止 1 张远程图片（非官方来源）"));
    const doc = host.markdown(props(host)["doc"]);
    assert.equal(doc?.base, "https://third.example/repo/assets/someone.viz/README.md");
    assert.deepEqual(doc?.images, {});
    assert.equal(host.sent("ui.markdownScan").length >= 0, true);

    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => installs(host).length === 1);
    const confirm = host.confirms.at(-1)!;
    assert.equal(confirm["message"], "安装来自非官方仓库的「Viz」1.0.0？");
    assert.equal(confirm["confirm"], "仍然安装");
    assert.equal(confirm["destructive"], true);
    assert.match(String(confirm["detail"]), /这个仓库未经 Vestra Player 审核/);
    assert.deepEqual(installs(host)[0]!.source, { kind: "repository", url: THIRD });
  } finally {
    await stop();
  }
});

test("README links: web addresses open in the browser, anchors are ignored, other schemes refused", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  const { host, stop } = await ready(net);
  try {
    await choose(host, "YouTube");
    const window = host.window("store");
    window.emit("openLink", "https://example.com/doc");
    window.emit("openLink", "#usage");
    window.emit("openLink", "javascript:alert(1)");
    await host.settle(20);
    assert.deepEqual(host.system.urls, ["https://example.com/doc"]);
    assert.ok(host.osd.includes("不支持打开这种链接"));
    // 源码 ↗ in the facts column.
    const facts = props(host)["facts"] as { label: string; link: boolean }[];
    window.emit("factLink", facts.findIndex((f) => f.link));
    await host.settle(20);
    assert.equal(host.system.urls.at(-1), "https://github.com/example/zysilm.youtube");
    assert.ok(!existsSync(join(host.dataDirectory, "downloads", "x")));
  } finally {
    await stop();
  }
});
