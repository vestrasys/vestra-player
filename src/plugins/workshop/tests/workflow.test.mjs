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
  assert.equal(w.status(), "候选已通过宿主校验：workshop.counter 1.0.0，可试载或安装。");
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
  assert.deepEqual(w.host.sent("packages.installed"), [{ id: "workshop.counter" }]);

  await w.send("rollback-version");
  await w.until(() => w.props.busy === false && w.host.sent("packages.rollback").length === 1, "rollback");
  assert.equal(w.host.sent("packages.rollback")[0].id, "workshop.counter");
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
  const text = JSON.stringify({ kind: "native-counter", id: "workshop.counter", version: "1.1.0", step: 3 });
  await fs.writeFile(file, text);
  w.host.dialogAnswers.push([file]);
  await w.send("choose-requirement");
  assert.equal(w.props["requirement-text"], text);
  assert.equal(w.status(), "需求已读取，点击生成。");

  await fs.writeFile(file, '{"kind":"native-counter","id":"workshop.counter","version":"1.1.0","step":1}');
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

/** An install that fails with `conflict` unless `replace` is set (another source holds the id). */
const conflictUnlessReplace = (submitted) => (params) => {
  submitted.push(params);
  if (!params.replace) throw new HostError("conflict", "workshop.counter is installed from another source");
  return { operationId: params.requestId, status: "committed" };
};

test("install passes the workshop source and no replace", async () => {
  const w = await startWorkshop();
  await w.generate();
  await w.send("install-candidate");
  await w.until(() => w.props.busy === false && w.status() === "安装完成。", "install");
  const [install] = w.host.sent("packages.install");
  assert.deepEqual(install.source, { kind: "workshop" });
  assert.equal(install.replace, undefined);
  assert.equal(w.host.confirms.length, 0, "no question without a conflict");
  await w.stop();
});

test("conflict: the user confirms and the install is retried with replace", async () => {
  const submitted = [];
  const w = await startWorkshop({ install: conflictUnlessReplace(submitted) });
  await w.generate();
  w.host.dialogAnswers.push({ confirmed: true, checked: false });
  await w.send("install-candidate");
  await w.until(() => w.props.busy === false && w.status() === "安装完成。", "replacing install");
  assert.equal(w.host.confirms.length, 1);
  const [question] = w.host.confirms;
  assert.equal(question.message, "替换已安装的 workshop.counter？");
  assert.match(String(question.detail), /从别处安装.*新的空数据目录.*原插件的数据保留/s);
  assert.equal(question.confirm, "替换");
  assert.equal(question.destructive, true);
  assert.deepEqual(
    submitted.map((p) => [p.source, p.replace]),
    [
      [{ kind: "workshop" }, undefined],
      [{ kind: "workshop" }, true],
    ],
  );
  assert.equal(submitted[0].token, submitted[1].token, "the same validated candidate");
  assert.notEqual(submitted[0].requestId, submitted[1].requestId, "a new request, not a resubmission");
  assert.match(String(w.props["log-text"]), /用户确认替换 workshop\.counter/);
  await w.stop();
});

test("conflict: cancelling keeps the installed plugin and the candidate", async () => {
  const submitted = [];
  const w = await startWorkshop({ install: conflictUnlessReplace(submitted) });
  await w.generate();
  w.host.dialogAnswers.push({ confirmed: false, checked: false });
  await w.send("install-candidate");
  await w.until(() => w.props.busy === false, "cancelled replace");
  assert.equal(w.host.confirms.length, 1);
  assert.equal(submitted.length, 1, "nothing is resubmitted");
  assert.equal(w.status(), "未安装：已保留原插件和本次候选，可稍后再安装。");
  assert.match(String(w.props["log-text"]), /用户取消替换 workshop\.counter，候选保留/);
  assert.deepEqual([w.props["can-install"], w.props["can-trial"]], [true, true]);

  // The kept candidate installs later, after a fresh question.
  w.host.dialogAnswers.push({ confirmed: true, checked: false });
  await w.send("install-candidate");
  await w.until(() => w.props.busy === false && w.status() === "安装完成。", "second attempt");
  assert.equal(w.host.confirms.length, 2);
  assert.deepEqual(
    submitted.map((p) => p.replace),
    [undefined, undefined, true],
  );
  await w.stop();
});

test("unsupported: an id of a built-in plugin asks for another id and asks nothing", async () => {
  const submitted = [];
  const w = await startWorkshop({
    install: (params) => {
      submitted.push(params);
      throw new HostError("unsupported", "built-in plugins are updated with the player");
    },
  });
  await w.generate();
  await w.send("install-candidate");
  assert.equal(w.props.busy, false);
  assert.equal(w.status(), "错误：workshop.counter 与播放器内置插件同名，不能安装。请在需求中改用别的 id 后重新生成。");
  assert.equal(w.host.confirms.length, 0);
  assert.equal(submitted.length, 1);
  assert.equal(w.props["can-install"], true);
  await w.stop();
});

test("other install failures are reported unchanged", async () => {
  const w = await startWorkshop({
    install: () => {
      throw new HostError("stale", "validation token expired");
    },
  });
  await w.generate();
  await w.send("install-candidate");
  assert.equal(w.props.busy, false);
  assert.match(w.status(), /^错误：.*validation token expired/);
  assert.equal(w.host.confirms.length, 0);
  await w.stop();
});
