// Run from an isolated local npm project; imports resolve from this file's copy there.
import { Agent } from '@earendil-works/pi-agent-core';
import { createAssistantMessageEventStream } from '@earendil-works/pi-ai';
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const evidence = path.resolve(process.argv[2]);
const candidate = path.join(evidence, 'candidate');
await fs.mkdir(candidate, { recursive: true });
assert.equal((await fs.lstat(candidate)).isSymbolicLink(), false);
const root = await fs.realpath(candidate);
const controller = new AbortController();
let agent, actions = 0, repairs = 0, revision = 0, turn = 0, child;
const events = [], validations = [], requests = [];
const started = Date.now();
const timer = setTimeout(() => { controller.abort(new Error('120sec deadline')); agent?.abort(); child?.kill('SIGKILL'); }, 120000);
const digest = text => createHash('sha256').update(text).digest('hex');
async function safeFile(name) {
  assert.equal(name, 'counter.ts', 'only counter.ts is allowed (no traversal or absolute path)');
  assert.equal(await fs.realpath(candidate), root, 'candidate root changed');
  assert.equal((await fs.lstat(candidate)).isSymbolicLink(), false);
  const file = path.resolve(root, name);
  assert.equal(path.dirname(file), root);
  try { const stat = await fs.lstat(file); assert(!stat.isSymbolicLink() && stat.isFile(), 'symlink/non-file rejected'); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  return file;
}
function check(signal) { controller.signal.throwIfAborted(); signal?.throwIfAborted(); }
const result = value => ({ content: [{ type: 'text', text: JSON.stringify(value) }], details: {} });
const string = { type: 'string' };
const schema = properties => ({ type: 'object', properties, required: Object.keys(properties), additionalProperties: false });
async function compiler(signal) {
  check(signal);
  const args = [require.resolve('typescript/lib/tsc.js'), '--strict', '--target', 'ES2022', '--module', 'ES2022', '--types', 'node', '--typeRoots', path.join(root, 'no-types'), '--noEmitOnError', '--outDir', path.join(root, 'compiled'), path.join(root, 'counter.ts')];
  // No @types needed; remove types selection rather than discover unrelated project ambient types.
  args.splice(args.indexOf('--types'), 2);
  let stdout = '', stderr = '';
  const proc = spawn(process.execPath, args, { cwd: root, env: {}, stdio: ['ignore', 'pipe', 'pipe'] }); child = proc;
  const kill = () => proc.kill('SIGKILL');
  signal?.addEventListener('abort', kill, { once: true }); controller.signal.addEventListener('abort', kill, { once: true });
  const localTimer = setTimeout(kill, 20000);
  proc.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1000000) kill(); });
  proc.stderr.on('data', chunk => { stderr += chunk; if (stderr.length > 1000000) kill(); });
  try {
    const status = await new Promise((resolve, reject) => { proc.once('error', reject); proc.once('close', (code, signal) => resolve({ code, signal })); });
    const source = await fs.readFile(await safeFile('counter.ts'), 'utf8');
    const report = { revision, digest: digest(source), command: [process.execPath, ...args], ...status, stdout, stderr };
    validations.push(report); check(signal); return report;
  } finally { clearTimeout(localTimer); signal?.removeEventListener('abort', kill); controller.signal.removeEventListener('abort', kill); child = undefined; }
}
const tools = [
 { name: 'write_candidate', label: 'Write candidate', description: 'Write only this task counter.ts', parameters: schema({ path: string, text: string }), async execute(id, args, signal) { check(signal); assert(args.text.length < 65536); const file = await safeFile(args.path); await fs.writeFile(file, args.text, { flag: 'w' }); revision++; return result({ revision, digest: digest(args.text) }); } },
 { name: 'read_candidate', label: 'Read candidate', description: 'Read only this task counter.ts', parameters: schema({ path: string }), async execute(id, args, signal) { check(signal); const text = await fs.readFile(await safeFile(args.path), 'utf8'); return result({ revision, text, digest: digest(text) }); } },
 { name: 'validate_candidate', label: 'Validate candidate', description: 'Execute pinned local TypeScript compiler', parameters: schema({ revision: { type: 'integer' } }), async execute(id, args, signal) { check(signal); assert.equal(args.revision, revision, 'stale revision'); await safeFile('counter.ts'); const report = await compiler(signal); return { ...result(report), isError: report.code !== 0 }; } },
];
const source = bad => `// Standalone counter prerequisite, not an installable plugin.\nconst step: number = ${bad ? '"1"' : '1'};\nlet count: number = 0;\nexport function increment(): number { count += step; return count; }\nexport function reset(): number { count = 0; return count; }\n`;
const model = { id: 'deterministic-local-mock', name: 'Local mock; no provider access', api: 'openai-completions', provider: 'local-mock', baseUrl: 'http://invalid.localhost', reasoning: false, input: ['text'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 32768, maxTokens: 4096 };
const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
function streamFn(model, context, options) {
  const stream = createAssistantMessageEventStream();
  const message = { role: 'assistant', api: model.api, provider: model.provider, model: model.id, usage: structuredClone(usage), content: [], stopReason: 'stop', timestamp: Date.now() };
  try {
    check(options?.signal);
    turn++; assert(turn <= 3, 'model turn limit');
    assert(context.messages.some(m => m.role === 'system' && m.toolsAdded?.length === 3), 'tools must arrive in system transcript');
    requests.push(structuredClone({ turn, context }));
    const reports = context.messages.filter(m => m.role === 'toolResult' && m.toolName === 'validate_candidate');
    const reads = context.messages.filter(m => m.role === 'toolResult' && m.toolName === 'read_candidate');
    const calls = (bad, rev) => [
      { type: 'toolCall', id: `write-${turn}`, name: 'write_candidate', arguments: { path: 'counter.ts', text: source(bad) } },
      { type: 'toolCall', id: `read-${turn}`, name: 'read_candidate', arguments: { path: 'counter.ts' } },
      { type: 'toolCall', id: `validate-${turn}`, name: 'validate_candidate', arguments: { revision: rev } },
    ];
    if (turn === 1) { assert.equal(reports.length, 0); message.content = calls(true, 1); }
    else {
      const standardContent = reports.at(-1)?.content;
      assert(standardContent?.every(c => c.type === 'text'));
      const report = JSON.parse(standardContent.map(c => c.text).join(''));
      const observed = JSON.parse(reads.at(-1).content[0].text);
      assert.equal(observed.digest, report.digest, 'read and validator must see same source');
      if (turn === 2) {
        assert.equal(report.code, 1); assert.equal(report.signal, null);
        assert(/counter\.ts\(2,7\): error TS2322:/.test(report.stdout), 'real expected file/line/column/code required');
        assert(observed.text.includes('const step: number = "1"'));
        repairs++; assert(repairs <= 2); message.content = calls(false, report.revision + 1);
      } else { assert.equal(report.code, 0); assert.equal(report.stdout, ''); assert.equal(report.stderr, ''); assert.equal(report.signal, null); message.content = [{ type: 'text', text: 'Standalone counter passed real TypeScript compilation after diagnostic-driven repair.' }]; }
    }
    message.stopReason = message.content.some(c => c.type === 'toolCall') ? 'toolUse' : 'stop';
    stream.push({ type: 'start', partial: message });
    for (const [contentIndex, block] of message.content.entries()) {
      if (block.type === 'toolCall') stream.push({ type: 'toolcall_end', contentIndex, toolCall: block, partial: message });
      else stream.push({ type: 'text_end', contentIndex, content: block.text, partial: message });
    }
    stream.push({ type: 'done', reason: message.stopReason, message });
  } catch (error) { message.stopReason = controller.signal.aborted ? 'aborted' : 'error'; message.errorMessage = String(error.stack); stream.push({ type: 'error', reason: message.stopReason, error: message }); }
  return stream;
}
agent = new Agent({ initialState: { systemPrompt: 'Generate a standalone typed counter, observe real tool diagnostics, repair only after TS2322, then stop. No plugin APIs.', model, tools }, streamFn, toolExecution: 'sequential', getApiKey: () => undefined,
 beforeToolCall: async (_, signal) => { check(signal); actions++; if (actions > 12) { controller.abort(); agent.abort(); return { block: true, reason: '12 action limit', terminate: true }; } },
});
const unsubscribe = agent.subscribe(event => { events.push(structuredClone(event)); });
let outcome;
try {
  await agent.prompt('Create the finite standalone counter prerequisite.'); await agent.waitForIdle();
  const last = agent.state.messages.at(-1);
  assert.equal(last.role, 'assistant'); assert.equal(last.stopReason, 'stop'); assert(!last.errorMessage && !agent.state.errorMessage);
  assert.equal(agent.state.isStreaming, false); assert.equal(agent.state.pendingToolCalls.size, 0);
  assert.equal(actions, 6); assert.equal(repairs, 1); assert.equal(validations.length, 2);
  assert.equal(events.filter(e => e.type === 'tool_execution_start').length, 6);
  assert.equal(events.filter(e => e.type === 'tool_execution_end').length, 6);
  const emittedResults = events.filter(e => e.type === 'message_end' && e.message.role === 'toolResult'); assert.equal(emittedResults.length, 6);
  assert.equal(events.at(-1).type, 'agent_end');
  const finalSource = await fs.readFile(await safeFile('counter.ts'), 'utf8');
  const emitted = await fs.readFile(path.join(root, 'compiled/counter.js'), 'utf8'); assert(emitted.includes('const step = 1;'));
  assert.equal(digest(finalSource), validations[1].digest);
  const module = await import('data:text/javascript,' + encodeURIComponent(emitted)); assert.deepEqual([module.increment(), module.increment(), module.reset()], [1, 2, 0]);
  outcome = { passed: true, actions, repairs, modelTurns: turn, durationMs: Date.now() - started, finalStopReason: last.stopReason, sourceSha256: digest(finalSource), emittedSha256: digest(emitted), counterBehavior: [1, 2, 0], pendingChild: Boolean(child), scope: 'real Pi loop + standalone TS only; not product/plugin/Slint/trial/install' };
} catch (error) { outcome = { passed: false, error: String(error.stack), actions, repairs, modelTurns: turn, durationMs: Date.now() - started }; process.exitCode = 1; }
finally { clearTimeout(timer); controller.abort(); agent.abort(); await agent.waitForIdle(); unsubscribe(); }
await fs.writeFile(path.join(evidence, 'events.json'), JSON.stringify(events, null, 2));
await fs.writeFile(path.join(evidence, 'model-requests.json'), JSON.stringify(requests, null, 2));
await fs.writeFile(path.join(evidence, 'validations.json'), JSON.stringify(validations, null, 2));
await fs.writeFile(path.join(evidence, 'result.json'), JSON.stringify(outcome, null, 2));
console.log(JSON.stringify(outcome, null, 2));
