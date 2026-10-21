// Workshop control flow on the SDK test host: real Pi generation and TypeScript,
// scripted package answers. This is not managed-host or GUI acceptance.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { startWorkshop } from "./fixture.mjs";

test("one menu entrance; reopening shows the same window", async () => {
  const w = await startWorkshop();
  assert.deepEqual(w.host.command("workshop.open").declaration, {
    title: "插件工坊…",
    path: ["插件工坊…"],
    order: 820,
    group: "windows",
    radio: false,
    bindings: [],
    focus: null,
    repeat: false,
  });
  await w.host.command("workshop.open").run();
  assert.equal(w.host.sent("ui.open").length, 1);
  assert.equal(w.window.file, "workshop.slint");
  assert.equal(w.status(), "选择工作区并输入需求后生成。");
  assert.equal(w.props["workspace-text"], "尚未选择：请选择专用空文件夹");
  assert.equal(w.props["can-trial"], false);
  assert.equal(w.props["can-recover"], false);
  await w.send("generate");
  assert.match(w.status(), /^错误：.*请先选择本次工坊专用工作区/);
  await w.stop();
});

test("generate → validate → trial → stop → install → rollback", async () => {
  const w = await startWorkshop();
  await w.generate();
  const [validated] = w.host.sent("packages.validate");
  assert.equal(w.status(), "候选已通过宿主校验：community.counter 1.0.0，可试载或安装。");
  assert.equal(path.dirname(String(validated.directory)), await fs.realpath(w.workspace));
  await fs.access(path.join(String(validated.directory), "main.js"));
  assert.match(String(w.props["log-text"]), /真实TypeScript诊断/);
  assert.equal(w.props["can-trial"], true);
  assert.equal(w.props["can-install"], true);
  assert.doesNotMatch(String(w.props["log-text"]), /"revision"|digest/, "no revision or digest in the log");

  await w.send("start-trial");
  assert.equal(w.status(), "试载中：请在计数器窗口测试「增加」「归零」。");
  assert.deepEqual([w.props["can-stop"], w.props["can-trial"], w.props["can-install"]], [true, false, false]);
  await w.send("install-candidate");
  assert.match(w.status(), /请先等待任务完成并停止试载/);
  assert.equal(w.host.sent("packages.install").length, 0);
  await w.send("stop-trial");
  assert.equal(w.status(), "试载已停止并清理。");
  assert.deepEqual(w.host.sent("packages.stopTrial"), [{ trialId: 1 }]);

  await w.send("install-candidate");
  await w.until(() => w.props.busy === false && w.status() === "安装完成。", "install");
  const [install] = w.host.sent("packages.install");
  assert.equal(install.token, "validated");
  assert.equal(install.expectedRevision, 7);
  assert.deepEqual(w.host.sent("packages.installed"), [{ id: "community.counter" }]);

  await w.send("rollback-version");
  await w.until(() => w.props.busy === false && w.host.sent("packages.rollback").length === 1, "rollback");
  assert.equal(w.host.sent("packages.rollback")[0].id, "community.counter");
  assert.equal(w.status(), "回退完成。");
  await w.stop();
});

for (const [name, failure] of [
  ["a refused read", () => new HostError("unavailable", "read failure")],
  ["a lost read reply", () => new Error("read failure")],
]) {
  test(`${name} of the installed revision releases busy and submits nothing`, async () => {
    const w = await startWorkshop({
      installed: () => {
        throw failure();
      },
    });
    await w.send("rollback-version");
    assert.equal(w.props.busy, false);
    assert.match(w.status(), /^错误：.*read failure/);
    assert.equal(w.host.sent("packages.rollback").length, 0);
    await w.chooseWorkspace();
    assert.equal(w.host.sent("dialogs.open").length, 1);
    assert.equal(w.props["workspace-text"], await fs.realpath(w.workspace));
    await w.stop();
  });
}

for (const action of ["install-candidate", "rollback-version"]) {
  test(`${action}: a lost accepted reply is followed by the same request id, never resubmitted`, async () => {
    const submitted = [];
    const lookups = [];
    const lose = ({ requestId }) => {
      submitted.push(requestId);
      throw new Error("lost accepted response");
    };
    const w = await startWorkshop({
      install: lose,
      rollback: lose,
      operation: ({ operationId }) => {
        lookups.push(operationId);
        return { operationId, status: "committed" };
      },
    });
    await w.generate();
    await w.send(action);
    await w.until(() => w.props.busy === false && /^(安装|回退)完成。$/.test(w.status()), action);
    assert.equal(submitted.length, 1);
    assert.ok(lookups.length >= 1 && lookups.every((id) => id === submitted[0]));
    await w.stop();
  });
}

test("cancel during an install asks the host to cancel the same operation", async () => {
  let status = "preparing";
  const w = await startWorkshop({
    install: ({ requestId }) => ({ operationId: requestId, status }),
    operation: ({ operationId }) => ({ operationId, status }),
    cancel: ({ operationId }) => ({ operationId, status: (status = "cancelled") }),
  });
  await w.generate();
  await w.send("install-candidate");
  assert.equal(w.status(), "安装：正在准备…");
  assert.equal(w.props.busy, true);
  assert.equal(w.props["can-trial"], false);
  await w.send("cancel-task");
  await w.until(() => w.props.busy === false, "cancel");
  assert.equal(w.status(), "安装已取消。");
  assert.deepEqual(w.host.sent("packages.cancel"), [{ operationId: w.host.sent("packages.install")[0].requestId }]);
  await w.stop();
});

test("cancelling a generation stops Pi and validates nothing", async () => {
  const w = await startWorkshop();
  await w.chooseWorkspace();
  w.window.emit("generate");
  w.window.emit("cancel-task");
  await w.until(() => w.props.busy === false && /取消/.test(w.status()), "cancellation");
  await w.host.settle(50);
  assert.equal(w.status(), "生成已取消。");
  assert.equal(w.host.sent("packages.validate").length, 0);
  assert.equal(w.props["can-trial"], false);
  await w.stop();
});

test("a requirement file replaces the draft; invalid or edited drafts are checked before generating", async () => {
  const w = await startWorkshop();
  const file = path.join(w.workspace, "requirement.json");
  const text = JSON.stringify({ kind: "native-counter", id: "community.counter", version: "1.1.0", step: 3 });
  await fs.writeFile(file, text);
  w.host.dialogAnswers.push([file]);
  await w.send("choose-requirement");
  assert.equal(w.props["requirement-text"], text);
  assert.equal(w.status(), "需求已读取，点击生成。");

  await fs.writeFile(file, '{"kind":"native-counter","id":"community.counter","version":"1.1.0","step":1}');
  w.host.dialogAnswers.push([file]);
  await w.send("choose-requirement");
  assert.match(w.status(), /^错误：.*仅支持native-counter/);
  assert.equal(w.props["requirement-text"], text);

  w.host.dialogAnswers.push(null);
  await w.send("choose-requirement");
  assert.equal(w.props["requirement-text"], text);

  await w.chooseWorkspace();
  await w.send("draft-changed", "{not json");
  await w.send("generate");
  assert.match(w.status(), /^错误：/);
  assert.equal(w.props.busy, false);
  assert.equal(w.host.sent("packages.validate").length, 0);
  await w.stop();
});
