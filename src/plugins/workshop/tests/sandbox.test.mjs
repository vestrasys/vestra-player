// The agent's boundaries: the file layer of its tools and the Node permission
// model of its process.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { workerArguments, workerEnvironment } from "../agent.ts";
import { WorkFolder } from "../worker/files.ts";
import { dataDirectory, server, startAi } from "./fixture.mjs";

async function folder() {
  const root = await dataDirectory();
  await fs.mkdir(path.join(root, "work"));
  await fs.writeFile(path.join(root, "secret.txt"), "outside");
  return { root, work: await WorkFolder.open(path.join(root, "work")) };
}

test("the work folder: inside is fine, outside, links and hidden names are refused", async () => {
  const { root, work } = await folder();
  await work.writeFile("src/main.ts", "export {};\n");
  assert.equal(await fs.readFile(path.join(root, "work/src/main.ts"), "utf8"), "export {};\n");
  assert.equal((await work.readFile(path.join(work.root, "src/main.ts"))).toString(), "export {};\n");
  assert.deepEqual(await work.walk(".", 100), ["src/main.ts"]);

  await assert.rejects(work.readFile("../secret.txt"), /工作目录以外/);
  await assert.rejects(work.readFile(path.join(root, "secret.txt")), /工作目录以外/);
  await assert.rejects(work.readFile("/etc/hosts"), /工作目录以外/);
  await assert.rejects(work.writeFile(".pi/settings.json", "{}"), /以「.」开头/);
  await assert.rejects(work.writeFile("src/.env", "x"), /以「.」开头/);

  await fs.symlink(path.join(root, "secret.txt"), path.join(root, "work/link.txt"));
  await fs.symlink(root, path.join(root, "work/up"));
  await assert.rejects(work.readFile("link.txt"), /符号链接/);
  await assert.rejects(work.writeFile("link.txt", "x"), /符号链接/);
  await assert.rejects(work.writeFile("up/secret.txt", "x"), /符号链接/);
  assert.equal(await fs.readFile(path.join(root, "secret.txt"), "utf8"), "outside");
  assert.deepEqual(await work.readdir("."), ["src"]);
  assert.deepEqual(await work.walk(".", 100), ["src/main.ts"]);

  await assert.rejects(work.writeFile("big.txt", "x".repeat(1024 * 1024 + 1)), /1 MB/);

  // delete: a file or an empty folder inside; never through a link, never outside or the folder itself.
  await work.writeFile("src/debug.test.ts", "x");
  assert.equal(await work.remove("src/debug.test.ts"), "src/debug.test.ts");
  await assert.rejects(work.remove("src"), /不是空的/);
  await assert.rejects(work.remove("link.txt"), /符号链接/);
  await assert.rejects(work.remove("up/secret.txt"), /符号链接/);
  await assert.rejects(work.remove("../secret.txt"), /工作目录以外/);
  await assert.rejects(work.remove("."), /工作目录本身/);
  assert.equal(await fs.readFile(path.join(root, "secret.txt"), "utf8"), "outside");
  await work.remove("src/main.ts");
  await work.remove("src");
  assert.deepEqual(await work.readdir("."), []);
});

test("the worker process cannot read outside, spawn, or see the key in its environment", async () => {
  const session = await fs.realpath(await dataDirectory());
  const args = workerArguments(session);
  assert.ok(args.includes("--permission"));
  assert.ok(!args.some((a) => /allow-(child-process|worker|addons|wasi)/.test(a)));
  assert.deepEqual(args.filter((a) => a.startsWith("--allow-fs-write")), [`--allow-fs-write=${session}`]);
  const env = workerEnvironment();
  assert.equal(env.PI_TELEMETRY, "0");
  assert.equal(env.PI_OFFLINE, "1");
  assert.ok(!Object.keys(env).some((name) => /KEY|TOKEN|SECRET/i.test(name)));

  const probe = `
    const fs = require("node:fs"), cp = require("node:child_process");
    const tryIt = (f) => { try { f(); return "ok"; } catch (e) { return e.code; } };
    console.log(JSON.stringify({
      home: tryIt(() => fs.readdirSync(${JSON.stringify(os.homedir())})),
      writeOutside: tryIt(() => fs.writeFileSync(${JSON.stringify(path.join(os.tmpdir(), "vestra-ai-escape"))}, "x")),
      writeSession: tryIt(() => fs.writeFileSync(${JSON.stringify(path.join(session, "ok.txt"))}, "x")),
      spawn: tryIt(() => cp.spawnSync(process.execPath, ["-v"])),
    }));`;
  const result = spawnSync(process.execPath, [...args.slice(0, -1), "-e", probe], { env, encoding: "utf8" });
  assert.deepEqual(JSON.parse(result.stdout), { home: "ERR_ACCESS_DENIED", writeOutside: "ERR_ACCESS_DENIED", writeSession: "ok", spawn: "ERR_ACCESS_DENIED" });
});

test("a tool call that tries to leave the work folder fails and the turn goes on", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).role === "tool"
      ? { text: ["读不到。"] }
      : { toolCalls: [{ name: "read", args: { path: "../meta.json" } }] },
  );
  const ai = await startAi({ compat: mock.url });
  await ai.ask("读 ../meta.json");
  assert.equal(ai.kinds(), "user,tools,assistant");
  assert.equal(ai.rows()[1].state, 2);
  assert.equal(ai.rows()[1].detail, "✗ 读取 ../meta.json");
  const result = mock.chats[1].body.messages.at(-1);
  assert.match(typeof result.content === "string" ? result.content : JSON.stringify(result.content), /工作目录以外/);
  await ai.stop();
});

test("delete removes a debug file from the work folder in the worker", async () => {
  const mock = await server((req) => {
    const done = req.body.messages.filter((m) => m.role === "tool").length;
    if (done === 0) return { toolCalls: [{ name: "write", args: { path: "demo/tests/debug.test.ts", content: "x" } }] };
    if (done === 1) return { toolCalls: [{ name: "delete", args: { path: "demo/tests/debug.test.ts" } }] };
    return { text: ["删好了。"] };
  });
  const ai = await startAi({ compat: mock.url });
  await ai.ask("写一个调试文件再删掉");
  const sessions = path.join(ai.plugin.dataDirectory, "sessions");
  const [session] = await fs.readdir(sessions);
  await assert.rejects(fs.access(path.join(sessions, session, "work/demo/tests/debug.test.ts")));
  await fs.access(path.join(sessions, session, "work/demo/tests"));
  const result = mock.chats[2].body.messages.at(-1);
  assert.match(typeof result.content === "string" ? result.content : JSON.stringify(result.content), /Deleted demo\/tests\/debug\.test\.ts/);
  await ai.stop();
});
