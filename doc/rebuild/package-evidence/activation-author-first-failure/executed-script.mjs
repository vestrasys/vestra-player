/** Author activation lifecycle probe. Default prepares only; no mock runtime events. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, cp } from "node:fs/promises";
import { resolve } from "node:path";
import { tmpdir } from "node:os";

const work = await mkdtemp(resolve(tmpdir(), "vestra-activation-author-"));
const manager = resolve(work, "manager");
const native = process.argv.includes("--native");
const run = process.argv.includes("--run");
assert.ok(!(run && process.argv.includes("--prepare-only")), "choose --run or --prepare-only");
await mkdir(manager);
const command = (label) => `await host.commands.register("activation-proof",1,{title:${JSON.stringify(label)},path:["插件","Activation experiment",${JSON.stringify(label)}],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});`;
const candidates = {
  a: { capabilities: ["commands"], source: `export async function activate(host){${command("a")}await Promise.resolve();}` },
  b: { capabilities: ["commands"], source: `export async function activate(host){${command("b")}throw Error("intentional activation throw after command");}` },
  c: { capabilities: ["commands"], source: `export async function activate(host){${command("c")}await new Promise(()=>{});}` },
  d: { capabilities: ["commands", "ui.mount"], source: `export async function activate(host){${command("d")}void host.ui.mount({slot:"bottom",file:"invalid.slint",component:"Broken",global:"Bridge",properties:{}}).catch(()=>{});}`,
    files: { "invalid.slint": "export global Bridge {} export component Broken inherits Rectangle { this is deliberately invalid Slint }\n" } },
  e: { capabilities: ["commands", "ui.mount"], source: `export async function activate(host){${command("e")}await host.ui.mount({slot:"bottom",file:"valid.slint",component:"ActivationBar",global:"Bridge",properties:{count:1}});await Promise.resolve();}`,
    files: { "valid.slint": 'export global Bridge { in-out property <int> count: 0; } export component ActivationBar inherits Rectangle { min-height: 40px; background: #203040; Text { text: "Activation UI " + Bridge.count; color: white; } }\n' } },
  invalid: { capabilities: ["commands"], source: `export async function activate(host){${command("invalid-timeout")}await Promise.resolve();}` },
};
for (const [label, candidate] of Object.entries(candidates)) {
  const directory = resolve(work, label); await mkdir(directory);
  await writeFile(resolve(directory, "manifest.json"), JSON.stringify({ id: `author.activation.${label}`, apiVersion: "vestra.experimental/0.1", entry: "main.mjs", capabilities: candidate.capabilities }));
  await writeFile(resolve(directory, "main.mjs"), candidate.source + "\n");
  for (const [file, source] of Object.entries(candidate.files ?? {})) await writeFile(resolve(directory, file), source);
}

async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_ACTIVATION_SMOKE;
  const native = process.env.VESTRA_ACTIVATION_NATIVE === "1";
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const traces = [], checkpoints = [], accepted = [], loaded = new Set();
  let capabilities;
  function entries(snapshot) {
    const found = [];
    function visit(nodes) {
      for (const node of nodes) {
        if (node.kind === "group") visit(node.children);
        else if (node.kind === "command") found.push(node.entry);
        else assert.fail(`unknown public command node: ${JSON.stringify(node)}`);
      }
    }
    visit(snapshot.roots); return found;
  }
  async function observe(owner, label) {
    const state = await host.plugins.activation(owner);
    assert.equal(state.owner, owner);
    assert.ok(traces.length < 4096, "bounded activation trace exhausted");
    traces.push({ at: Date.now(), label, state });
    return state;
  }
  async function terminal(owner, label, expected, milliseconds = 5000) {
    const deadline = Date.now() + milliseconds;
    let last;
    do {
      last = await observe(owner, label);
      if (expected !== "ready") assert.notEqual(last.activation.status, "ready", `${label} must never be observed ready`);
      const finalRunning = expected === "ready";
      if (last.activation.status === expected && last.running === finalRunning) return last;
      assert.notEqual(last.activation.status, "unknown", `${label} activation receipt disappeared`);
      await wait(10);
    } while (Date.now() < deadline);
    throw Error(`${label} activation timeout: ${JSON.stringify(last)}`);
  }
  async function load(label, timeout) {
    // Only owner returned by this actual load is used to query activation. This
    // probe does not claim load is idempotent or recoverable before the response.
    const reply = await host.plugins.load(path.join(work, label), timeout);
    assert.equal(reply.id, `author.activation.${label}`);
    assert.equal(reply.accepted, true);
    assert.ok(Number.isSafeInteger(reply.owner) && reply.owner > 0);
    assert.deepEqual(reply.activation, { status: "pending", runtimeReady: false, pendingUi: 0 });
    accepted.push({ label, reply }); loaded.add(reply.id);
    return reply;
  }
  async function commandOwner(owner, exists, label) {
    const snapshot = await host.commands.snapshot();
    const owned = entries(snapshot).filter((entry) => entry.key.owner === owner);
    assert.equal(owned.some((entry) => entry.key.localId === "activation-proof"), exists, label);
    if (!exists) assert.equal(owned.length, 0, `${label} left owner commands`);
    checkpoints.push({ label, owner, snapshot });
    return snapshot;
  }
  async function receiptStays(owner, terminalState, label) {
    await wait(120);
    const again = await observe(owner, `${label} retained receipt`);
    assert.deepEqual(again, terminalState);
    return again;
  }
  try {
    capabilities = await host.capabilities();
    assert.equal(capabilities.ui, native, "run mode must match actual host UI availability");
    const beforeList = await host.plugins.list();
    const beforeCommands = await host.commands.snapshot();
    const invalid = [];
    for (const timeout of [999, 120001, -1, 1.5]) {
      await assert.rejects(() => host.plugins.load(path.join(work, "invalid"), timeout), (error) => {
        invalid.push({ timeout, code: error.code, message: String(error) });
        return error.code === "invalid";
      });
      assert.deepEqual(await host.plugins.list(), beforeList, "invalid timeout spawned a plugin");
      assert.deepEqual(await host.commands.snapshot(), beforeCommands, "invalid timeout registered commands");
    }
    checkpoints.push({ label: "wrong timeout rejected without public side effects", invalid, beforeList, beforeCommands });

    const a = await load("a"); // exercises the public default timeout shape
    const aReady = await terminal(a.owner, "a resolving activate", "ready");
    await commandOwner(a.owner, true, "a command ready");

    const b = await load("b", 5000);
    const bFailed = await terminal(b.owner, "b actual JS throw", "failed");
    assert.match(bFailed.activation.message, /intentional activation throw after command/);
    loaded.delete(b.id);
    await commandOwner(b.owner, false, "throw cleanup");
    await receiptStays(b.owner, bFailed, "b failed");
    await commandOwner(a.owner, true, "b failure keeps a alive");

    const c = await load("c", 1000);
    const cStartedAt = Date.now();
    let cPendingWithCommand;
    do {
      const state = await observe(c.owner, "c pending before timeout");
      assert.notEqual(state.activation.status, "ready");
      const snapshot = await host.commands.snapshot();
      if (state.activation.status === "pending" && entries(snapshot).some((entry) => entry.key.owner === c.owner)) {
        cPendingWithCommand = { state, snapshot }; break;
      }
      await wait(10);
    } while (Date.now() - cStartedAt < 1500);
    assert.ok(cPendingWithCommand, "never-resolving activate did not actually register its command while pending");
    const cTimedOut = await terminal(c.owner, "c never-resolving activate", "timedOut");
    loaded.delete(c.id);
    await commandOwner(c.owner, false, "timeout cleanup");
    await receiptStays(c.owner, cTimedOut, "c timedOut");
    await commandOwner(a.owner, true, "timeout keeps a alive");

    const d = await load("d", 5000);
    const dFailed = await terminal(d.owner, "d fire-and-forget UI cannot become ready", "failed");
    loaded.delete(d.id);
    if (!native) assert.match(dFailed.activation.message, /headless|unavailable/i);
    await commandOwner(d.owner, false, "failed initial UI cleanup");
    await receiptStays(d.owner, dFailed, "d initial UI failure");

    let e;
    if (native) {
      const reply = await load("e", 10000);
      const ready = await terminal(reply.owner, "e real native first UI preparation", "ready", 12000);
      await commandOwner(reply.owner, true, "native e ready");
      e = { reply, ready };
      await host.plugins.disable(reply.id); loaded.delete(reply.id);
      await commandOwner(reply.owner, false, "e disabled cleanup");
    }
    await commandOwner(a.owner, true, "all failure paths keep a command");
    await host.plugins.disable(a.id); loaded.delete(a.id);
    await commandOwner(a.owner, false, "a explicit disable cleanup");
    const after = await host.plugins.list();
    assert.deepEqual(after, beforeList);
    const finalCommands = await host.commands.snapshot();
    assert.deepEqual(entries(finalCommands), entries(beforeCommands));
    const result = { pass: true, native, capabilities, accepted, aReady, bFailed, cPendingWithCommand, cTimedOut, dFailed, e, traces, checkpoints, after, finalCommands,
      limits: ["Author activation lifecycle smoke, not package installation/trial/rollback or independent acceptance.", "load acceptance is not activation-ready; owner recovery here starts only after the load response.", native ? "Native branch exercises actual Slint preparation, not screen pixels/focus/continuous-playback acceptance." : "Headless initial ui.mount fails unavailable: proves lifecycle barrier rejection only, not a real Slint compiler diagnosis. Native e is prepared but not executed.", "Polling rejects any observed premature Ready; it is not an exhaustive event-history proof.", "running=false and public command removal are host lifecycle facts, not an acknowledged OS process-reap or all-resource cleanup fence."] };
    await fs.writeFile(path.join(work, "manager-result.json"), JSON.stringify(result, null, 2));
    console.error("ACTIVATION_SMOKE_DONE");
  } catch (error) {
    await fs.writeFile(path.join(work, "manager-failure.json"), JSON.stringify({ error: String(error), code: error.code, stack: error.stack, native, capabilities, accepted, traces, checkpoints }, null, 2));
    throw error;
  } finally {
    // Only public package lifecycle APIs; never read candidates' private state,
    // callback objects, files or a candidate-created side channel.
    const cleanup = [];
    for (const id of loaded) {
      try { await host.plugins.disable(id); cleanup.push({ id, disabled: true }); }
      catch (error) { cleanup.push({ id, code: error.code, error: String(error) }); }
    }
    await fs.writeFile(path.join(work, "manager-cleanup.json"), JSON.stringify(cleanup, null, 2));
  }
}

await writeFile(resolve(manager, "manifest.json"), JSON.stringify({ id: "author.activation.manager", apiVersion: "vestra.experimental/0.1", entry: "main.mjs", capabilities: ["plugins.manage", "commands"] }));
await writeFile(resolve(manager, "main.mjs"), `export const activate = ${activate.toString()};\n`);
const binary = process.env.VESTRA_HOST_BINARY ?? resolve(import.meta.dirname, "../host/target/debug/vestra-host");
const args = [...(native ? [] : ["--headless"]), "--silent", "--seconds", native ? "30" : "18", "--plugin", manager];
const env = { VESTRA_ACTIVATION_SMOKE: work, VESTRA_ACTIVATION_NATIVE: native ? "1" : "0" };
const nativeArgs = ["--silent", "--seconds", "30", "--plugin", manager];
let nativeLaunch;
if (native) {
  // Stage only this explicit native branch. Manager/candidates are already in
  // this run's own temporary directory; no GUI runtime code is read from Documents.
  const bundleWork = await mkdtemp(resolve("/tmp", "vestra-activation-native-"));
  const bundle = resolve(bundleWork, "Vestra Activation Probe.app");
  const contents = resolve(bundle, "Contents"), resources = resolve(contents, "Resources");
  const bundleId = `local.vestra.rebuild.activation-${randomUUID()}`;
  await mkdir(resolve(contents, "MacOS"), { recursive: true });
  await mkdir(resources, { recursive: true });
  await cp(binary, resolve(contents, "MacOS/host"));
  for (const name of ["sdk", "runtime"]) await cp(resolve(import.meta.dirname, "..", name), resolve(resources, name), { recursive: true });
  await cp(resolve(import.meta.dirname, "../host/ui"), resolve(resources, "ui"), { recursive: true });
  await writeFile(resolve(contents, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>host</string><key>CFBundleIdentifier</key><string>${bundleId}</string><key>CFBundleName</key><string>Vestra Activation Probe</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>`);
  const nativeEnv = { ...env, VESTRA_ACTIVATION_NATIVE: "1", PATH: "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin" };
  const openArgs = ["-n", bundle, ...Object.entries(nativeEnv).flatMap(([key, value]) => ["--env", `${key}=${value}`]),
    "--stdout", resolve(work, "host.stdout.log"), "--stderr", resolve(work, "host.stderr.log"), "--args", ...nativeArgs];
  nativeLaunch = { bundleWork, bundle, bundleId, binary: resolve(contents, "MacOS/host"), sourceBinary: binary,
    args: nativeArgs, env: nativeEnv, command: ["open", ...openArgs],
    note: "open success is launch acceptance only. Read manager-result.json/manager-cleanup.json and host logs after host exit before summarizing runtime outcomes." };
}
const launch = { work, preparedOnly: !run, mode: native ? "native" : "headless", binary: nativeLaunch?.binary ?? binary, args, env,
  nativeCandidate: { label: "e", directory: resolve(work, "e"), note: "prepared only unless a separate explicit --native --run is authorized; use a properly staged native bundle for CUA" },
  nativeLaunch,
  note: "No build or runtime starts by default. Candidates and manager communicate only through public host APIs." };
await writeFile(resolve(work, "launch.json"), JSON.stringify(launch, null, 2));
console.log(JSON.stringify(launch));
if (run && native) {
  const child = spawnSync("open", nativeLaunch.command.slice(1), { encoding: "utf8", timeout: 10000, maxBuffer: 1024 * 1024 });
  await writeFile(resolve(work, "open.stdout.log"), child.stdout ?? "");
  await writeFile(resolve(work, "open.stderr.log"), child.stderr ?? "");
  await writeFile(resolve(work, "command.json"), JSON.stringify({ command: nativeLaunch.command, bundle: nativeLaunch.bundle, bundleId: nativeLaunch.bundleId,
    openExitCode: child.status, signal: child.signal, error: String(child.error ?? ""), note: "open exits before the host. This is not an activation pass or a host-exit receipt." }, null, 2));
  assert.equal(child.status, 0, child.stderr || String(child.error));
  console.log(JSON.stringify({ work, bundle: nativeLaunch.bundle, bundleId: nativeLaunch.bundleId, openAccepted: true,
    note: "Runtime result pending: inspect manager-result.json, cleanup and host exit/logs separately." }));
} else if (run) {
  const child = spawnSync(binary, args, { encoding: "utf8", timeout: native ? 40000 : 25000, maxBuffer: 8 * 1024 * 1024, env: { ...process.env, ...env } });
  await writeFile(resolve(work, "host.stdout.log"), child.stdout ?? "");
  await writeFile(resolve(work, "host.stderr.log"), child.stderr ?? "");
  await writeFile(resolve(work, "command.json"), JSON.stringify({ binary, args, env, exitCode: child.status, signal: child.signal, error: String(child.error ?? "") }, null, 2));
  console.log(JSON.stringify({ work, exitCode: child.status, signal: child.signal }));
  assert.equal(child.status, 0, child.stderr || String(child.error));
  assert.match(child.stderr, /ACTIVATION_SMOKE_DONE/);
  const result = JSON.parse(await readFile(resolve(work, "manager-result.json"), "utf8"));
  assert.equal(result.pass, true);
  await writeFile(resolve(work, "result.json"), JSON.stringify({ ...result, work }, null, 2));
  console.log(JSON.stringify({ pass: true, work, native, limits: result.limits }));
}
