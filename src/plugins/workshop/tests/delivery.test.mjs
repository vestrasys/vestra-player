// The result card and the agent's install tool on the SDK test host: a
// scaffolded project shows on the card; Install (the agent's install asks
// first; there is no undo), Export .vxp, Export Source .zip. The player side of
// validation is simulated with host handlers.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { dataDirectory, server, startAi } from "./fixture.mjs";
import { shadowedText } from "./texts.mjs";
import { unzip } from "./unzip.mjs";

/** The model: per user message, a list of steps (one per tool result so far in that turn). */
function scripted(turns) {
  return (req) => {
    const messages = req.body.messages;
    const lastUser = messages.findLastIndex((m) => m.role === "user");
    const prompt = typeof messages[lastUser].content === "string" ? messages[lastUser].content : messages[lastUser].content.map((p) => p.text).join("");
    const done = messages.slice(lastUser).filter((m) => m.role === "tool").length;
    return turns[prompt]?.[done] ?? { text: ["好了。"] };
  };
}

const toolText = (mock, n) => {
  const content = mock.chats[n].body.messages.at(-1).content;
  return typeof content === "string" ? content : JSON.stringify(content);
};

/** The player's side: validation by file (or a validation error), install, uninstall. */
function player(ai, { invalid = null } = {}) {
  const { host } = ai;
  host.handle("packages.validateArchive", async ({ file }) => {
    if (invalid) throw Object.assign(new Error(invalid), { code: "invalid" });
    // The manifest of the packed project: dist/<id>-<version>.vxp → work/<id>/manifest.json
    const id = path.basename(String(file)).replace(/-\d+\.\d+\.\d+\.vxp$/, "");
    const manifest = JSON.parse(await fs.readFile(path.join(path.dirname(path.dirname(String(file))), "work", id, "manifest.json"), "utf8"));
    host.candidates.set(String(file), { ...manifest, entry: "main.js" });
    return { token: String(file), digest: "0".repeat(64), apiVersion: "vestra/1", manifest: { ...manifest, entry: "main.js" }, checks: ["manifest", "slintCompile"], trustedExecution: true };
  });
}

test("a scaffolded plugin shows on the card with 安装 (no trial); the player's validation error reaches the card and the agent", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  assert.equal(ai.props["has-project"], false);
  await ai.ask("做个插件");
  assert.equal(await ai.calls(1), "✓ 创建插件 demo");
  await ai.until(() => ai.props["has-project"] === true, "card");
  assert.equal(ai.props["project-title"], "演示 · local.demo 0.1.0");
  assert.ok(!("trial-active" in ai.props), "no trial on the card");
  assert.ok(!mock.chats[0].body.tools.some((t) => /trial/.test(t.function.name)), "no trial tool");
  assert.doesNotMatch(mock.chats[0].body.messages[0].content, /试载|trial/);

  player(ai, { invalid: "ui/main.slint:3: unknown element FancyButton" });
  await ai.send("install");
  await ai.until(() => ai.props["card-status"].code === "invalid", "validation error on the card");
  assert.equal(ai.card(), "播放器校验没有通过（invalid）：ui/main.slint:3: unknown element FancyButton");
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.equal(await ai.calls(-2), "✗ 安装 local.demo");
  assert.match(toolText(mock, mock.chats.length - 1), /unknown element FancyButton/);
  assert.deepEqual(ai.host.packageCalls, []);
  await ai.stop();
});

test("install_plugin asks the user first; there is no undo, and a click on 安装 upgrades in place; a refusal is reported to the agent", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  player(ai);
  ai.host.dialogAnswers.push({ confirmed: false, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /The user cancelled the install/);
  assert.equal(ai.card(), "没有安装（已取消）。");
  assert.deepEqual(ai.host.packageCalls, []);

  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.deepEqual(ai.host.confirmTexts.at(-1), {
    message: "安装 AI 生成的插件「演示」？",
    detail: "local.demo 0.1.0\n能力：commands、ui.window",
    confirm: "安装",
    cancel: ai.host.confirmTexts.at(-1).cancel,
  });
  const told = toolText(mock, mock.chats.length - 1);
  assert.match(told, /Install 演示 0\.1\.0: done\. The plugin is enabled in the player\./);
  assert.doesNotMatch(told, /Roll Back|undo/i);
  assert.equal(ai.card(), "已安装 演示 0.1.0。");
  assert.deepEqual(ai.host.packageCalls.map((c) => [c.method, c.id, c.source?.kind]), [["install", "local.demo", "workshop"]]);
  assert.ok(!("can-rollback" in ai.props), "no Roll Back on the card");

  // A click on 安装 is the user's own decision: no second dialog. A new version upgrades the plugin in place.
  await ai.send("open-project");
  const manifestFile = path.join(ai.host.system.opened.at(-1), "manifest.json");
  await fs.writeFile(manifestFile, JSON.stringify({ ...JSON.parse(await fs.readFile(manifestFile, "utf8")), version: "0.1.1" }, null, 2));
  const confirms = ai.host.confirms.length;
  await ai.send("install");
  await ai.until(() => ai.props["card-status"].code === "installed" && ai.props["card-status"].args[1] === "0.1.1", "upgraded by click");
  assert.equal(ai.card(), "已安装 演示 0.1.1。");
  assert.equal(ai.host.confirms.length, confirms);
  assert.deepEqual(
    ai.host.packageCalls.map((c) => [c.method, c.id, c.source?.kind]),
    [
      ["install", "local.demo", "workshop"],
      ["install", "local.demo", "workshop"],
    ],
  );
  await ai.stop();
});

test("after an install, bindings another command holds are reported to the card and the agent", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  player(ai);
  // The player's table once local.demo runs: its `]` is held by a built-in plugin's command.
  const info = (plugin, command, title, current, shadowed = []) => ({ plugin, command, title, path: null, scope: "global", defaults: current, current, overridden: false, shadowed, labels: current });
  ai.host.handle("bindings.list", () => ({
    commands: [
      info("vestra.marks", "ab.set-b", "设置 B 点", ["]"]),
      info("local.demo", "faster", "加快 0.25×", ["]"], [{ input: "]", by: { plugin: "vestra.marks", command: "ab.set-b" } }]),
      info("local.demo", "slower", "减慢 0.25×", ["{"]),
    ],
    orphans: [],
  }));
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  const told = toolText(mock, mock.chats.length - 1);
  assert.match(told, /Install 演示 0\.1\.0: done/);
  assert.match(told, /Shortcuts not in effect: \] \(加快 0\.25×\) is taken by vestra\.marks "设置 B 点"\./);
  assert.equal(ai.card(), "已安装 演示 0.1.0。快捷键没有生效：");
  assert.deepEqual(
    ai.props["card-keys"].map((k) => shadowedText(ai.host, k)),
    ["]（加快 0.25×）已被vestra.marks「设置 B 点」占用"],
  );
  await ai.stop();
});

test("Export .vxp and Export Source .zip save where the user chose", async () => {
  const mock = await server(scripted({ 做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  await ai.until(() => ai.props["has-project"] === true, "card");
  const out = await dataDirectory();
  ai.host.dialogAnswers.push([path.join(out, "demo.vxp")]);
  await ai.send("export-vxp");
  await ai.until(() => ai.props["card-status"].code === "exported", "vxp exported");
  assert.equal(ai.card(), "已导出 demo.vxp");
  assert.equal(ai.host.confirmTexts.length, 0);
  assert.ok(unzip(await fs.readFile(path.join(out, "demo.vxp"))).has("manifest.json"));

  ai.host.dialogAnswers.push([path.join(out, "demo.zip")]);
  await ai.send("export-source");
  await ai.until(() => ai.props["card-status"].code === "exported-source-local", "source exported");
  assert.match(ai.card(), /^已导出 demo\.zip。id 以 local\. 开头/);
  assert.ok(unzip(await fs.readFile(path.join(out, "demo.zip"))).has("demo/.github/workflows/ci.yml"));


  await ai.send("open-project");
  assert.equal(path.basename(ai.host.system.opened.at(-1)), "local.demo");
  await ai.stop();
});

test("the GitHub name in the settings is the prefix of new plugins", async () => {
  const mock = await server(scripted({ 做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo" } }] }] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  assert.match(ai.ownerHint(), /local\.<名字>/);
  ai.page.emit("set-owner", "vestra");
  await ai.host.settle();
  assert.equal(ai.ownerHint(), "「vestra」留给官方插件，请填你自己的 GitHub 用户名（新插件暂时用 local.<名字>）");
  ai.page.emit("set-owner", "OctoCat");
  await ai.host.settle();
  assert.match(ai.ownerHint(), /^新插件的 id 是 octocat\.<名字>/);
  await ai.ask("做个插件");
  assert.match(mock.chats[0].body.messages[0].content, /currently "octocat"/);
  assert.match(mock.chats[0].body.messages[0].content, /The user's interface language is Simplified Chinese/);
  await ai.until(() => ai.props["project-title"] === "demo · octocat.demo 0.1.0", "owner prefix");
  await ai.stop();
});
