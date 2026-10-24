// The eval harness and the agent's plugin tool chain, end to end on the mock
// server: the model reads the reference, scaffolds, writes a type error,
// type-checks, fixes it, tests and checks. No paid model is called.
import assert from "node:assert/strict";
import test from "node:test";
import { runTask } from "./eval/harness.mjs";
import { server } from "./fixture.mjs";

const BROKEN = `import type { Plugin } from "@vestra-player/sdk";
export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("ui/main.slint");
  const count: number = "zero";
  window.bind(() => ({ message: String(count) }));
  plugin.commands.add({ id: "demo.open", path: ["演示…"], run: () => window.show() });
}
`;
const FIXED = BROKEN.replace(`const count: number = "zero";`, "const count = 0;").replace("String(count)", `"Hello"`);

/** The scripted model: one step per tool result so far. */
function script(seen) {
  return (req) => {
    const messages = req.body.messages;
    const results = messages.filter((m) => m.role === "tool");
    seen.push(results.at(-1)?.content);
    const index = /Index (\S+llms\.txt):/.exec(messages[0].content)?.[1];
    const steps = [
      { toolCalls: [{ name: "read", args: { path: index } }] },
      { toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] },
      { toolCalls: [{ name: "write", args: { path: "eval.demo/main.ts", content: BROKEN } }] },
      { toolCalls: [{ name: "typecheck", args: { id: "eval.demo" } }] },
      { toolCalls: [{ name: "write", args: { path: "eval.demo/main.ts", content: FIXED } }] },
      { toolCalls: [{ name: "typecheck", args: { id: "eval.demo" } }] },
      { toolCalls: [{ name: "test", args: { id: "eval.demo" } }] },
      { toolCalls: [{ name: "check", args: { id: "eval.demo" } }] },
      { text: ["完成：eval.demo 通过了类型检查、测试和检查。"] },
    ];
    return steps[results.length] ?? { text: ["?"] };
  };
}

const text = (content) => (typeof content === "string" ? content : JSON.stringify(content ?? ""));

test("the harness: the agent reads the reference, fixes a type error and passes; fix rounds count the failed check", async () => {
  const seen = [];
  const mock = await server(script(seen));
  const result = await runTask({ id: "mock", prompt: "写一个演示插件" }, { provider: "custom-openai", modelId: "mock-1", baseUrl: mock.url, api: "openai-completions", key: "sk-eval" }, { timeoutMs: 120_000 });
  assert.equal(result.error, null);
  assert.equal(result.pass, true, JSON.stringify(result));
  assert.equal(result.fixRounds, 1);
  assert.deepEqual(result.projects, [{ id: "eval.demo", typecheck: true, test: true, check: true, pass: true }]);

  // What the model saw: the reference, the scaffold, the structured type error, the passes.
  assert.match(text(seen[1]), /Vestra Player (plugin )?SDK/);
  assert.match(text(seen[2]), /Created the plugin project work\/eval\.demo/);
  assert.match(text(seen[4]), /Type check failed: 1 error/);
  assert.match(text(seen[4]), /"file": "main\.ts"[\s\S]*"line": 4[\s\S]*TS2322/);
  assert.match(text(seen[6]), /Type check passed/);
  assert.match(text(seen[7]), /All tests passed \(1\)/);
  assert.match(text(seen[8]), /Check passed/);

  // The system prompt: the checklist and where the reference is; no full reference inline.
  const system = mock.chats[0].body.messages[0].content;
  for (const rule of ["Read the documentation before writing code", "<owner>.<name>", "vestra.", "import type", "typecheck with 0 errors", "llms-full.txt", "including the notes between tool calls", "Keep progress notes to a minimum", "Leave no debugging files behind", "state.phase", "testing.d.ts", "The user's interface language is English"]) assert.ok(system.includes(rule), rule);
  assert.ok(system.length < 6000, `${system.length} characters`);
  // Without a player: no install tool.
  const tools = mock.chats[0].body.tools.map((t) => t.function.name).sort();
  assert.deepEqual(tools, ["check", "delete", "edit", "find", "grep", "ls", "pack", "read", "scaffold_plugin", "test", "typecheck", "write"]);
});
