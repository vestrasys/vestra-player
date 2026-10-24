// The plugin tools on their own (no model): scaffold, type check, tests in
// the restricted child, the release check, packing, the source export, and
// what they refuse.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { Toolchain, checkOwner, parseTap, parseTsc } from "../toolchain.ts";
import { runTool } from "../toolhost.ts";
import { dataDirectory } from "./fixture.mjs";
import { unzip } from "./unzip.mjs";

const plugin = path.resolve(import.meta.dirname, "..");

async function setup() {
  const data = await dataDirectory();
  const session = { dir: path.join(data, "sessions", "s-test"), work: path.join(data, "sessions", "s-test", "work") };
  await fs.mkdir(session.work, { recursive: true });
  return { data, session, toolchain: new Toolchain(plugin, data) };
}

test("the GitHub name becomes the id prefix; vestra and local are refused", () => {
  assert.equal(checkOwner(""), "");
  assert.equal(checkOwner(" OctoCat "), "octocat");
  assert.throws(() => checkOwner("vestra"), /官方插件/);
  assert.throws(() => checkOwner("local"), /local/);
  assert.throws(() => checkOwner("a b"), /字母、数字/);
});

test("scaffold → typecheck → test → check → pack: the template passes everything", async () => {
  const { data, session, toolchain } = await setup();
  const made = await toolchain.scaffold(session, { name: "demo", title: "演示" }, "octocat");
  assert.equal(made.ok, true, made.text);
  const project = path.join(session.work, "octocat.demo");
  for (const file of ["manifest.json", "main.ts", "ui/main.slint", "tests/main.test.ts", "README.md", "CHANGELOG.md", "package.json", "tsconfig.json"])
    await fs.access(path.join(project, file));
  assert.equal(JSON.parse(await fs.readFile(path.join(project, "manifest.json"), "utf8")).id, "octocat.demo");
  // The SDK copy where the project resolves it, without the CLI.
  await fs.access(path.join(data, "node_modules/@vestra-player/sdk/dist/plugin/testing.js"));
  await assert.rejects(fs.access(path.join(data, "node_modules/@vestra-player/sdk/bin")));
  assert.match((await toolchain.scaffold(session, { name: "demo" }, "octocat")).text, /已存在/);
  assert.match((await toolchain.scaffold(session, { name: "Bad Name" }, "")).text, /小写字母/);

  assert.deepEqual(await toolchain.typecheck(session, "octocat.demo"), { ok: true, text: "类型检查通过（2 个文件）" });
  assert.deepEqual(await toolchain.test(session, "octocat.demo"), { ok: true, text: "测试全部通过（1 个）" });
  const checked = await toolchain.check(session, "octocat.demo", "octocat");
  assert.equal(checked.ok, true, checked.text);
  const packed = await toolchain.pack(session, "octocat.demo", "octocat");
  assert.equal(packed.ok, true, packed.text);
  assert.equal(packed.file, path.join(session.dir, "dist", "octocat.demo-0.1.0.vxp"));
  const vxp = unzip(await fs.readFile(packed.file));
  assert.deepEqual([...vxp.keys()].sort(), ["CHANGELOG.md", "README.md", "main.js", "manifest.json", "package.json", "ui/main.slint"]);
  assert.deepEqual(await toolchain.projects(session).then((list) => list.map((p) => [p.id, p.name, p.version])), [["octocat.demo", "演示", "0.1.0"]]);
  // Nothing of a call stays behind in tmp/.
  assert.deepEqual(await fs.readdir(path.join(session.dir, "tmp")), []);
});

test("type errors and failed tests come back structured; the test process cannot leave the project", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  await fs.appendFile(path.join(project, "main.ts"), "const n: number = 'x';\n");
  const typed = await toolchain.typecheck(session, "local.demo");
  assert.equal(typed.ok, false);
  assert.match(typed.text, /^类型检查未通过：1 个错误\n/);
  const details = JSON.parse(typed.text.split("\n").slice(1).join("\n"));
  assert.deepEqual(details.errors, [{ file: "main.ts", line: 14, column: 7, code: "TS2322", message: "Type 'string' is not assignable to type 'number'." }]);

  const secret = path.join(session.dir, "meta.json");
  await fs.writeFile(secret, "{}");
  await fs.writeFile(
    path.join(project, "tests/escape.test.ts"),
    `import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
test("one is two", () => assert.equal(1, 2, "one is not two"));
test("reads outside", () => fs.readFileSync(${JSON.stringify(secret)}));
test("writes outside", () => fs.writeFileSync(${JSON.stringify(path.join(session.dir, "x.txt"))}, "x"));
test("network", async () => { await fetch("http://127.0.0.1:9/"); });
test("child process", async () => { (await import("node:child_process")).execFileSync(process.execPath, ["-v"]); });
test("object diff", () => {
  console.log("debug state", JSON.stringify({ phase: "ready" }));
  console.error("debug on stderr");
  assert.deepEqual({ error: null, n: 1 }, { error: null, n: 2 });
});
`,
  );
  const tested = await toolchain.test(session, "local.demo");
  assert.equal(tested.ok, false);
  assert.match(tested.text, /^测试未通过：6 个失败，1 个通过/);
  const report = JSON.parse(tested.text.split("\n").slice(1).join("\n"));
  const failure = (name) => report.failures.find((f) => f.name === name);
  assert.match(failure("one is two").error, /one is not two/);
  assert.match(failure("one is two").location, /^tests\/escape\.test\.ts:4:1$/);
  assert.match(failure("reads outside").error, /allow-fs-read/);
  assert.match(failure("writes outside").error, /allow-fs-write/);
  assert.ok(failure("network"), "the network is refused");
  assert.match(failure("child process").error, /allow-child-process|restricted/);
  // The assertion's own message, not the `error: ~` of the compared objects; and what the tests printed.
  assert.match(failure("object diff").error, /^Expected values to be strictly deep-equal:[\s\S]*n: 1/);
  assert.match(report.output, /debug state \{"phase":"ready"\}/);
  assert.match(report.output, /debug on stderr/);
  assert.doesNotMatch(report.output, /^(TAP version|ok |not ok |# )/m);
  // The template's own test still fails in the same run: the main.ts type error is only a type error.
  await assert.rejects(fs.access(path.join(session.dir, "x.txt")));
});

test("the check refuses vestra. ids, npm dependencies and a manifest id that is not the folder", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  await fs.writeFile(path.join(project, "package.json"), JSON.stringify({ name: "x", dependencies: { "left-pad": "1.0.0" } }));
  const manifest = JSON.parse(await fs.readFile(path.join(project, "manifest.json"), "utf8"));
  await fs.writeFile(path.join(project, "manifest.json"), JSON.stringify({ ...manifest, id: "vestra.demo" }));
  const checked = await toolchain.check(session, "local.demo", "");
  assert.equal(checked.ok, false);
  assert.match(checked.text, /left-pad/);
  assert.match(checked.text, /要和文件夹名「local\.demo」一致/);
  assert.equal((await toolchain.pack(session, "local.demo", "")).ok, false);

  await fs.mkdir(path.join(session.work, "vestra.demo"));
  await fs.writeFile(path.join(session.work, "vestra.demo/manifest.json"), JSON.stringify({ ...manifest, id: "vestra.demo" }));
  assert.match((await toolchain.check(session, "vestra.demo", "")).text, /「vestra\.」留给官方插件/);
  assert.match((await runTool(toolchain, session, "", "typecheck", { id: "../x" }, new AbortController().signal)).text, /不是插件 id/);
  assert.match((await runTool(toolchain, session, "", "typecheck", { id: "a.missing" }, new AbortController().signal)).text, /没有插件项目/);
});

test("a tool call is cancelled with its signal", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  await fs.writeFile(path.join(session.work, "local.demo/tests/slow.test.ts"), `import test from "node:test";\ntest("slow", () => new Promise((r) => setTimeout(r, 20_000)));\n`);
  const controller = new AbortController();
  const running = toolchain.test(session, "local.demo", controller.signal);
  setTimeout(() => controller.abort(), 500);
  const started = Date.now();
  assert.deepEqual(await running, { ok: false, text: "测试已取消" });
  assert.ok(Date.now() - started < 5000);
});

test("the source export is the user's own repository: README, tests, CI, LICENSE placeholder, the SDK as dev dependency", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo", title: "演示" }, "");
  await toolchain.pack(session, "local.demo", ""); // leaves work/local.demo/dist/
  const { data, fileName, notes } = await toolchain.exportSource(session, "local.demo");
  assert.equal(fileName, "local.demo-0.1.0-source.zip");
  const files = unzip(data);
  assert.deepEqual([...files.keys()].sort(), [
    "demo/.github/workflows/ci.yml",
    "demo/.gitignore",
    "demo/CHANGELOG.md",
    "demo/LICENSE",
    "demo/README.md",
    "demo/main.ts",
    "demo/manifest.json",
    "demo/package.json",
    "demo/tests/main.test.ts",
    "demo/tsconfig.json",
    "demo/ui/main.slint",
  ]);
  const pkg = JSON.parse(files.get("demo/package.json").toString());
  assert.equal(pkg.devDependencies["@vestra-player/sdk"], `^${toolchain.sdkVersion}`);
  assert.equal(pkg.scripts.test, "vestra-plugin test");
  assert.match(files.get("demo/.github/workflows/ci.yml").toString(), /npx vestra-plugin check[\s\S]*npx vestra-plugin test[\s\S]*npx vestra-plugin pack/);
  assert.match(files.get("demo/LICENSE").toString(), /TODO: choose a license/);
  assert.ok(notes.some((n) => /local\./.test(n)));
  // The same bytes for the same project.
  assert.deepEqual((await toolchain.exportSource(session, "local.demo")).data, data);
});

test("parsers: tsc lines and the TAP report", () => {
  assert.deepEqual(parseTsc("main.ts(3,5): error TS2304: Cannot find name 'x'.\n  more text\nother"), [
    { file: "main.ts", line: 3, column: 5, code: "TS2304", message: "Cannot find name 'x'. more text" },
  ]);
  const tap = ["not ok 1 - adds", "  ---", "  location: '/p/tests/a.test.ts:2:1'", "  error: 'boom'", "  ...", "# pass 3", "# fail 1"].join("\n");
  assert.deepEqual(parseTap(tap, "/p"), { passed: 3, failed: 1, failures: [{ name: "adds", error: "boom", location: "tests/a.test.ts:2:1" }] });
});
