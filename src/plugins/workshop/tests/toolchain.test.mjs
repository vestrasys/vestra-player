// The plugin tools on their own (no model): scaffold, type check, tests in
// the restricted child, the release check, packing, the source export, and
// what they refuse.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { Toolchain, checkOwner, nodeOption, parseTap, parseTsc } from "../toolchain.ts";
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

/**
 * The plugin as the player installs it: what the toolchain runs (vendor/
 * without pi, worker/vendor-resolve.ts) copied to a folder with spaces in its
 * path ("C:\Program Files\Vestra Player\…") and no node_modules above it,
 * with a data folder that has spaces too (a user name).
 */
async function installedCopy() {
  const root = await dataDirectory();
  const copy = path.join(root, "Vestra Player", "plugins", "vestra.workshop");
  for (const tree of ["vendor/@vestra-player", "vendor/typescript", "vendor/@types", "vendor/undici-types"]) await fs.cp(path.join(plugin, tree), path.join(copy, tree), { recursive: true });
  await fs.mkdir(path.join(copy, "worker"));
  await fs.copyFile(path.join(plugin, "worker/vendor-resolve.ts"), path.join(copy, "worker/vendor-resolve.ts"));
  const data = path.join(root, "plugin data with spaces");
  const session = { dir: path.join(data, "sessions", "s-test"), work: path.join(data, "sessions", "s-test", "work") };
  await fs.mkdir(session.work, { recursive: true });
  return { copy, session, toolchain: new Toolchain(copy, data) };
}

test("installed: typecheck, check and pack work from a path with spaces and no node_modules above", async () => {
  const { session, toolchain } = await installedCopy();
  assert.equal((await toolchain.scaffold(session, { name: "demo" }, "octocat")).ok, true);
  const typed = await toolchain.typecheck(session, "octocat.demo");
  assert.equal(typed.ok, true, typed.text);
  const checked = await toolchain.check(session, "octocat.demo", "octocat");
  assert.equal(checked.ok, true, checked.text);
  const packed = await toolchain.pack(session, "octocat.demo", "octocat");
  assert.equal(packed.ok, true, packed.text);
  await fs.access(packed.file);
  // Still confined: a type error in a file outside the project is not read.
  const outside = path.join(session.dir, "outside.d.ts");
  await fs.writeFile(outside, "declare const outside: number;\n");
  const main = path.join(session.work, "octocat.demo", "main.ts");
  await fs.writeFile(main, `/// <reference path=${JSON.stringify(outside)} />\n${await fs.readFile(main, "utf8")}export const seen: number = outside;\n`);
  const denied = await toolchain.typecheck(session, "octocat.demo");
  assert.equal(denied.ok, false, denied.text);
  assert.match(denied.text, /TS6053|TS2304/);
});

test("a CLI that fails without its report: the result has the exit code and stderr, never an empty failure", async () => {
  const { copy, session, toolchain } = await installedCopy();
  assert.equal((await toolchain.scaffold(session, { name: "demo" }, "octocat")).ok, true);
  // The CLI cannot load: it dies with a stack trace and no report lines.
  await fs.rm(path.join(copy, "vendor/typescript"), { recursive: true });
  for (const run of [() => toolchain.typecheck(session, "octocat.demo"), () => toolchain.check(session, "octocat.demo", "octocat"), () => toolchain.pack(session, "octocat.demo", "octocat")]) {
    const failed = await run();
    assert.equal(failed.ok, false);
    assert.match(failed.text, /exited with code 1 without a report/);
    assert.match(failed.text, /"exitCode": 1/);
    assert.match(failed.text, /"stderr": ".*typescript/s);
  }
});

test("NODE_OPTIONS flags keep spaces, quotes and backslashes", () => {
  assert.equal(nodeOption("--allow-fs-read=C:\\Program Files\\Vestra Player"), '"--allow-fs-read=C:\\\\Program Files\\\\Vestra Player"');
  assert.equal(nodeOption('--x=a"b'), '"--x=a\\"b"');
});

test("the GitHub name becomes the id prefix; vestra and local are refused", () => {
  assert.equal(checkOwner(""), "");
  assert.equal(checkOwner(" OctoCat "), "octocat");
  assert.throws(() => checkOwner("vestra"), /reserved for official plugins/);
  assert.throws(() => checkOwner("local"), /local/);
  assert.throws(() => checkOwner("a b"), /only letters, digits/);
});

test("scaffold → typecheck → test → check → pack: the template passes everything", async () => {
  const { data, session, toolchain } = await setup();
  const made = await toolchain.scaffold(session, { name: "demo", title: "演示" }, "octocat");
  assert.equal(made.ok, true, made.text);
  const project = path.join(session.work, "octocat.demo");
  for (const file of ["manifest.json", "main.ts", "ui/main.slint", "tests/main.test.ts", "README.md", "CHANGELOG.md", "package.json", "tsconfig.json"])
    await fs.access(path.join(project, file));
  const manifest = JSON.parse(await fs.readFile(path.join(project, "manifest.json"), "utf8"));
  assert.equal(manifest.id, "octocat.demo");
  // SDK 1.9.0: no declarations in the manifest (player 0.9.0 refuses the field as unknown; an older one requires it) and no editor plugin in tsconfig.json.
  assert.equal("capabilities" in manifest, false);
  assert.equal(manifest.minPlayerVersion, "0.9.0");
  assert.equal(JSON.parse(await fs.readFile(path.join(project, "tsconfig.json"), "utf8")).compilerOptions.plugins, undefined);
  // The SDK copy where the project resolves it, without the CLI.
  await fs.access(path.join(data, "node_modules/@vestra-player/sdk/dist/plugin/testing.js"));
  await assert.rejects(fs.access(path.join(data, "node_modules/@vestra-player/sdk/bin")));
  assert.match((await toolchain.scaffold(session, { name: "demo" }, "octocat")).text, /already exists/);
  assert.match((await toolchain.scaffold(session, { name: "Bad Name" }, "")).text, /lower-case letters/);

  assert.deepEqual(await toolchain.typecheck(session, "octocat.demo"), { ok: true, text: "Type check passed (2 files)" });
  assert.deepEqual(await toolchain.test(session, "octocat.demo"), { ok: true, text: "All tests passed (1)" });
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
  assert.match(typed.text, /^Type check failed: 1 error\(s\)\n/);
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
  assert.match(tested.text, /^Tests failed: 6 failed, 1 passed/);
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
  const typed = await toolchain.typecheck(session, "local.demo");
  assert.equal(typed.ok, false);
  assert.match(typed.text, /left-pad/);
  const checked = await toolchain.check(session, "local.demo", "");
  assert.equal(checked.ok, false);
  assert.match(checked.text, /left-pad/);
  assert.match(checked.text, /must match the folder name \\?"local\.demo\\?"/);
  assert.equal((await toolchain.pack(session, "local.demo", "")).ok, false);

  await fs.mkdir(path.join(session.work, "vestra.demo"));
  await fs.writeFile(path.join(session.work, "vestra.demo/manifest.json"), JSON.stringify({ ...manifest, id: "vestra.demo" }));
  assert.match((await toolchain.check(session, "vestra.demo", "")).text, /\\?"vestra\.\\?" is reserved for official plugins/);
  assert.match((await runTool(toolchain, session, "", "typecheck", { id: "../x" }, new AbortController().signal)).text, /Not a plugin id/);
  assert.match((await runTool(toolchain, session, "", "typecheck", { id: "a.missing" }, new AbortController().signal)).text, /No plugin project/);
  // Model-facing results are English, like the rest of the tools' text.
  const unknown = await runTool(toolchain, session, "", "rm_rf", {}, new AbortController().signal);
  assert.deepEqual(unknown, { ok: false, text: "No tool named rm_rf" });
  // install_plugin without a player (the eval runner) is not a tool either.
  assert.equal((await runTool(toolchain, session, "", "install_plugin", { id: "local.demo" }, new AbortController().signal)).text, "No tool named install_plugin");
});

test("the check refuses an exported brush or color property the plugin would write (the player refuses the value only at run time)", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  const slint = await fs.readFile(path.join(project, "ui/main.slint"), "utf8");
  // A helper component may take a brush from Slint code; the exported window may not take one from the plugin.
  await fs.writeFile(
    path.join(project, "ui/main.slint"),
    `import { VerticalBox } from "std-widgets.slint";
// in property <brush> commented-out;
component Dot inherits Rectangle { in property <brush> tint; background: self.tint; }
${slint.replace("export component MainWindow inherits Window {", 'export component MainWindow inherits Window {\n    in property <string> label: "a { brace } in a string";\n    in property <brush> high-tint;\n    in-out property <color> low-tint;\n    Dot { tint: root.high-tint; in property <brush> nested; }')}
export global Bridge { in property <color> accent; in property <float> strength; }
`,
  );
  const checked = await toolchain.check(session, "local.demo", "");
  assert.equal(checked.ok, false);
  assert.match(checked.text, /^Check failed: 3 problem\(s\)/);
  const { problems } = JSON.parse(checked.text.split("\n").slice(1).join("\n"));
  const written = (await fs.readFile(path.join(project, "ui/main.slint"), "utf8")).split("\n");
  const lineOf = (text) => written.findIndex((l) => l.includes(text)) + 1;
  assert.deepEqual(
    problems.map((p) => p.split(":").slice(0, 3).join(":")),
    [
      `ui/main.slint:${lineOf("<brush> high-tint")}: in property <brush> high-tint in component MainWindow`,
      `ui/main.slint:${lineOf("<color> low-tint")}: in-out property <color> low-tint in component MainWindow`,
      `ui/main.slint:${lineOf("<color> accent")}: in property <color> accent in global Bridge`,
    ],
  );
  assert.match(problems[0], /cannot write a brush from a plugin[\s\S]*compute the brush in Slint/);
  // Pure Slint text passes: the same file without the plugin-side colour properties.
  await fs.writeFile(path.join(project, "ui/main.slint"), slint.replace("export component MainWindow inherits Window {", "export component MainWindow inherits Window {\n    in property <float> hue;\n    Rectangle { in property <brush> tint; }"));
  const passed = await toolchain.check(session, "local.demo", "");
  assert.equal(passed.ok, true, passed.text);
});

test("the brush check skips the exported components of a .slint file another one imports (a parts library) and vendor/, not the entry window", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  const slint = await fs.readFile(path.join(project, "ui/main.slint"), "utf8");
  // Slint needs `export` to import a component from another file: Swatch is a helper.
  await fs.writeFile(path.join(project, "ui/parts.slint"), "export component Swatch inherits Rectangle { in property <brush> fill; background: self.fill; }\n");
  // A copy of someone else's Slint, like the SDK's icons.slint (`export component Icon { in property <brush> color; }`).
  await fs.mkdir(path.join(project, "ui/vendor"), { recursive: true });
  await fs.writeFile(path.join(project, "ui/vendor/icons.slint"), "export component Icon inherits Rectangle { in property <brush> color; }\n");
  const entry = `import { Swatch } from "./parts.slint";\n${slint.replace("export component MainWindow inherits Window {", "export component MainWindow inherits Window {\n    in property <float> hue;\n    Swatch { fill: #336699; }")}`;
  await fs.writeFile(path.join(project, "ui/main.slint"), entry);
  const passed = await toolchain.check(session, "local.demo", "");
  assert.equal(passed.ok, true, passed.text);
  // The entry window's own brush property is still refused.
  await fs.writeFile(path.join(project, "ui/main.slint"), entry.replace("in property <float> hue;", "in property <brush> hue;"));
  const checked = await toolchain.check(session, "local.demo", "");
  assert.equal(checked.ok, false);
  const { problems } = JSON.parse(checked.text.split("\n").slice(1).join("\n"));
  assert.deepEqual(
    problems.map((p) => p.split(":").slice(0, 1).join(":") + ":" + p.split(": ")[1]),
    ["ui/main.slint:in property <brush> hue in component MainWindow"],
  );
});

test("the type check runs under the permission model: it reads the project and the SDK, nothing else", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  // Outside the project: a declaration the project points at. Without the
  // permission model tsc reads it and the check passes.
  const outside = path.join(session.dir, "outside.d.ts");
  await fs.writeFile(outside, "declare const outside: number;\n");
  const main = await fs.readFile(path.join(project, "main.ts"), "utf8");
  await fs.writeFile(path.join(project, "main.ts"), `/// <reference path=${JSON.stringify(outside)} />\n${main}export const seen: number = outside;\n`);
  const typed = await toolchain.typecheck(session, "local.demo");
  assert.equal(typed.ok, false, typed.text);
  assert.match(typed.text, /TS6053|TS2304/);
  // check and pack (not confined yet, see Toolchain.confined) still write dist/ and the .vxp.
  await fs.writeFile(path.join(project, "main.ts"), main);
  assert.equal((await toolchain.check(session, "local.demo", "")).ok, true);
  assert.equal((await toolchain.pack(session, "local.demo", "")).ok, true);
});

test("a tool call is cancelled with its signal", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  await fs.writeFile(path.join(session.work, "local.demo/tests/slow.test.ts"), `import test from "node:test";\ntest("slow", () => new Promise((r) => setTimeout(r, 20_000)));\n`);
  const controller = new AbortController();
  const running = toolchain.test(session, "local.demo", controller.signal);
  setTimeout(() => controller.abort(), 500);
  const started = Date.now();
  assert.deepEqual(await running, { ok: false, text: "Tests cancelled", say: { code: "cancelled", args: ["Tests"] } });
  assert.ok(Date.now() - started < 5000);
});

test("the source export is the user's own repository: README, tests, CI, LICENSE placeholder, the SDK as dev dependency", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo", title: "演示" }, "");
  await toolchain.pack(session, "local.demo", ""); // leaves work/local.demo/dist/
  const { data, fileName, local } = await toolchain.exportSource(session, "local.demo");
  assert.equal(fileName, "local.demo-0.1.0-source.zip");
  const files = unzip(data);
  assert.deepEqual([...files.keys()].sort(), [
    "demo/.github/workflows/ci.yml",
    "demo/.gitignore",
    "demo/.vscode/settings.json",
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
  // No editor plugin (SDK 1.9.0); the editor uses the TypeScript that npm install brings along with the SDK.
  assert.equal(JSON.parse(files.get("demo/tsconfig.json").toString()).compilerOptions.plugins, undefined);
  assert.deepEqual(JSON.parse(files.get("demo/.vscode/settings.json").toString()), {
    "typescript.tsdk": "node_modules/typescript/lib",
    "typescript.enablePromptUseWorkspaceTsdk": true,
  });
  assert.equal(local, true, "the card says what to change for a local. id");
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

test("SDK 1.9.0: an SDK call needs no declaration; a manifest field the SDK does not know fails, a former one too", async () => {
  const { session, toolchain } = await setup();
  await toolchain.scaffold(session, { name: "demo" }, "");
  const project = path.join(session.work, "local.demo");
  const main = path.join(project, "main.ts");
  await fs.writeFile(main, (await fs.readFile(main, "utf8")).replace("run: () => mainWindow.show(),", "run: () => { mainWindow.show(); void plugin.player.seek(0); },"));
  // Before 1.9.0 both failed here ("1 SDK use(s) need a capability manifest.json does not declare", playback.control).
  for (const run of [await toolchain.typecheck(session, "local.demo"), await toolchain.check(session, "local.demo", "")]) assert.equal(run.ok, true, run.text);

  const file = path.join(project, "manifest.json");
  const manifest = JSON.parse(await fs.readFile(file, "utf8"));
  await fs.writeFile(file, `${JSON.stringify({ ...manifest, capabilities: ["commands", "ui.window", "playback.control"] }, null, 2)}\n`);
  for (const run of [await toolchain.typecheck(session, "local.demo"), await toolchain.check(session, "local.demo", "")]) {
    assert.equal(run.ok, false, run.text);
    const report = JSON.parse(run.text.slice(run.text.indexOf("\n") + 1));
    assert.equal(report.undeclared, undefined);
    // An unknown field (SDK 1.11.0: no message of its own; before, one that said to delete it).
    assert.match(report.problems.join("\n"), /"capabilities"/);
  }
});
