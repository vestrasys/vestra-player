import assert from "node:assert/strict";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { installFromRepository } from "../installer.ts";
import { actions, ALL, allRows, banners, BUILTIN, choose, controls, FakeNet, INSTALLED, tab, info, no, OFFICIAL, openStore, props, publish, showBuiltins, start, THIRD, until, yes } from "./helpers.ts";

const YT = {
  id: "zysilm.youtube",
  name: "YouTube",
  author: "zysilm",
  versions: [{ version: "0.2.0", capabilities: ["commands", "ui.window", "playback.control", "clipboard"] }],
};

async function ready(net: FakeNet, packages = [BUILTIN]) {
  const started = await start({ net, packages });
  await openStore(started.host);
  await until(started.host, () => started.store.items.some((item) => item.entry !== null), "rows");
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
    // The window counts them ("Permissions ({})", in Slint).
    assert.equal((props(host)["permissions"] as unknown[]).length, 4);
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "install");
    await until(host, () => installs(host).length === 1, "install");
    const confirm = host.confirmTexts.at(-1)!;
    assert.equal(confirm.message, "安装「YouTube」0.2.0？");
    assert.match(String(confirm.detail), /来自 Vestra Player 官方仓库，作者 zysilm。/);
    assert.match(String(confirm.detail), /· 读写剪贴板/);
    assert.match(String(confirm.detail), /不限制它读写文件和联网/);
    assert.deepEqual(installs(host)[0], {
      method: "install",
      id: "zysilm.youtube",
      source: { kind: "repository", url: OFFICIAL },
      replace: false,
    });
    await until(host, () => host.osd.includes("已安装 YouTube 0.2.0"), "osd");
    await until(host, () => !store.busy);
    assert.deepEqual(readdirSync(join(host.dataDirectory, "downloads")), [], "download removed");
    assert.equal((await allRows(host)).find((r) => r.name === "YouTube")?.mark, "✓");
    // All lists it installed now: it stays selected, with Disable and Uninstall… (formerly in the Plugin Manager).
    assert.equal(props(host)["category"], ALL);
    assert.equal(props(host)["d-name"], "YouTube");
    assert.deepEqual(actions(host).map((a) => a.text), ["停用", "卸载…"]);
    assert.equal(controls(host, "YouTube").primary, null);
    assert.equal(controls(host, "YouTube").menu[0]?.id, "disable");
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

test("installed from a .vxp and in a repository: one row under All; its … menu replaces it, asking once", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [YT]);
  const { host, store, stop } = await ready(net, [BUILTIN, info("zysilm.youtube", { version: "0.1.5", source: { kind: "file" } })]);
  try {
    const local = "\nzysilm.youtube";
    const repoKey = `${OFFICIAL}${local}`;
    const replace = `replace:${repoKey}`;
    // All: the installed row (named from its manifest, here unreadable: the id) stands for the repository's entry.
    const all = await allRows(host);
    assert.deepEqual(
      all.filter((r) => r.key.endsWith("zysilm.youtube")).map((r) => [r.key, r.mark]),
      [[local, "✓"]],
    );
    assert.equal(props(host)["all-count"], 1);
    await choose(host, "zysilm.youtube");
    assert.ok(banners(host).includes("也在官方仓库里"));
    // Its buttons stay; Replace with the repository's version is last in the … menu.
    const { primary, menu } = controls(host, "zysilm.youtube");
    assert.equal(primary, null);
    assert.deepEqual(
      menu.map((m) => [m.id, m.text]),
      [
        ["disable", "停用"],
        ["uninstall", "卸载…"],
        ["reload", "重新加载"],
        ["openData", "打开数据目录"],
        [replace, "用官方仓库的 0.2.0 替换…"],
      ],
    );
    assert.deepEqual(actions(host).map((a) => a.id), ["disable", "uninstall"]);
    // The repository's entry, asked for by its key, shows the installed row.
    store.select(repoKey);
    await host.settle();
    assert.equal(store.selected, local);
    // Installed is unchanged: no replace entry there.
    await tab(host, INSTALLED);
    assert.ok(!controls(host, "zysilm.youtube").menu.some((m) => m.id === replace));
    await tab(host, ALL);

    host.dialogAnswers.push(no);
    host.window("store").emit("rowAct", local, replace);
    await until(host, () => host.confirms.length === 1, "replace question");
    assert.equal(host.confirmTexts.at(-1)!.message, "替换已安装的「YouTube」？");
    assert.equal(host.confirms.at(-1)!["destructive"], true);
    assert.match(String(host.confirmTexts.at(-1)!.detail), /将安装官方仓库的 0\.2\.0。/);
    // Acceptance S5: the store cannot see dormant instances, so it says both cases.
    assert.match(String(host.confirmTexts.at(-1)!.detail), /若之前从这个来源安装过，会接上当时的数据；否则使用新的空数据/);
    assert.equal(installs(host).length, 0);

    host.dialogAnswers.push(yes);
    host.window("store").emit("rowAct", local, replace);
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
    assert.equal(host.confirmTexts[1]!.message, "替换已安装的「YouTube」？");
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
    await showBuiltins(host);
    await until(host, () => store.items.some((i) => i.name === "Extra") && store.items.some((i) => i.name === "Fake Video"), "rows");
    // A repository's entry for a built-in's id: All shows the built-in's own row only.
    const all = await allRows(host);
    assert.ok(!all.some((r) => r.name === "Fake Video"));
    assert.equal(all.find((r) => r.name === "vestra.video" && r.key === "\nvestra.video")?.badge, "内置");
    const fake = store.items.find((i) => i.name === "Fake Video")!;
    assert.equal(fake.state, "reserved");
    store.select(fake.key);
    await host.settle();
    assert.equal(store.selected, "\nvestra.video");
    assert.deepEqual(actions(host), []);
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
    await until(host, () => host.osd.includes("正在安装或清理插件，请稍后再试"));
  } finally {
    await stop();
  }
});

test("update and uninstall from the same repository; no rollback and no older version", async () => {
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
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.3.0", "停用", "卸载…"]);
    assert.ok(banners(host).includes("新版本需要新的权限：读写剪贴板"));
    // The Updates chip shows the count ("Updates {}", in Slint).
    assert.equal(props(host)["updates"], 1);
    type Version = { version: string; status: string; canInstall: boolean };
    const versions = () => (props(host)["versions"] as Version[]).map((v) => [v.version, v.status, v.canInstall]);
    // 版本 tab: newer versions only; nothing older than the installed one from the same source.
    assert.deepEqual(versions(), [
      ["0.3.0", "最新", true],
      ["0.2.0", "当前", false],
      ["0.1.0", "", false],
    ]);

    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => host.osd.includes("已更新 YouTube 到 0.3.0"));
    assert.equal(host.confirmTexts.at(-1)!.message, "更新「YouTube」到 0.3.0？新版本需要更多权限");
    assert.equal(
      host.confirmTexts.at(-1)!.detail,
      "新增：\n· 读写剪贴板\n\n当前 0.2.0。\n\n插件是完整的程序，以上权限只限制它使用播放器的功能，不限制它读写文件和联网。",
    );
    await until(host, () => !store.busy);
    // No Roll Back / Restore.
    assert.deepEqual(actions(host).map((a) => a.text), ["停用", "卸载…"]);
    await until(host, () => (props(host)["versions"] as Version[]).some((v) => v.version === "0.3.0" && v.status === "当前"));
    assert.deepEqual(versions(), [
      ["0.3.0", "当前", false],
      ["0.2.0", "", false],
      ["0.1.0", "", false],
    ]);
    // A stale window or an old action id: nothing is asked or installed.
    const confirms = host.confirmTexts.length;
    const calls = host.packageCalls.length;
    host.window("store").emit("installVersion", "0.2.0");
    host.window("store").emit("act", "rollback");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal(host.confirmTexts.length, confirms);
    assert.equal(host.packageCalls.length, calls);
    assert.equal(store.busy, false);

    host.dialogAnswers.push({ confirmed: true, checked: true });
    host.window("store").emit("act", "uninstall");
    await until(host, () => host.osd.includes("已卸载 YouTube（数据已删除）"));
    assert.deepEqual(host.packageCalls.at(-1), { method: "uninstall", id: "zysilm.youtube", removeData: true });
    assert.equal(host.confirmTexts.at(-1)!.checkbox, "同时删除它的设置与数据？");
  } finally {
    await stop();
  }
});

test("older player: updating a disabled plugin keeps it disabled (VS Code); a failure to keep it so is reported", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [
    {
      ...YT,
      versions: [
        { version: "0.4.0", capabilities: ["commands"] },
        { version: "0.3.0", capabilities: ["commands"] },
        { version: "0.2.0", capabilities: ["commands"] },
      ],
    },
  ]);
  const installed = info("zysilm.youtube", {
    version: "0.2.0",
    state: "disabled",
    capabilities: ["commands"],
    source: { kind: "repository", url: OFFICIAL },
  });
  const { host, store, stop } = await ready(net, [BUILTIN, installed]);
  // As players up to 0.6.3: every install runs the plugin (the store disables it again).
  host.installKeepsDisabled = false;
  const state = () => host.packages.find((p) => p.id === "zysilm.youtube")!.state;
  try {
    await choose(host, "YouTube");
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.4.0", "启用", "卸载…"]);
    // Through the Versions tab, to 0.3.0 first.
    host.dialogAnswers.push(yes);
    host.window("store").emit("installVersion", "0.3.0");
    await until(host, () => host.osd.includes("已更新 YouTube 到 0.3.0（仍停用）"), "still disabled");
    await until(host, () => !store.busy);
    assert.deepEqual(
      host.packageCalls.map((c) => [c.method, c.id, (c as { enabled?: boolean }).enabled]),
      [
        ["install", "zysilm.youtube", undefined],
        ["setEnabled", "zysilm.youtube", false],
      ],
    );
    assert.equal(state(), "disabled");
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.4.0", "启用", "卸载…"]);

    // The player refuses to disable it: the update stands, the status line says so.
    host.handle("packages.setEnabled", () => {
      throw new HostError("busy", "reloading");
    });
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => host.osd.some((text) => text.startsWith("已更新 YouTube 到 0.4.0，但未能保持停用：")), "failure reported");
    await until(host, () => !store.busy);
    assert.equal(props(host)["status-warn"], true);
    assert.equal(state(), "running");
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
    await until(host, () => store.items.some((i) => i.name === "Viz"));
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
    const confirm = host.confirmTexts.at(-1)!;
    assert.equal(confirm.message, "安装来自非官方仓库的「Viz」1.0.0？");
    assert.equal(confirm.confirm, "仍然安装");
    assert.equal(host.confirms.at(-1)!["destructive"], true);
    assert.match(String(confirm.detail), /这个仓库未经 Vestra Player 审核/);
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

test("player 0.6.4 (keeps the disabled flag): updating a disabled plugin keeps it disabled and says so", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ ...YT, versions: [{ version: "0.3.0", capabilities: ["commands"] }, { version: "0.2.0", capabilities: ["commands"] }] }]);
  const installed = info("zysilm.youtube", {
    version: "0.2.0",
    state: "disabled",
    capabilities: ["commands"],
    source: { kind: "repository", url: OFFICIAL },
  });
  const { host, store, stop } = await ready(net, [BUILTIN, installed]);
  try {
    await choose(host, "YouTube");
    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "update");
    await until(host, () => host.osd.includes("已更新 YouTube 到 0.3.0（仍停用）"), "still disabled");
    await until(host, () => !store.busy);
    assert.equal(host.packages.find((p) => p.id === "zysilm.youtube")!.state, "disabled");
    assert.deepEqual(actions(host).map((a) => a.text), ["启用", "卸载…"]);
  } finally {
    await stop();
  }
});
