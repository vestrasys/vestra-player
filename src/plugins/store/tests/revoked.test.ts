import assert from "node:assert/strict";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { actions, banners, BUILTIN, choose, FakeNet, info, no, OFFICIAL, openStore, props, publish, start, THIRD, until, yes } from "./helpers.ts";

const fromOfficial = (id: string, version: string) =>
  info(id, { version, source: { kind: "repository", url: OFFICIAL } });
const disables = (host: { packageCalls: readonly { method: string; enabled?: boolean }[] }) =>
  host.packageCalls.filter((c) => c.method === "setEnabled" && c.enabled === false).length;

function serve(net: FakeNet, revoked: { id: string; versions: string; reason: string }[]) {
  publish(
    net,
    OFFICIAL,
    [
      { id: "zysilm.youtube", name: "YouTube", versions: [{ version: "0.2.1" }, { version: "0.2.0" }, { version: "0.1.0" }] },
      { id: "zysilm.bilibili", name: "Bilibili", versions: [{ version: "0.2.0" }] },
    ],
    { revoked },
  );
}

test("a revoked version installed from the repository is disabled once, with one notice", async () => {
  const net = new FakeNet();
  serve(net, [
    { id: "zysilm.youtube", versions: ">=0.2.0 <0.2.1", reason: "会删除下载目录" },
    { id: "zysilm.bilibili", versions: "*", reason: "接口失效" },
  ]);
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, fromOfficial("zysilm.youtube", "0.2.0"), fromOfficial("zysilm.bilibili", "0.2.0")],
  });
  try {
    host.dialogAnswers.push(no);
    await store.refreshAll(true);
    await until(host, () => host.confirms.length === 1, "notice");
    assert.deepEqual(
      host.packageCalls.map((c) => [c.method, c.id, (c as { enabled?: boolean }).enabled]),
      [
        ["setEnabled", "zysilm.youtube", false],
        ["setEnabled", "zysilm.bilibili", false],
      ],
    );
    const notice = host.confirmTexts[0]!;
    assert.equal(notice.message, "已停用 2 个被撤回的插件");
    assert.match(String(notice.detail), /^仓库撤回了这些版本，原因：/);
    // The reasons are nested templates: English until the player translates nested arguments.
    assert.match(String(notice.detail), /“YouTube” 0\.2\.0: 会删除下载目录\nYou can update to 0\.2\.1 in the Plugin Store\./);
    assert.match(String(notice.detail), /“Bilibili” 0\.2\.0: 接口失效/);
    assert.equal(notice.confirm, "打开插件商城");
    assert.equal(notice.cancel, "知道了");

    await store.refreshAll(true);
    await host.settle(30);
    assert.equal(disables(host), 2, "not disabled again");
    assert.equal(host.confirms.length, 1);

    await openStore(host);
    const row = await choose(host, "YouTube");
    assert.equal(row.badge, "已撤回");
    assert.ok(banners(host).includes("此版本已被撤回：会删除下载目录。商城已停用它。"));
    assert.deepEqual(actions(host).map((a) => a.text), ["更新到 0.2.1", "仍然启用…", "卸载…"]);
    // The revoked version cannot be installed again from 版本.
    host.window("store").emit("setTab", 2);
    await host.settle();
    const versions = props(host)["versions"] as { version: string; status: string; canInstall: boolean }[];
    assert.deepEqual(versions[1], { version: "0.2.0", date: "2026-10-21", minimum: "0.2.0", status: "当前，已撤回：会删除下载目录", tone: 3, canInstall: false });
  } finally {
    await stop();
  }
});

test("仍然启用… asks about the risk; once accepted the store leaves it running", async () => {
  const net = new FakeNet();
  serve(net, [{ id: "zysilm.youtube", versions: "0.2.0", reason: "会删除下载目录" }]);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, fromOfficial("zysilm.youtube", "0.2.0")] });
  try {
    host.dialogAnswers.push(yes); // the notice: 打开插件商城
    await store.refreshAll(true);
    await until(host, () => store.selected === `${OFFICIAL}\nzysilm.youtube`, "store opened on the plugin");
    assert.equal(host.window("store").visible, true);

    host.dialogAnswers.push(no);
    const asked = host.confirms.length;
    host.window("store").emit("act", "enable");
    await until(host, () => host.confirms.length === asked + 1, "risk question");
    assert.equal(host.confirmTexts.at(-1)!.message, "仍然启用被撤回的「YouTube」0.2.0？");
    assert.equal(host.confirms.at(-1)!["destructive"], true);
    assert.match(String(host.confirmTexts.at(-1)!.detail), /继续使用可能有风险/);
    assert.equal(host.packages.find((p) => p.id === "zysilm.youtube")!.state, "disabled");

    host.dialogAnswers.push(yes);
    host.window("store").emit("act", "enable");
    await until(host, () => host.packages.find((p) => p.id === "zysilm.youtube")!.state === "running");
    await store.refreshAll(true);
    await host.settle(30);
    assert.equal(disables(host), 1);
    assert.equal(host.confirms.length, 3, "no question after accepting");
    assert.ok(banners(host).includes("此版本已被撤回：会删除下载目录。你已确认风险并重新启用，商城不会再停用它。"));
  } finally {
    await stop();
  }
});

test("enabled again elsewhere (插件管理): confirm the risk or it is disabled again", async () => {
  const net = new FakeNet();
  serve(net, [{ id: "zysilm.youtube", versions: "0.2.0", reason: "会删除下载目录" }]);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, fromOfficial("zysilm.youtube", "0.2.0")] });
  try {
    host.dialogAnswers.push(no);
    await store.refreshAll(true);
    await until(host, () => disables(host) === 1);

    // The user enables it in 插件管理 and then picks 「停用」.
    host.dialogAnswers.push(no);
    host.setPackages(host.packages.map((p) => (p.id === "zysilm.youtube" ? { ...p, state: "running" } : p)));
    await until(host, () => disables(host) === 2, "disabled again");
    assert.equal(host.confirmTexts.at(-1)!.message, "「YouTube」0.2.0 已被撤回，确定继续使用吗？");
    assert.equal(host.confirmTexts.at(-1)!.cancel, "停用");

    // Again, and this time 「继续使用」.
    host.dialogAnswers.push(yes);
    host.setPackages(host.packages.map((p) => (p.id === "zysilm.youtube" ? { ...p, state: "running" } : p)));
    await until(host, () => host.confirms.length === 3);
    await store.refreshAll(true);
    await host.settle(30);
    assert.equal(disables(host), 2);
    assert.equal(store.settings.value.revocations[JSON.stringify([OFFICIAL, "zysilm.youtube", "0.2.0", "0.2.0"])], "accepted");
  } finally {
    await stop();
  }
});

test("only the source repository revokes: not side-loads, not built-ins, not another repository", async () => {
  const net = new FakeNet();
  serve(net, [
    { id: "zysilm.youtube", versions: "*", reason: "x" },
    { id: "vestra.video", versions: "*", reason: "x" },
  ]);
  publish(net, THIRD, [{ id: "zysilm.bilibili", name: "Bili", versions: [{ version: "9.0.0" }] }], {
    revoked: [{ id: "zysilm.bilibili", versions: "*", reason: "hostile" }],
  });
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, info("zysilm.youtube", { version: "0.2.0", source: { kind: "file" } }), fromOfficial("zysilm.bilibili", "0.2.0")],
  });
  try {
    store.settings.save({ ...store.settings.value, repos: [THIRD] });
    store.repos.sync([THIRD]);
    await store.refreshAll(true);
    await host.settle(30);
    assert.equal(disables(host), 0);
    assert.equal(host.confirms.length, 0);
  } finally {
    await stop();
  }
});

test("disabling fails while an install runs: tried again on the next pass", async () => {
  const net = new FakeNet();
  serve(net, [{ id: "zysilm.youtube", versions: "0.2.0", reason: "x" }]);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, fromOfficial("zysilm.youtube", "0.2.0")] });
  try {
    host.handle("packages.setEnabled", () => {
      throw new HostError("busy", "installing");
    });
    await store.refreshAll(true);
    await host.settle(30);
    assert.equal(host.confirms.length, 0);
    assert.deepEqual(store.settings.value.revocations, {});
    host.reset("packages.setEnabled");
    host.dialogAnswers.push(no);
    await store.refreshAll(true);
    await until(host, () => disables(host) === 1);
  } finally {
    await stop();
  }
});

test("review A: a delisted plugin revoked with '*' is still disabled (and one whose entries are all unusable)", async () => {
  const net = new FakeNet();
  // The publisher drops a deleted plugin from `plugins` but keeps `revoked`.
  publish(net, OFFICIAL, [{ id: "zysilm.other", versions: [{ version: "1.0.0" }] }], {
    revoked: [
      { id: "zysilm.evil", versions: "*", reason: "恶意插件" },
      { id: "zysilm.broken", versions: "*", reason: "坏了" },
    ],
  });
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, fromOfficial("zysilm.evil", "0.1.0"), fromOfficial("zysilm.broken", "0.3.0"), fromOfficial("zysilm.gone", "1.0.0")],
  });
  try {
    host.dialogAnswers.push(no);
    await store.refreshAll(true);
    await until(host, () => disables(host) === 2, "both disabled");
    assert.match(String(host.confirmTexts[0]!.detail), /恶意插件/);
    const evil = store.items.find((i) => i.id === "zysilm.evil")!;
    assert.equal(evil.state, "revoked");
    assert.equal(evil.entry, null);
    await openStore(host);
    host.window("store").emit("select", evil.key);
    await host.settle();
    assert.ok(banners(host).includes("此版本已被撤回：恶意插件。商城已停用它。"));
    assert.ok(!banners(host).some((b) => b.includes("仓库已从商城移除")));
    assert.deepEqual(actions(host).map((a) => a.text), ["仍然启用…", "卸载…"]);
    // Delisted without a revocation: says so truthfully.
    host.window("store").emit("select", "\nzysilm.gone");
    await host.settle();
    assert.ok(banners(host).includes("仓库已不再提供这个插件，商城不再为它检查更新（仍会检查撤回）"));
  } finally {
    await stop();
  }
});
