/** Independent frozen-host JSONL command consumer; headless, no menu/draw claims. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const work = await mkdtemp(resolve(tmpdir(), "vestra-commands-review-"));

async function wire(onEvent) {
  const { createInterface } = await import("node:readline");
  const lines = createInterface({ input: process.stdin });
  const pending = new Map();
  let id = 1000000;
  lines.on("line", (line) => {
    const m = JSON.parse(line);
    if (m.event) onEvent(m.event, m.data);
    const p = pending.get(m.id);
    if (!p) return;
    clearTimeout(p.timer);
    pending.delete(m.id);
    if (m.error) p.reject(Object.assign(new Error(m.error.message), m.error));
    else p.resolve(m.result);
  });
  return (method, params = {}) =>
    new Promise((resolve, reject) => {
      const key = ++id;
      const timer = setTimeout(() => {
        pending.delete(key);
        reject(Error(`${method}: outcome unknown`));
      }, 5000);
      pending.set(key, { resolve, reject, timer });
      process.stdout.write(JSON.stringify({ id: key, method, params }) + "\n");
    });
}
function flatten(nodes) {
  return nodes.flatMap((n) =>
    n.kind === "command" ? [n.entry] : flatten(n.children),
  );
}

async function consumer(host) {
  const assert = (await import("node:assert/strict")).default;
  const fs = await import("node:fs/promises"),
    path = await import("node:path");
  const work = process.env.VESTRA_COMMANDS_REVIEW,
    callbacks = [],
    checks = {};
  const request = await wire((event, data) => {
    if (event === "commands.execute") callbacks.push(data);
  });
  const snapshot = () => request("commands.snapshot"),
    wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const normal = { enabled: true, checked: null, busy: false, reason: null };
  const register = (id, menu, state = normal) =>
    request("commands.register", {
      id,
      revision: 1,
      declaration: { title: menu.at(-1), path: menu, shortcut: null },
      state,
    });
  async function until(predicate, label) {
    const deadline = Date.now() + 4000;
    while (Date.now() < deadline) {
      const s = await snapshot();
      if (predicate(s)) return s;
      await wait(20);
    }
    throw Error(label + ": " + JSON.stringify(await snapshot()));
  }
  try {
    assert.ok((await host.capabilities()).capabilities.includes("commands"));
    const domains = [
      "文件",
      "播放",
      "视频",
      "音频",
      "字幕",
      "视图",
      "模拟器",
      "插件",
    ];
    for (const [i, domain] of [...domains].reverse().entries())
      await register(`domain-${i}`, [domain, "独立命令审查"]);
    await register("run", ["播放", "测试分组", "Z"]);
    await register("a", ["播放", "测试分组", "A"]);
    const initial = await snapshot();
    assert.deepEqual(
      initial.roots.map((n) => n.title),
      domains,
    );
    const group = initial.roots[1].children.find(
      (n) => n.kind === "group" && n.title === "测试分组",
    );
    assert.deepEqual(
      group.children.map((n) => n.entry.declaration.title),
      ["A", "Z"],
    );
    const own = flatten(initial.roots).find((e) => e.key.localId === "run");
    await request("commands.execute", { id: "run" });
    await wait(30);
    assert.deepEqual(callbacks, [{ id: "run", revision: 1 }]);
    checks.orderAndOwnExecution = { initial, callback: callbacks[0] };

    // Capture the snapshot BEFORE sending the request under test.
    let before = await snapshot();
    await assert.rejects(
      register("duplicate-path", ["播放", "测试分组", "Z"]),
      (e) => e.code === "command" && /conflict/.test(e.message),
    );
    assert.deepEqual(await snapshot(), before);
    await assert.rejects(
      register("group-collision", ["播放", "测试分组"]),
      (e) => e.code === "command" && /conflict/.test(e.message),
    );
    assert.deepEqual(await snapshot(), before);
    await request("commands.update", {
      id: "run",
      revision: 3,
      state: { ...normal, enabled: false, checked: true, reason: "No source" },
    });
    before = await snapshot();
    const callbackCount = callbacks.length;
    await assert.rejects(
      request("commands.update", { id: "run", revision: 2, state: normal }),
      (e) => e.code === "command" && /stale/.test(e.message),
    );
    await assert.rejects(
      request("commands.execute", { id: "run" }),
      (e) => e.code === "command" && /disabled.*No source/.test(e.message),
    );
    assert.deepEqual(await snapshot(), before);
    await request("commands.update", {
      id: "run",
      revision: 4,
      state: { ...normal, busy: true, reason: "Preparing" },
    });
    before = await snapshot();
    await assert.rejects(
      request("commands.execute", { id: "run" }),
      (e) => e.code === "command" && /busy.*Preparing/.test(e.message),
    );
    assert.deepEqual(await snapshot(), before);
    await wait(30);
    assert.equal(callbacks.length, callbackCount);
    await request("commands.update", {
      id: "run",
      revision: 5,
      state: { ...normal, checked: false },
    });
    await request("commands.execute", { id: "run" });
    await wait(30);
    assert.deepEqual(callbacks.at(-1), { id: "run", revision: 5 });
    checks.rejectionsAndLatestRevision = {
      snapshot: await snapshot(),
      callbacks: [...callbacks],
    };

    const helperPath = path.join(work, "helper");
    await host.plugins.load(helperPath);
    const firstTree = await until(
      (s) =>
        flatten(s.roots).some(
          (e) =>
            e.declaration.path.join("/") === "插件/独立辅助命令" &&
            e.revision === 2,
        ),
      "helper first callback",
    );
    const first = flatten(firstTree.roots).find(
      (e) => e.declaration.path.join("/") === "插件/独立辅助命令",
    );
    assert.notEqual(first.key.owner, own.key.owner);
    assert.equal(first.key.localId, "run");
    await request("commands.execute", { id: "run" });
    await wait(30);
    assert.deepEqual(callbacks.at(-1), { id: "run", revision: 5 });
    before = await snapshot();
    await assert.rejects(
      register("other-owner-path", ["插件", "独立辅助命令"]),
      (e) => e.code === "command" && /conflict/.test(e.message),
    );
    assert.deepEqual(await snapshot(), before);
    await host.plugins.disable("review.commands-helper");
    const removed = await snapshot();
    assert.ok(
      !flatten(removed.roots).some((e) => e.key.owner === first.key.owner),
    );
    assert.deepEqual(
      flatten(removed.roots).filter((e) => e.key.owner === own.key.owner),
      flatten(before.roots).filter((e) => e.key.owner === own.key.owner),
    );
    await host.plugins.load(helperPath);
    const secondTree = await until(
      (s) =>
        flatten(s.roots).some(
          (e) =>
            e.declaration.path.join("/") === "插件/独立辅助命令" &&
            e.revision === 2,
        ),
      "replacement helper",
    );
    const second = flatten(secondTree.roots).find(
      (e) => e.declaration.path.join("/") === "插件/独立辅助命令",
    );
    assert.notEqual(first.key.owner, second.key.owner);
    await wait(900); // Old helper's delayed execute must never run after disable.
    await host.plugins.disable("review.commands-helper");
    checks.ownerRoutingAndReload = {
      first,
      second,
      removed,
      final: await snapshot(),
      ownCallbacks: callbacks,
    };
    await fs.writeFile(
      path.join(work, "result.json"),
      JSON.stringify(
        {
          pass: true,
          checks,
          limits: [
            "Real JSONL owner routing and registry facts only; not native menu residency, shortcuts or final menu layout.",
            "Old delayed work is terminated; this does not force an already queued stale native menu event to arrive after replacement.",
          ],
        },
        null,
        2,
      ),
    );
    console.error("COMMANDS_REVIEW_PASS");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "failure.json"),
      JSON.stringify(
        {
          pass: false,
          error: String(error),
          stack: error.stack,
          checks,
          callbacks,
        },
        null,
        2,
      ),
    );
    throw error;
  }
}
async function helper() {
  const fs = await import("node:fs/promises"),
    path = await import("node:path");
  const waiters = [],
    callbacks = [];
  let owner;
  const request = await wire((event, data) => {
    if (event !== "commands.execute") return;
    callbacks.push(data);
    // Test evidence only, inspected by the external harness after host exit.
    void fs
      .appendFile(
        path.join(process.env.VESTRA_COMMANDS_REVIEW, "helper-callbacks.jsonl"),
        JSON.stringify({ owner, data }) + "\n",
      )
      .then(() => waiters.shift()?.())
      .catch(console.error);
  });
  const normal = { enabled: true, checked: null, busy: false, reason: null };
  await request("commands.register", {
    id: "run",
    revision: 1,
    declaration: {
      title: "独立辅助命令",
      path: ["插件", "独立辅助命令"],
      shortcut: null,
    },
    state: normal,
  });
  const tree = await request("commands.snapshot");
  owner = flatten(tree.roots).find(
    (e) => e.declaration.path.join("/") === "插件/独立辅助命令",
  ).key.owner;
  const received = new Promise((r) => waiters.push(r));
  await request("commands.execute", { id: "run" });
  await received;
  await request("commands.update", {
    id: "run",
    revision: 2,
    state: { ...normal, checked: true },
  });
  setTimeout(() => {
    void request("commands.execute", { id: "run" }).catch(console.error);
  }, 700);
}

for (const [name, activate] of [
  ["consumer", consumer],
  ["helper", helper],
]) {
  const directory = resolve(work, name);
  await mkdir(directory);
  await writeFile(
    resolve(directory, "manifest.json"),
    JSON.stringify({
      id: `review.commands-${name}`,
      apiVersion: "vestra.experimental/0.1",
      entry: "main.ts",
      capabilities: [
        "commands",
        ...(name === "consumer" ? ["plugins.manage"] : []),
      ],
    }),
  );
  await writeFile(
    resolve(directory, "main.ts"),
    `const wire=${wire.toString()};\nconst flatten=${flatten.toString()};\nexport const activate=${activate.toString()};\n`,
  );
}
const args = [
  "--headless",
  "--silent",
  "--seconds",
  "9",
  "--plugin",
  resolve(work, "consumer"),
];
const child = spawnSync(
  resolve(import.meta.dirname, "../host/target/debug/vestra-host"),
  args,
  {
    encoding: "utf8",
    timeout: 15000,
    env: { ...process.env, VESTRA_COMMANDS_REVIEW: work },
  },
);
await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
await writeFile(
  resolve(work, "command.json"),
  JSON.stringify(
    { args, work, exitCode: child.status, signal: child.signal },
    null,
    2,
  ),
);
console.log(JSON.stringify({ work, exitCode: child.status }));
assert.equal(child.status, 0, child.stderr);
assert.match(child.stderr, /COMMANDS_REVIEW_PASS/);
const result = JSON.parse(await readFile(resolve(work, "result.json"), "utf8"));
const callbacks = (
  await readFile(resolve(work, "helper-callbacks.jsonl"), "utf8")
)
  .trim()
  .split("\n")
  .map(JSON.parse);
const { first, second } = result.checks.ownerRoutingAndReload;
assert.deepEqual(
  callbacks.filter((e) => e.owner === first.key.owner).map((e) => e.data),
  [{ id: "run", revision: 1 }],
  "own execute must not leak into old helper; old timer must not run",
);
assert.deepEqual(
  callbacks.filter((e) => e.owner === second.key.owner).map((e) => e.data),
  [
    { id: "run", revision: 1 },
    { id: "run", revision: 2 },
  ],
);
await writeFile(
  resolve(work, "external-review.json"),
  JSON.stringify(
    {
      pass: true,
      callbacks,
      scope:
        "External harness verified exact helper owner callback counts after actual host exit",
    },
    null,
    2,
  ),
);
