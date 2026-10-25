import assert from "node:assert/strict";
import { cpSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { banners, BUILTIN, choose, FakeNet, info, no, OFFICIAL, openStore, pageProps, publish, rows, start, THIRD, until, yes } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-settings-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

type RepoRow = { name: string; url: string; note: string; removable: boolean };

test("the settings page: add a third-party repository with a warning, refuse bad ones, remove it", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  publish(net, THIRD, [{ id: "someone.viz", name: "Viz", versions: [{ version: "1.0.0" }] }], { extra: { name: "Foo 仓库" } });
  const data = join(scratch, "data");
  const first = await start({
    net,
    dataDirectory: data,
    packages: [BUILTIN, info("someone.viz", { version: "1.0.0", source: { kind: "repository", url: THIRD } })],
  });
  const { host, store } = first;
  const page = host.settingsPage("store.repos");
  assert.equal(page.category, "plugins");
  assert.equal(page.title, "插件商城");
  assert.equal(pageProps(host)["auto-check"], true);
  assert.deepEqual((pageProps(host)["repos"] as RepoRow[]).map((r) => [r.name, r.removable]), [["Vestra Player 官方仓库", false]]);

  page.emit("startAdd");
  await host.settle();
  assert.equal(pageProps(host)["adding"], true);
  page.emit("add", "http://foo.example/");
  await host.settle();
  assert.equal(pageProps(host)["message"], "请输入 https:// 开头的仓库地址");
  page.emit("add", "https://plugins.test/");
  await host.settle();
  assert.equal(pageProps(host)["message"], "这个仓库已经添加过了");
  page.emit("add", "https://nothing.example/");
  await until(host, () => String(pageProps(host)["message"]).startsWith("无法读取这个仓库"));
  assert.equal(host.confirms.length, 0);

  host.dialogAnswers.push(no);
  page.emit("add", "https://third.example/repo/");
  await until(host, () => host.confirms.length === 1);
  assert.equal(host.confirmTexts[0]!.message, "添加非官方仓库「Foo 仓库」？");
  assert.match(String(host.confirmTexts[0]!.detail), /未经 Vestra Player 审核，可能读取你的文件或联网/);
  await host.settle(20);
  assert.deepEqual(store.settings.value.repos, []);

  host.dialogAnswers.push(yes);
  page.emit("add", "https://third.example/repo/");
  // "{} ({})" has no words, so extract (SDK 1.5.11) leaves it out of the catalog and the player
  // formats the English parentheses (was “Foo 仓库（third.example）”).
  await until(host, () => pageProps(host)["message"] === "已添加 Foo 仓库 (third.example)");
  assert.deepEqual(store.settings.value.repos, [THIRD]);
  assert.equal(pageProps(host)["adding"], false);
  assert.deepEqual(
    (pageProps(host)["repos"] as RepoRow[]).map((r) => [r.name, r.url, r.removable]),
    [
      ["Vestra Player 官方仓库", OFFICIAL, false],
      ["非官方：Foo 仓库（third.example）", THIRD, true],
    ],
  );
  page.emit("toggleAuto");
  await until(host, () => pageProps(host)["auto-check"] === false, "automatic check off");
  await first.stop();

  // Settings survive a restart.
  const copy = join(scratch, "copy");
  cpSync(data, copy, { recursive: true });
  const second = await start({
    net,
    dataDirectory: copy,
    packages: [BUILTIN, info("someone.viz", { version: "1.0.0", source: { kind: "repository", url: THIRD } })],
  });
  try {
    assert.deepEqual(second.store.settings.value.repos, [THIRD]);
    assert.equal(second.store.settings.value.autoCheck, false);
    assert.equal(second.store.repos.find(THIRD)?.index?.plugins.length, 1, "its cache is loaded");
    await openStore(second.host);
    assert.equal(rows(second.host).find((r) => r.name === "Viz")?.badge, "已安装");

    second.host.dialogAnswers.push(yes);
    second.host.settingsPage("store.repos").emit("remove", THIRD);
    await until(second.host, () => second.store.settings.value.repos.length === 0);
    assert.equal(second.host.confirmTexts.at(-1)!.message, "移除仓库「Foo 仓库（third.example）」？");
    assert.match(String(second.host.confirmTexts.at(-1)!.detail), /从它安装的插件会保留/);
    // What was installed from it stays, as a local row without update checks.
    await until(second.host, () => rows(second.host).some((r) => r.key === "\nsomeone.viz"));
    await choose(second.host, "someone.viz");
    assert.ok(banners(second.host).includes("它的仓库已从商城移除，商城不再为它检查更新和撤回"));
  } finally {
    await second.stop();
  }
});

test("at most 16 third-party repositories", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, []);
  const { host, store, stop } = await start({ net });
  try {
    const many = Array.from({ length: 16 }, (_, i) => `https://r${i}.example/index.json`);
    store.settings.save({ ...store.settings.value, repos: many });
    store.repos.sync(many);
    store.changed();
    await host.settle();
    const page = host.settingsPage("store.repos");
    assert.equal(pageProps(host)["can-add"], false);
    page.emit("add", "https://r99.example/");
    await host.settle();
    assert.equal(pageProps(host)["message"], "最多添加 16 个仓库");
  } finally {
    await stop();
  }
});
