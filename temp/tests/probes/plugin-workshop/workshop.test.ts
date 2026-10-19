// 实验验收：真实 Pi Agent + Node worker + Rust 校验/安装；只 mock 外部模型。
import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../../support/host.ts";
import { parseRequest } from "../../../plugins/plugin-workshop/engine.ts";
import type { RunEvidence } from "../../../plugins/plugin-workshop/engine.ts";

const manifest = fileURLToPath(
  new URL("../../../plugins/plugin-workshop/manifest.json", import.meta.url),
);
const id = "plugin-workshop";
const generated = "workshop-generated-counter";
const evidenceDirectory = resolve(
  process.env.VESTRA_WORKSHOP_EVIDENCE ??
    ".cache/probe-evidence/plugin-workshop",
);
async function properties(host: TestHost, plugin = id) {
  return (
    (await host.inspect()).plugins.find((p) => p.id === plugin)?.properties ??
    {}
  );
}
async function run(host: TestHost, action: string, value = 0) {
  const previous = (await properties(host)).report;
  await host.action(id, action, value);
  await host.until(async () => {
    const p = await properties(host);
    return (
      typeof p.report === "string" &&
      p.report.length > 0 &&
      p.report !== previous &&
      p.busy === false
    );
  }, 60000);
  const p = await properties(host);
  assert.equal(typeof p.report, "string");
  const path = String(p.report);
  const report: RunEvidence = JSON.parse(await readFile(path, "utf8"));
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(
    join(evidenceDirectory, `${action}-${value}.json`),
    JSON.stringify(report, null, 2),
  );
  return { report, properties: p, path };
}

test("需求 JSON 严格校验，拒绝无法表达的 mock 需求配置", () => {
  assert.deepEqual(parseRequest({ prompt: "生成加三计数器", increment: 3 }), {
    prompt: "生成加三计数器",
    increment: 3,
    scenario: "repair",
  });
  for (const input of [
    null,
    [],
    {},
    { prompt: "", increment: 1 },
    { prompt: "counter", increment: NaN },
    { prompt: "counter", increment: 1.5 },
    { prompt: "counter", increment: 11 },
    { prompt: "counter", increment: 1, secret: "x" },
  ]) {
    assert.throws(() => parseRequest(input));
  }
});

test(
  "W1–W3：真实 Pi 诊断修复/类型与Slint校验/独立执行/取消/事务失败/回退",
  { timeout: 180000 },
  async (t) => {
    const host = new TestHost();
    t.after(() => host.close());
    await host.load(manifest);
    await host.until(
      async () => (await properties(host)).phase === "idle",
      15000,
    );
    assert.deepEqual(
      (await host.inspect()).plugins.map((p) => p.id),
      [id],
      "工坊不依赖其他五个探针",
    );

    const first = await run(host, "generate", 1);
    assert.equal(first.report.outcome, "installed");
    assert.equal(first.report.pi, "@earendil-works/pi-agent-core@1.3.0");
    assert.deepEqual(
      first.report.validations.map((v) => v.valid),
      [false, true],
    );
    assert.match(first.report.validations[0].diagnostics.join("\n"), /TS2322/);
    assert.deepEqual(
      first.report.events
        .filter((e) => e.type === "tool_execution_start")
        .map((e) => e.tool),
      [
        "write_package",
        "validate_package",
        "write_package",
        "validate_package",
        "install_package",
      ],
    );
    assert.equal(
      first.report.events.filter((e) => e.type === "turn_start").length,
      6,
      "Pi工具循环真实请求六轮mock模型",
    );
    const transcript = JSON.stringify(first.report.transcript);
    assert.match(transcript, /TS2322/);
    assert.match(transcript, /not-a-number/);
    assert.doesNotMatch(
      await readFile(join(dirname(first.path), "index.ts"), "utf8"),
      /not-a-number/,
    );
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 1,
    );

    const old = (await host.inspect()).plugins.find((p) => p.id === generated);
    assert.ok(old);
    const invalid = await run(host, "invalid");
    assert.equal(invalid.report.outcome, "failed");
    assert.deepEqual(
      invalid.report.validations.map((v) => v.valid),
      [false],
    );
    assert.equal(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      old.route,
    );
    assert.equal((await properties(host, generated)).count, 1);

    const slint = await run(host, "slint-failure");
    assert.equal(slint.report.outcome, "failed");
    assert.match(
      slint.report.validations[0].diagnostics.join("\n"),
      /slint|Syntax|Parse|expected/i,
    );
    assert.equal(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      old.route,
    );

    const activation = await run(host, "activation-failure");
    assert.equal(activation.report.outcome, "failed");
    assert.equal(
      activation.report.validations[0].valid,
      true,
      "坏候选类型/Slint合法但真实activate失败",
    );
    assert.match(
      JSON.stringify(activation.report.transcript),
      /intentional candidate activation failure/,
    );
    assert.equal(
      (await host.inspect()).plugins.filter((p) => p.id === generated).length,
      1,
    );
    assert.equal(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      old.route,
    );
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 2,
    );

    const previousReport = (await properties(host)).report;
    await host.action(id, "slow");
    await host.until(async () => (await properties(host)).busy === true);
    await host.action(id, "cancel");
    await host.until(async () => {
      const p = await properties(host);
      return p.busy === false && p.report !== previousReport;
    }, 15000);
    const cancelPath = String((await properties(host)).report);
    const cancellation: RunEvidence = JSON.parse(
      await readFile(cancelPath, "utf8"),
    );
    assert.equal(cancellation.outcome, "cancelled");
    assert.equal(cancellation.validations.length, 0);
    assert.equal(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      old.route,
    );
    await writeFile(
      join(evidenceDirectory, "cancel.json"),
      JSON.stringify(cancellation, null, 2),
    );

    const second = await run(host, "generate", 2);
    assert.equal(second.report.outcome, "installed");
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 2,
    );
    assert.equal((await properties(host, generated)).increment, 2);
    assert.notEqual(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      old.route,
    );
    await host.action(id, "rollback");
    await host.until(
      async () => (await properties(host)).phase === "rolled-back",
      30000,
    );
    assert.equal((await properties(host, generated)).increment, 1);
    assert.equal(
      (await properties(host, generated)).count,
      0,
      "显式回退重启旧包，区别于激活失败保留原worker",
    );
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 1,
    );

    const beforeUnload = (await host.inspect()).plugins.find(
      (p) => p.id === generated,
    )?.route;
    await host.unload(id);
    await host.until(
      async () => !(await host.inspect()).plugins.some((p) => p.id === id),
    );
    assert.equal(
      (await host.inspect()).plugins.find((p) => p.id === generated)?.route,
      beforeUnload,
      "安装产物是独立插件，不随工坊窗口卸载",
    );
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 2,
    );
    await writeFile(
      join(evidenceDirectory, "summary.json"),
      JSON.stringify(
        {
          baseline: "probe-0.2",
          generatedBehavior: {
            firstIncrement: 1,
            replacementIncrement: 2,
            rollbackIncrement: 1,
            afterWorkshopUnload: 2,
          },
          preservedRouteOnFailure: old.route,
          evidence: [
            first.path,
            invalid.path,
            slint.path,
            activation.path,
            cancelPath,
            second.path,
          ],
          snapshot: await host.inspect(),
        },
        null,
        2,
      ),
    );
  },
);

test(
  "已安装生成包可通过工坊公共remove操作释放",
  { timeout: 90000 },
  async (t) => {
    const host = new TestHost();
    t.after(() => host.close());
    await host.load(manifest);
    await host.until(
      async () => (await properties(host)).phase === "idle",
      15000,
    );
    await mkdir(evidenceDirectory, { recursive: true });
    const requestFile = join(evidenceDirectory, "request.json");
    await writeFile(
      requestFile,
      JSON.stringify({ prompt: "给我一个加三的原生计数器", increment: 3 }),
    );
    await host.request("host.dialogs", { paths: [requestFile] });
    await host.action(id, "request-file");
    await host.until(
      async () =>
        (await properties(host)).request === "给我一个加三的原生计数器",
    );
    const created = await run(host, "generate");
    assert.equal(created.report.outcome, "installed");
    assert.equal(created.report.request.increment, 3);
    await host.action(generated, "increment");
    await host.until(
      async () => (await properties(host, generated)).count === 3,
    );
    await host.action(id, "remove-generated");
    await host.until(async () => (await properties(host)).phase === "removed");
    assert.deepEqual(
      (await host.inspect()).plugins.map((p) => p.id),
      [id],
    );
    await host.action(id, "rollback");
    await host.until(async () =>
      String((await properties(host)).status).includes("没有可回退版本"),
    );
    assert.equal((await properties(host)).busy, false);
  },
);
