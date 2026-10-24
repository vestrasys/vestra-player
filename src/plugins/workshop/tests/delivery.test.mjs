// The result card and the agent's trial/install tools on the SDK test host:
// a scaffolded project shows on the card; 试载 / 停止试载, 安装 (the agent's
// install asks first), 回退, 导出 .vxp, 导出源码 zip. The player side of
// validation and trials is simulated with host handlers.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { dataDirectory, server, startAi } from "./fixture.mjs";
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

/** The player's side: validation by file, a trial that starts (or fails), install, uninstall. */
function player(ai, { trialState = "running", trialError = null } = {}) {
  const { host } = ai;
  const projects = path.join(ai.plugin.dataDirectory, "sessions");
  host.handle("packages.validateArchive", async ({ file }) => {
    // The manifest of the packed project: dist/<id>-<version>.vxp → work/<id>/manifest.json
    const id = path.basename(String(file)).replace(/-\d+\.\d+\.\d+\.vxp$/, "");
    const manifest = JSON.parse(await fs.readFile(path.join(path.dirname(path.dirname(String(file))), "work", id, "manifest.json"), "utf8"));
    host.candidates.set(String(file), { ...manifest, entry: "main.js" });
    return { token: String(file), digest: "0".repeat(64), apiVersion: "vestra/1", manifest: { ...manifest, entry: "main.js" }, checks: ["manifest", "slintCompile"], trustedExecution: true };
  });
  const trialEntry = (id) => ({ id, version: "0.1.0", origin: "trial", state: trialState, error: trialError, capabilities: [], directory: "/trial", previousVersion: null, builtin: false, source: null, dataDirectory: null });
  host.handle("packages.trial", ({ token }) => {
    const id = path.basename(String(token)).replace(/-\d+\.\d+\.\d+\.vxp$/, "");
    host.setPackages([...host.packages.filter((p) => p.origin !== "trial"), trialEntry(id)]);
    return { trialId: 9, activationOwner: 9, status: "pending" };
  });
  const stopped = () => {
    host.setPackages(host.packages.filter((p) => p.origin !== "trial"));
    return { trialId: 9, activationOwner: 9, status: "stopped", cleanupConfirmed: true };
  };
  host.handle("packages.stopTrial", stopped);
  host.handle("packages.trialState", stopped);
  return projects;
}

test("a scaffolded plugin shows on the card; 试载 starts and 停止试载 ends the trial", async () => {
  const mock = await server(scripted({ 做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }] }));
  const ai = await startAi({ compat: mock.url });
  assert.equal(ai.props["has-project"], false);
  await ai.ask("做个插件");
  assert.equal(ai.rows()[1].detail, "✓ 创建插件 demo");
  await ai.until(() => ai.props["has-project"] === true, "card");
  assert.equal(ai.props["project-title"], "演示 · local.demo 0.1.0");
  assert.equal(ai.props["trial-active"], false);

  player(ai);
  await ai.send("trial");
  await ai.until(() => ai.props["trial-active"] === true, "trial");
  assert.equal(ai.props["card-status"], "试载中：演示");
  await ai.send("stop-trial");
  await ai.until(() => ai.props["trial-active"] === false, "trial stopped");
  assert.equal(ai.props["card-status"], "已停止试载 演示。");
  await ai.stop();
});

test("a trial that fails to activate reports the player's error; capabilities a trial cannot load are explained", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo" } }] }],
      试载: [{ toolCalls: [{ name: "trial_load", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url });
  await ai.ask("做个插件");
  player(ai, { trialState: "failed", trialError: "TypeError: plugin.osd is undefined" });
  await ai.ask("试载");
  assert.equal(ai.rows().at(-2).detail, "✗ 试载 local.demo");
  assert.match(toolText(mock, mock.chats.length - 1), /试载的插件没能启动：TypeError: plugin\.osd is undefined/);
  assert.equal(ai.props["trial-active"], false);

  const manifestFile = (await fs.readdir(path.join(ai.plugin.dataDirectory, "sessions"))).map((s) => path.join(ai.plugin.dataDirectory, "sessions", s, "work/local.demo/manifest.json"))[0];
  const manifest = JSON.parse(await fs.readFile(manifestFile, "utf8"));
  await fs.writeFile(manifestFile, JSON.stringify({ ...manifest, capabilities: [...manifest.capabilities, "ui.osd"] }));
  await ai.send("trial");
  await ai.until(() => /不能试载/.test(ai.props["card-status"]), "refused");
  assert.match(ai.props["card-status"], /还用到 ui\.osd。可以直接安装/);
  await ai.stop();
});

test("install_plugin asks the user first; 回退 removes a first install; a refusal is reported to the agent", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      安装: [{ toolCalls: [{ name: "install_plugin", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url });
  await ai.ask("做个插件");
  player(ai);
  ai.host.dialogAnswers.push({ confirmed: false, checked: false });
  await ai.ask("安装");
  assert.match(toolText(mock, mock.chats.length - 1), /用户取消了安装/);
  assert.deepEqual(ai.host.packageCalls, []);

  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.ask("安装");
  assert.match(ai.host.confirms.at(-1).message, /安装 AI 生成的插件「演示」/);
  assert.match(toolText(mock, mock.chats.length - 1), /安装 演示 0\.1\.0完成/);
  assert.deepEqual(ai.host.packageCalls.map((c) => [c.method, c.id, c.source?.kind]), [["install", "local.demo", "workshop"]]);
  await ai.until(() => ai.props["can-rollback"] === true, "rollback offered");

  await ai.send("rollback");
  await ai.until(() => ai.props["can-rollback"] === false, "rolled back");
  assert.deepEqual(ai.host.packageCalls.at(-1), { method: "uninstall", id: "local.demo", removeData: false });
  // A click on 安装 is the user's own decision: no second dialog.
  const confirms = ai.host.confirms.length;
  await ai.send("install");
  await ai.until(() => ai.props["can-rollback"] === true, "installed by click");
  assert.equal(ai.host.confirms.length, confirms);
  await ai.stop();
});

test("导出 .vxp and 导出源码 zip save where the user chose", async () => {
  const mock = await server(scripted({ 做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }] }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("做个插件");
  await ai.until(() => ai.props["has-project"] === true, "card");
  const out = await dataDirectory();
  ai.host.dialogAnswers.push([path.join(out, "demo.vxp")]);
  await ai.send("export-vxp");
  await ai.until(() => /已导出/.test(ai.props["card-status"]), "vxp exported");
  assert.ok(unzip(await fs.readFile(path.join(out, "demo.vxp"))).has("manifest.json"));

  ai.host.dialogAnswers.push([path.join(out, "demo.zip")]);
  await ai.send("export-source");
  await ai.until(() => /已导出 demo\.zip/.test(ai.props["card-status"]), "source exported");
  assert.ok(unzip(await fs.readFile(path.join(out, "demo.zip"))).has("demo/.github/workflows/ci.yml"));
  assert.match(ai.props["card-status"], /local\./);

  await ai.send("open-project");
  assert.equal(path.basename(ai.host.system.opened.at(-1)), "local.demo");
  await ai.stop();
});

test("the GitHub name in the settings is the prefix of new plugins", async () => {
  const mock = await server(scripted({ 做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo" } }] }] }));
  const ai = await startAi({ compat: mock.url });
  assert.match(ai.page.props["owner-hint"], /local\.<名字>/);
  ai.page.emit("set-owner", "vestra");
  await ai.host.settle();
  assert.match(ai.page.props["owner-hint"], /留给官方插件/);
  ai.page.emit("set-owner", "OctoCat");
  await ai.host.settle();
  assert.match(ai.page.props["owner-hint"], /octocat\.<名字>/);
  await ai.ask("做个插件");
  assert.match(mock.chats[0].body.messages[0].content, /现在是「octocat」/);
  await ai.until(() => ai.props["project-title"] === "demo · octocat.demo 0.1.0", "owner prefix");
  await ai.stop();
});
