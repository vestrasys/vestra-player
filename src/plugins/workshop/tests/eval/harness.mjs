// One eval task: a fresh session folder, the real agent worker and the real
// plugin tools (no player: install_plugin is not offered),
// one prompt, then an independent typecheck, test and check of every project
// the agent made. Used by run.mjs (a real model) and by tests/eval.test.mjs
// (the mock server).
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { AgentWorker } from "../../agent.ts";
import { systemPrompt } from "../../prompt.ts";
import { Toolchain } from "../../toolchain.ts";
import { BUILD_TOOLS, runTool } from "../../toolhost.ts";

const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const VERIFY = ["typecheck", "test", "check"];

/**
 * `model`: { provider, modelId, baseUrl, api, key }, as the plugin passes them
 * to the worker (custom endpoints: provider custom-openai / custom-anthropic,
 * baseUrl and api set). Resolves with what happened; never throws for a
 * failed task.
 */
export async function runTask(task, model, { timeoutMs = 15 * 60_000, owner = "eval", keep = false, log = () => {} } = {}) {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), `vestra-ai-eval-${task.id}-`));
  const session = { dir: path.join(data, "sessions", "s-eval"), work: path.join(data, "sessions", "s-eval", "work") };
  await fs.mkdir(session.work, { recursive: true });
  const toolchain = new Toolchain(plugin, data);
  const calls = []; // { name, id, ok, text }
  const started = Date.now();
  let error = null;
  let reply = "";
  let worker;
  try {
    await new Promise((resolve) => {
      let finished = false;
      const done = () => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(() => {
        error = `timeout after ${timeoutMs / 1000} s`;
        worker.abort();
        setTimeout(done, 2000);
      }, timeoutMs);
      worker = new AgentWorker(
        {
          sessionDir: session.dir,
          workDir: session.work,
          sessionFile: null,
          ...model,
          systemPrompt: systemPrompt({ sdk: toolchain.sdk, sdkVersion: toolchain.sdkVersion, owner, delivery: false }),
          docsRoots: toolchain.docsRoots(),
          pluginTools: BUILD_TOOLS,
        },
        {
          event: (event) => {
            if (event.kind === "assistant_end") {
              reply = event.text || reply;
              if (event.error) error = event.error;
            }
            if (event.kind === "tool_start") log(`  ${event.name} ${event.target}`);
            if (event.kind === "settled") done();
          },
          file: () => {},
          failed: (message) => {
            error = message;
            done();
          },
          exited: (code) => {
            if (!finished && !error) error = `worker exited (${code}): ${worker.lastError()}`;
            done();
          },
          tool: (call, name, args, signal) => {
            runTool(toolchain, session, owner, name, args, signal).then((result) => {
              calls.push({ name, id: String(args?.id ?? args?.name ?? ""), ok: result.ok, text: result.text });
              if (!result.ok) log(`    ✗ ${result.text.split("\n")[0]}`);
              worker.reply(call, result);
            });
          },
        },
      );
      worker.prompt(task.prompt).catch(() => {}); // failures arrive through `failed`
    });
  } finally {
    await worker?.stop().catch(() => {});
  }
  // Independent verification of what is on disk.
  const projects = await toolchain.projects(session);
  const verified = [];
  for (const project of projects) {
    const results = {};
    for (const name of VERIFY) results[name] = (await runTool(toolchain, session, owner, name, { id: project.id }, new AbortController().signal)).ok;
    verified.push({ id: project.id, ...results, pass: VERIFY.every((name) => results[name]) });
  }
  const failedChecks = calls.filter((c) => VERIFY.includes(c.name) && !c.ok).length;
  const result = {
    task: task.id,
    pass: !error && verified.length > 0 && verified.every((v) => v.pass),
    /** Failed typecheck/test/check calls the agent fixed its way through. */
    fixRounds: failedChecks,
    toolCalls: calls.length,
    projects: verified,
    seconds: Math.round((Date.now() - started) / 1000),
    error,
    reply: reply.slice(0, 400),
    folder: keep ? session.work : null,
  };
  if (!keep) await fs.rm(data, { recursive: true, force: true });
  return result;
}
