// The result card and the agent's install tool on the SDK test host: a
// scaffolded project shows on the card; Install (the agent's install asks
// first; there is no undo), Export .vxp, Export Source .zip. The player side of
// validation is simulated with host handlers.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
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
  assert.match(toolText(mock, mock.chats.length - 1), /run check again, before installing again/);
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
    detail: "local.demo 0.1.0",
    confirm: "安装",
    cancel: ai.host.confirmTexts.at(-1).cancel,
  });
  const told = toolText(mock, mock.chats.length - 1);
  assert.match(told, /Install 演示 0\.1\.0: done\. The plugin is enabled and running in the player\./);
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

test("the install dialog names the plugin, its id and version, and nothing about what its code uses", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  player(ai);
  await ai.send("open-project");
  const project = ai.host.system.opened.at(-1);
  // Code that uses the clipboard and another plugin's package: no capability
  // disclosure (user ruling 2026-11-16), so the dialog does not list it.
  await fs.appendFile(
    path.join(project, "main.ts"),
    '\nexport async function copy(plugin: Plugin) {\n  await plugin.clipboard.writeText("demo");\n  await plugin.packages.reload("local.other");\n}\n',
  );
  ai.host.dialogAnswers.push({ confirmed: false, checked: false });
  await ai.ask("安装");
  assert.deepEqual(ai.host.confirmTexts.at(-1), {
    message: "安装 AI 生成的插件「演示」？",
    detail: "local.demo 0.1.0",
    confirm: "安装",
    cancel: ai.host.confirmTexts.at(-1).cancel,
  });
  assert.match(toolText(mock, mock.chats.length - 1), /The user cancelled the install/);
  await ai.stop();
});

/**
 * The player after 0.6.3 (as in VS Code): an update or reinstall keeps a
 * disabled plugin disabled and does not start it; a first install runs.
 * `after` sets the listed state of other cases (e.g. a plugin that failed).
 */
function keepsDisabled(ai, after = (entry) => entry) {
  const { host } = ai;
  host.handle("packages.install", (p) => {
    const manifest = host.candidates.get(String(p.token));
    const before = host.packages.find((each) => each.id === manifest.id);
    host.packageCalls.push({ method: "install", id: manifest.id, source: p.source, replace: p.replace === true });
    const state = before?.state === "disabled" ? "disabled" : "running";
    const next = after({ id: manifest.id, version: manifest.version, origin: "installed", state, error: null, directory: `/test-host/packages/${manifest.id}`, builtin: false, source: p.source, dataDirectory: null });
    host.setPackages([...host.packages.filter((each) => each.id !== manifest.id), next]);
    return { operationId: String(p.requestId), status: "committed" };
  });
}

test("an update of a plugin the user disabled stays disabled: the agent and the card say so, and the workshop does not enable it", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  player(ai);
  keepsDisabled(ai);
  // The first install enables and runs it.
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /Install 演示 0\.1\.0: done\. The plugin is enabled and running in the player\./);
  assert.equal(ai.card(), "已安装 演示 0.1.0。");

  // The user disables it in the Plugin Store; the agent installs an iteration.
  ai.host.setPackages(ai.host.packages.map((p) => (p.id === "local.demo" ? { ...p, state: "disabled" } : p)));
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  const told = toolText(mock, mock.chats.length - 1);
  assert.match(told, /Install 演示 0\.1\.0: done\. The plugin is installed but disabled: the user turned it off in the player/);
  assert.match(told, /Only the user can enable it, in the Plugin Store/);
  assert.match(told, /do not try to enable it yourself/);
  assert.doesNotMatch(told, /enabled and running/);
  assert.equal(ai.props["card-status"].code, "installed-disabled");
  assert.equal(ai.card(), "已安装 演示 0.1.0。它保持你设置的停用状态；要启用，请在插件商城中操作。");
  assert.equal(ai.host.packages.find((p) => p.id === "local.demo").state, "disabled");
  assert.deepEqual(
    ai.host.packageCalls.map((c) => c.method),
    ["install", "install"],
    "no setEnabled",
  );

  // A plugin that failed once it ran: installed (recorded), the error reaches the agent and the card.
  keepsDisabled(ai, (entry) => ({ ...entry, state: "failed", error: "activate threw: boom" }));
  ai.host.setPackages(ai.host.packages.filter((p) => p.id !== "local.demo"));
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /Install 演示 0\.1\.0: done, but the plugin then failed in the player: activate threw: boom\. Fix the cause/);
  assert.equal(ai.card(), "已安装 演示 0.1.0，但它出错了：activate threw: boom");
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

test("the card shows once the turn has ended, not while the agent still works; a later turn hides it until it ends", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }, { text: ["做好了。"], delayMs: 1500 }],
      改一下: [{ toolCalls: [{ name: "write", args: { path: "local.demo/README.md", content: "# 演示\n" } }] }, { text: ["改好了。"], delayMs: 1500 }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件", { wait: false });
  // The project exists (the scaffold's result went back to the model); the turn goes on.
  await ai.until(() => mock.chats.length === 2, "scaffold done");
  await ai.until(() => ai.props["project-title"] === "演示 · local.demo 0.1.0", "project known");
  assert.equal(ai.props.busy, true);
  assert.equal(ai.props["has-project"], false, "no card while the agent works");
  await ai.until(() => ai.props.busy === false && ai.props.rows.at(-1).state !== 0, "reply");
  await ai.until(() => ai.props["has-project"] === true, "card after the turn");

  await ai.ask("改一下", { wait: false });
  await ai.until(() => ai.props.busy === true, "busy");
  assert.equal(ai.props["has-project"], false, "hidden again while the agent edits the plugin");
  await ai.until(() => mock.chats.length === 4, "edit done");
  assert.equal(ai.props["has-project"], false);
  await ai.until(() => ai.props.busy === false && ai.props.rows.at(-1).state !== 0, "second reply");
  await ai.until(() => ai.props["has-project"] === true, "card after the second turn");
  await ai.stop();
});

test("the player's reserved refusal (a vestra.* id), at validation or at install, reads as a built-in plugin's name", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  // The id rules refuse vestra.* before packing; the player's answer is simulated for local.demo.
  const reserved = () => {
    throw new HostError("reserved", "vestra.* ids belong to the player's built-in plugins");
  };
  ai.host.handle("packages.validateArchive", reserved);
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /local\.demo has the name of a plugin built into the player and cannot be installed; choose another id\./);
  assert.equal(ai.props["card-status"].code, "builtin-id");

  player(ai);
  ai.host.handle("packages.install", reserved);
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /local\.demo has the name of a plugin built into the player and cannot be installed; choose another id\./);
  assert.equal(ai.card(), "local.demo 和播放器内置插件同名，不能安装，请换一个 id。");
  await ai.stop();
});
