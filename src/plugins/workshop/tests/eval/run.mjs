#!/usr/bin/env node
// Vestra Player AI eval: the fixed tasks of tasks.json against a REAL model.
// Opt-in and manual (it spends the provider account's money); run it after a
// change to the system prompt, the tools or the reference, and record the
// result in the round's notes. Never part of `vestra-plugin test`.
//
//   VESTRA_EVAL_PROVIDER=anthropic VESTRA_EVAL_MODEL=claude-sonnet-4-5 \
//   VESTRA_EVAL_API_KEY=… npm run eval -- [--task <id>…] [--json <file>] [--keep] [--timeout <minutes>]
//
//   VESTRA_EVAL_PROVIDER   a provider id of pi's catalog (vendor/pi/models.json),
//                          or custom-openai / custom-anthropic with VESTRA_EVAL_BASE_URL
//   VESTRA_EVAL_MODEL      the model id
//   VESTRA_EVAL_API_KEY    the key; read from the environment only, never written anywhere
//
// Per task: pass (every project the agent made passes typecheck, test and
// check, verified again after the run) and fix rounds (failed typecheck /
// test / check calls on the way). Installing needs the player and is
// not part of the eval.
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runTask } from "./harness.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const values = (flag) => args.flatMap((a, i) => (a === flag && args[i + 1] ? [args[i + 1]] : []));
const provider = process.env.VESTRA_EVAL_PROVIDER;
const modelId = process.env.VESTRA_EVAL_MODEL;
const key = process.env.VESTRA_EVAL_API_KEY ?? "";
const baseUrl = process.env.VESTRA_EVAL_BASE_URL ?? null;
if (!provider || !modelId) {
  console.error("Set VESTRA_EVAL_PROVIDER, VESTRA_EVAL_MODEL and VESTRA_EVAL_API_KEY (see the header of tests/eval/run.mjs).");
  process.exit(2);
}
const api = provider === "custom-openai" ? "openai-completions" : provider === "custom-anthropic" ? "anthropic-messages" : null;
if (api && !baseUrl) {
  console.error(`${provider} needs VESTRA_EVAL_BASE_URL`);
  process.exit(2);
}
const { tasks } = JSON.parse(await fs.readFile(path.join(here, "tasks.json"), "utf8"));
const wanted = values("--task");
const chosen = wanted.length ? tasks.filter((t) => wanted.includes(t.id)) : tasks;
const timeoutMs = Number(values("--timeout")[0] ?? 15) * 60_000;

const results = [];
for (const task of chosen) {
  console.log(`▶ ${task.id}`);
  const result = await runTask(task, { provider, modelId, baseUrl: api ? baseUrl : null, api, key }, { timeoutMs, keep: args.includes("--keep"), log: (line) => console.log(line) });
  results.push(result);
  console.log(`${result.pass ? "✓ pass" : "✗ fail"}  fix rounds ${result.fixRounds}, ${result.toolCalls} tool calls, ${result.seconds} s${result.error ? `, error: ${result.error}` : ""}${result.folder ? `\n  kept: ${result.folder}` : ""}`);
}

console.log(`\nmodel ${provider}/${modelId}`);
console.log("task                 pass  fix rounds  tool calls  seconds");
for (const r of results) console.log(`${r.task.padEnd(20)} ${(r.pass ? "yes" : "no").padEnd(5)} ${String(r.fixRounds).padEnd(11)} ${String(r.toolCalls).padEnd(11)} ${r.seconds}`);
const passed = results.filter((r) => r.pass).length;
console.log(`${passed}/${results.length} passed; first-try passes ${results.filter((r) => r.pass && r.fixRounds === 0).length}`);
const out = values("--json")[0];
if (out) await fs.writeFile(out, `${JSON.stringify({ provider, modelId, date: new Date().toISOString(), results }, null, 2)}\n`);
process.exitCode = passed === results.length ? 0 : 1;
