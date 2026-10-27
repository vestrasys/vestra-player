// check has the player compile the packed plugin (packages.validateArchive,
// Slint included) before any install: the compiler in the loop, so a Slint
// error reaches the model while it is still working, with the player's own
// diagnostics. The player side is simulated with a TestHost handler.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { Toolchain } from "../toolchain.ts";
import { dataDirectory, server, startAi } from "./fixture.mjs";

const plugin = path.resolve(import.meta.dirname, "..");

/**
 * What the player (0.5.10) answers for a Slint file that does not compile:
 * code `slint`, the message its Debug form of Slint's diagnostics, with the
 * file in the player's extraction folder and a byte offset, not a line.
 */
const SLINT_ERROR =
  '[Diagnostic { message: "\'changed\' is not a callback in SearchField", span: SourceLocation { source_file: Some("C:\\\\Users\\\\me\\\\AppData\\\\Local\\\\Temp\\\\.tmpAbC123\\\\ui\\\\main.slint"), span: Span { offset: 412, length: 7 } }, level: Error }]';

async function session() {
  const data = await dataDirectory();
  const paths = { dir: path.join(data, "sessions", "s-test"), work: path.join(data, "sessions", "s-test", "work") };
  await fs.mkdir(paths.work, { recursive: true });
  return { data, paths };
}

test("check: the player's Slint diagnostics come back whole as a failed check, and the scratch package is removed", async () => {
  const { data, paths } = await session();
  const seen = [];
  const toolchain = new Toolchain(plugin, data, async (file) => {
    seen.push({ file, exists: await fs.stat(file).then((s) => s.isFile()) });
    throw new HostError("slint", SLINT_ERROR);
  });
  assert.equal((await toolchain.scaffold(paths, { name: "demo" }, "")).ok, true);
  const checked = await toolchain.check(paths, "local.demo", "");
  assert.equal(checked.ok, false, checked.text);
  assert.match(checked.text, /^Check failed: the player's Slint compile reported errors\n/);
  const details = JSON.parse(checked.text.slice(checked.text.indexOf("\n") + 1));
  assert.deepEqual(details.problems, [SLINT_ERROR]);
  assert.match(details.next, /run check again/);
  assert.equal(seen.length, 1);
  assert.match(path.basename(seen[0].file), /^local\.demo-0\.1\.0\.vxp$/);
  assert.equal(seen[0].exists, true);
  await assert.rejects(fs.access(seen[0].file), "the scratch package is gone after the check");

  // Another refusal keeps its code in the summary.
  const other = new Toolchain(plugin, data, async () => {
    throw new HostError("javascript", "main.js:3\nSyntaxError: Unexpected token");
  });
  const refused = await other.check(paths, "local.demo", "");
  assert.equal(refused.ok, false);
  assert.match(refused.text, /^Check failed: the player's package check refused it \(javascript\)/);
  assert.match(refused.text, /SyntaxError: Unexpected token/);
});

test("check: when the player accepts the package, the check says Slint was compiled", async () => {
  const { data, paths } = await session();
  let calls = 0;
  const toolchain = new Toolchain(plugin, data, async () => {
    calls++;
    return { checks: ["manifest", "digest", "productionJavaScript", "slintCompile"] };
  });
  assert.equal((await toolchain.scaffold(paths, { name: "demo" }, "")).ok, true);
  const checked = await toolchain.check(paths, "local.demo", "");
  assert.equal(checked.ok, true, checked.text);
  assert.match(checked.text, /^Check passed \(manifest, strict type check, build, the player's Slint compile\)/);
  assert.equal(calls, 1);
  assert.deepEqual(await fs.readdir(path.join(paths.dir, "tmp")).catch(() => []), [], "no scratch folder left");
});

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

test("in the player: the agent's check calls packages.validateArchive and the model reads the Slint error", async () => {
  const mock = await server(
    scripted({
      做个插件: [{ toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] }],
      检查: [{ toolCalls: [{ name: "check", args: { id: "local.demo" } }] }],
    }),
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  const files = [];
  ai.host.handle("packages.validateArchive", ({ file }) => {
    files.push(String(file));
    throw new HostError("slint", SLINT_ERROR);
  });
  await ai.ask("检查");
  assert.equal(files.length, 1);
  assert.match(path.basename(files[0]), /^local\.demo-0\.1\.0\.vxp$/);
  const told = mock.chats.at(-1).body.messages.at(-1);
  const text = typeof told.content === "string" ? told.content : JSON.stringify(told.content);
  assert.match(text, /Check failed: the player's Slint compile reported errors/);
  assert.match(text, /'changed' is not a callback in SearchField/);
  assert.match(text, /main\.slint/);
  assert.match(text, /run check again/);
  // The system prompt tells the model check compiles Slint.
  assert.match(mock.chats.at(-1).body.messages[0].content, /check also has the player compile the packed plugin, every \.slint file included/);
  assert.deepEqual(ai.host.packageCalls, [], "nothing installed");
  await ai.stop();
});
