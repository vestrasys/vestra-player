import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { BUILTIN, FakeNet, info, no, OFFICIAL, publish, start, until, vxp, yes } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-sideload-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

function packageFile(name: string, manifest: Parameters<typeof vxp>[0]) {
  const path = join(scratch, name);
  writeFileSync(path, vxp(manifest));
  return path;
}

const installs = (host: { packageCalls: readonly { method: string }[] }) => host.packageCalls.filter((c) => c.method === "install");

test("a .vxp dropped on the video area: the window opens, a side-load warning, install as a file", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({ net });
  try {
    assert.deepEqual(host.dropProviders.map((p) => p.extensions), [["vxp"]]);
    const a = packageFile("a.vxp", { id: "someone.a", version: "1.0.0", name: "A", capabilities: ["commands", "bindings.manage"] });
    const b = packageFile("b.vxp", { id: "someone.b", version: "2.0.0", capabilities: [] });
    host.dialogAnswers.push(yes, no);
    await host.drop([a, b]);
    await until(host, () => host.confirms.length === 2 && !store.busy, "two confirmations");
    assert.equal(host.window("store").visible, true);
    assert.equal(host.confirmTexts[0]!.message, "安装插件包「A」1.0.0？");
    assert.equal(host.confirmTexts[0]!.confirm, "仍然安装");
    assert.equal(host.confirms[0]!["destructive"], true);
    assert.match(String(host.confirmTexts[0]!.detail), /来自文件 a\.vxp，未经任何仓库审核，也不会自动更新/);
    assert.doesNotMatch(String(host.confirmTexts[0]!.detail), /· 修改所有快捷键/);
    assert.equal(host.confirmTexts[1]!.message, "安装插件包「someone.b」2.0.0？");
    assert.deepEqual(installs(host), [{ method: "install", id: "someone.a", source: { kind: "file" }, replace: false }]);
    await until(host, () => host.osd.includes("已安装 A 1.0.0"));
    assert.equal(store.selected, "\nsomeone.a");
  } finally {
    await stop();
  }
});

test("从文件安装…: cancel, an invalid file, a built-in id, replacing a repository install", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({
    net,
    packages: [BUILTIN, info("zysilm.youtube", { version: "0.2.0", source: { kind: "repository", url: OFFICIAL } })],
  });
  try {
    await host.command("open").run();
    const window = host.window("store");
    host.dialogAnswers.push(null);
    window.emit("fromFile");
    await host.settle(20);
    assert.equal(host.confirms.length, 0);

    const junk = join(scratch, "junk.vxp");
    writeFileSync(junk, "not a package");
    host.dialogAnswers.push([junk]);
    window.emit("fromFile");
    await until(host, () => host.osd.some((t) => t.startsWith("这不是有效的插件包")));
    await until(host, () => !store.busy);

    host.dialogAnswers.push([packageFile("video.vxp", { id: "vestra.video", version: "9.0.0" })]);
    window.emit("fromFile");
    await until(host, () => host.osd.includes("「vestra.video」：vestra. 开头的 id 只留给播放器内置插件，不能安装或试载"));
    await until(host, () => !store.busy);

    host.dialogAnswers.push([packageFile("yt.vxp", { id: "zysilm.youtube", version: "0.2.1", name: "YouTube" })], yes);
    window.emit("fromFile");
    await until(host, () => installs(host).length === 1);
    assert.equal(host.confirmTexts.at(-1)!.message, "用插件包「YouTube」0.2.1 替换已安装的版本？");
    assert.match(String(host.confirmTexts.at(-1)!.detail), /来自仓库 plugins\.test。替换后，若之前从这个来源安装过，会接上当时的数据；否则使用新的空数据/);
    assert.deepEqual(installs(host)[0], { method: "install", id: "zysilm.youtube", source: { kind: "file" }, replace: true });
  } finally {
    await stop();
  }
});

test("从链接安装…: https only, 64 MB at most, the permissions come from the package", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  net.route("https://example.org/x-1.0.0.vxp", { body: vxp({ id: "someone.x", version: "1.0.0", capabilities: ["secrets"] }) });
  net.route("https://example.org/huge.vxp", { headers: { "content-length": String(65 * 1024 * 1024) }, body: "" });
  const { host, store, stop } = await start({ net });
  try {
    await host.command("open").run();
    const window = host.window("store");
    window.emit("fromLink");
    await host.settle();
    assert.equal(window.props["link-open"], true);

    window.emit("linkAccepted", "http://example.org/x.vxp");
    await until(host, () => host.osd.includes("只能从 https:// 链接安装"));

    window.emit("linkAccepted", "https://example.org/huge.vxp");
    await until(host, () => host.osd.includes("插件包超过 64 MB，已停止下载"));
    await until(host, () => !store.busy);

    host.dialogAnswers.push(yes);
    window.emit("linkAccepted", "https://example.org/x-1.0.0.vxp");
    await until(host, () => installs(host).length === 1);
    assert.match(String(host.confirmTexts.at(-1)!.detail), /来自链接 https:\/\/example\.org\/x-1\.0\.0\.vxp/);
    assert.doesNotMatch(String(host.confirmTexts.at(-1)!.detail), /· 在系统钥匙串保存登录信息/);
    assert.deepEqual(installs(host)[0], {
      method: "install",
      id: "someone.x",
      source: { kind: "link", url: "https://example.org/x-1.0.0.vxp" },
      replace: false,
    });
    await until(host, () => !store.busy);
    assert.deepEqual(readdirSync(join(host.dataDirectory, "downloads")), []);
    assert.equal(window.props["link-open"], false);
  } finally {
    await stop();
  }
});

const setEnabledCalls = (host: { packageCalls: readonly { method: string; enabled?: boolean }[] }) =>
  host.packageCalls.filter((c) => c.method === "setEnabled").map((c) => c.enabled);

// Both players: one up to 0.6.3 turns a plugin on when it installs it (`installKeepsDisabled = false`);
// 0.6.4 keeps its disabled flag (the TestHost default). Either way a disabled plugin stays disabled,
// and the status line says what the player reports after the install.
for (const player of ["older player", "player 0.6.4"] as const) {
  const newer = player !== "older player";

  test(`${player}: reinstalling a disabled plugin from a file keeps it disabled`, async () => {
    const net = new FakeNet();
    publish(net, OFFICIAL, []);
    const installed = info("someone.a", { version: "1.0.0", state: "disabled", source: { kind: "file" } });
    const { host, store, stop } = await start({ net, packages: [BUILTIN, installed] });
    host.installKeepsDisabled = newer;
    const state = () => host.packages.find((p) => p.id === "someone.a")!.state;
    try {
      await host.command("open").run();
      host.dialogAnswers.push([packageFile("a-1.1.0.vxp", { id: "someone.a", version: "1.1.0", name: "A" })], yes);
      host.window("store").emit("fromFile");
      await until(host, () => host.osd.includes("已更新 A 到 1.1.0（仍停用）"), "still disabled");
      await until(host, () => !store.busy);
      assert.equal(state(), "disabled");
      assert.deepEqual(installs(host), [{ method: "install", id: "someone.a", source: { kind: "file" }, replace: false }]);
      // Disabled again for an older player; harmless (already disabled) on a newer one.
      assert.deepEqual(setEnabledCalls(host), [false]);

      // The same version again (a reinstall).
      host.dialogAnswers.push([packageFile("a-1.1.0-again.vxp", { id: "someone.a", version: "1.1.0", name: "A" })], yes);
      host.window("store").emit("fromFile");
      await until(host, () => host.osd.includes("已安装 A 1.1.0（仍停用）"), "reinstall still disabled");
      await until(host, () => !store.busy);
      assert.equal(state(), "disabled");
    } finally {
      await stop();
    }
  });

  test(`${player}: a link replacing a disabled plugin from a repository keeps it disabled`, async () => {
    const net = new FakeNet();
    publish(net, OFFICIAL, []);
    const url = "https://example.org/x-1.0.0.vxp";
    net.route(url, { body: vxp({ id: "someone.x", version: "1.0.0", name: "X", capabilities: [] }) });
    const installed = info("someone.x", { version: "0.9.0", state: "disabled", source: { kind: "repository", url: OFFICIAL } });
    const { host, store, stop } = await start({ net, packages: [BUILTIN, installed] });
    host.installKeepsDisabled = newer;
    const state = () => host.packages.find((p) => p.id === "someone.x")!.state;
    try {
      await host.command("open").run();
      host.dialogAnswers.push(yes);
      host.window("store").emit("linkAccepted", url);
      await until(host, () => host.osd.includes("已安装 X 1.0.0（仍停用）"), "still disabled");
      await until(host, () => !store.busy);
      assert.equal(host.confirmTexts.at(-1)!.message, "用插件包「X」1.0.0 替换已安装的版本？");
      assert.deepEqual(installs(host), [{ method: "install", id: "someone.x", source: { kind: "link", url }, replace: true }]);
      assert.deepEqual(setEnabledCalls(host), [false]);
      assert.equal(state(), "disabled");
    } finally {
      await stop();
    }
  });

  test(`${player}: a first install from a file runs`, async () => {
    const net = new FakeNet();
    publish(net, OFFICIAL, []);
    const { host, store, stop } = await start({ net });
    host.installKeepsDisabled = newer;
    try {
      await host.command("open").run();
      host.dialogAnswers.push([packageFile("b-1.0.0.vxp", { id: "someone.b", version: "1.0.0", name: "B" })], yes);
      host.window("store").emit("fromFile");
      await until(host, () => host.osd.includes("已安装 B 1.0.0"), "installed");
      await until(host, () => !store.busy);
      assert.equal(host.packages.find((p) => p.id === "someone.b")!.state, "running");
      assert.deepEqual(setEnabledCalls(host), []);
    } finally {
      await stop();
    }
  });
}

test("the disabled state is read again when the install starts (the store's list may be old)", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, info("someone.a", { version: "1.0.0", source: { kind: "file" } })] });
  try {
    await host.command("open").run();
    // Disabled behind the store's back: no packages.changed reaches it.
    host.packages = host.packages.map((p) => (p.id === "someone.a" ? { ...p, state: "disabled" } : p));
    assert.equal(store.packages.find((p) => p.id === "someone.a")!.state, "running");
    host.dialogAnswers.push([packageFile("a-1.0.1.vxp", { id: "someone.a", version: "1.0.1", name: "A" })], yes);
    host.window("store").emit("fromFile");
    await until(host, () => host.osd.includes("已更新 A 到 1.0.1（仍停用）"), "still disabled");
    await until(host, () => !store.busy);
    assert.equal(host.packages.find((p) => p.id === "someone.a")!.state, "disabled");
  } finally {
    await stop();
  }
});
