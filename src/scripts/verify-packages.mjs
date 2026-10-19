/** Finite real JS package lifecycle author check; no fabricated runtime events. */
import { mkdtemp, mkdir, writeFile, readFile, cp } from "node:fs/promises";
import { resolve } from "node:path";
import { tmpdir } from "node:os";
import { spawn } from "node:child_process";
const root = await mkdtemp(resolve(tmpdir(), "vestra-packages-"));
const native = process.argv.includes("--native");
for (const [name, version, step, bad] of [
  ["one", "1.0.0", 1, false],
  ["two", "1.1.0", 3, false],
  ["bad", "1.2.0", 4, true],
]) {
  const dir = resolve(root, name);
  await mkdir(dir);
  await writeFile(
    resolve(dir, "manifest.json"),
    JSON.stringify({
      id: "community.counter",
      apiVersion: "vestra.experimental/0.1",
      version,
      entry: "main.mjs",
      capabilities: ["commands", ...(native ? ["ui.window"] : [])],
    }),
  );
  await writeFile(
    resolve(dir, "counter.slint"),
    'import { Button, VerticalBox } from "std-widgets.slint"; export component Counter inherits Window { title: "Package counter"; width: 260px; height: 160px; in-out property <int> count: 0; callback increase(); VerticalBox { Text { text: "Count " + root.count; } Button { text: "Increase"; clicked => { root.increase(); } } } }',
  );
  await writeFile(
    resolve(dir, "main.mjs"),
    `export async function activate(host){let count=0;const increase=()=>{count+=${step};${native ? 'void host.ui.set("counter","count",count);' : ""}};await host.commands.register("increase",1,{title:"Increase ${version}",path:["插件","Package counter","Increase"],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});host.commands.onExecute(increase);${native ? 'await host.ui.open("counter","counter.slint",{count},["increase"]);host.ui.onCallback(increase);' : ""}${bad ? 'throw Error("intentional bad activation");' : ""}}`,
  );
}
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    assert = (await import("node:assert/strict")).default;
  const root = process.env.VESTRA_PACKAGE_PROOF;
  const trace = [];
  const pause = () => new Promise((r) => setTimeout(r, 20));
  async function until(fn, predicate) {
    const end = Date.now() + 30000;
    let v;
    do {
      v = await fn();
      trace.push(v);
      if (predicate(v)) return v;
      await pause();
    } while (Date.now() < end);
    throw Error("deadline " + JSON.stringify(v));
  }
  const terminal = (s) =>
    ["committed", "failed", "cancelled", "failedDegraded"].includes(s);
  async function operation(v) {
    return until(
      () => host.packages.operation(v.operationId),
      (r) => terminal(r.status),
    );
  }
  try {
    const a = await host.packages.validate(path.join(root, "one")),
      b = await host.packages.validate(path.join(root, "two")),
      bad = await host.packages.validate(path.join(root, "bad"));
    trace.push(a, b, bad);
    const trial = await host.packages.trial(b.token);
    await until(
      () => host.packages.trialState(trial.trialId),
      (v) => v.status === "ready",
    );
    await host.packages.stopTrial(trial.trialId);
    await until(
      () => host.packages.trialState(trial.trialId),
      (v) => v.status === "stopped" && v.cleanupConfirmed,
    );
    const first = await operation(
      await host.packages.install(a.token, 0, "install-one"),
    );
    assert.equal(first.status, "committed");
    assert.equal(first.installed.current.manifest.version, "1.0.0");
    const failed = await operation(
      await host.packages.install(bad.token, 1, "bad-upgrade"),
    );
    assert.equal(failed.status, "failed");
    assert.equal(failed.restored, true);
    assert.equal(
      (await host.packages.installed("community.counter")).revision,
      1,
    );
    const second = await operation(
      await host.packages.install(b.token, 1, "install-two"),
    );
    assert.equal(second.status, "committed");
    assert.equal(second.installed.current.manifest.version, "1.1.0");
    const old = await operation(
      await host.packages.rollback("community.counter", 2, "back-one"),
    );
    assert.equal(old.status, "committed");
    assert.equal(old.installed.current.manifest.version, "1.0.0");
    assert.equal(
      (await host.packages.install(a.token, 0, "install-one")).operationId,
      "install-one",
    );
    const late = await host.packages.cancel("back-one");
    assert.equal(late.status, "tooLate");
    const cancel = await host.packages.install(b.token, 3, "cancel-upgrade");
    await host.packages.cancel(cancel.operationId);
    const cancelled = await operation(cancel);
    assert.equal(cancelled.status, "cancelled");
    assert.equal(
      (await host.packages.installed("community.counter")).revision,
      3,
    );
    await fs.writeFile(
      path.join(root, "result.json"),
      JSON.stringify(
        {
          passed: true,
          trace,
          limits: [
            "Author protocol lifecycle proof, not visible button/menu interaction.",
            "Production JS and real Slint compiler; headless run does not show native windows.",
            "Node candidates are trusted, not sandboxed.",
          ],
        },
        null,
        2,
      ),
    );
  } catch (e) {
    await fs.writeFile(
      path.join(root, "result.json"),
      JSON.stringify(
        { passed: false, error: String(e), stack: e.stack, trace },
        null,
        2,
      ),
    );
  }
}
const manager = resolve(root, "manager");
await mkdir(manager);
await writeFile(
  resolve(manager, "manifest.json"),
  JSON.stringify({
    id: "package.proof.manager",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: ["plugins.manage"],
  }),
);
await writeFile(resolve(manager, "main.mjs"), "export " + activate.toString());
console.log(JSON.stringify({ root, native }));
let binary = resolve("src/host/target/debug/vestra-host");
if (native) {
  const bundle = resolve(root, "Package Proof.app/Contents");
  await mkdir(resolve(bundle, "MacOS"), { recursive: true });
  await mkdir(resolve(bundle, "Resources"), { recursive: true });
  await cp(binary, resolve(bundle, "MacOS/host"));
  binary = resolve(bundle, "MacOS/host");
  for (const name of ["sdk", "runtime"])
    await cp(resolve("src", name), resolve(bundle, "Resources", name), {
      recursive: true,
    });
  await cp(resolve("src/host/ui"), resolve(bundle, "Resources/ui"), {
    recursive: true,
  });
  await writeFile(
    resolve(bundle, "Info.plist"),
    '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>host</string><key>CFBundleIdentifier</key><string>local.vestra.package-proof</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>',
  );
}
await writeFile(
  resolve(root, "command.json"),
  JSON.stringify(
    {
      binary,
      args: [
        ...(native ? [] : ["--headless"]),
        "--silent",
        "--seconds",
        "75",
        "--plugin",
        manager,
      ],
      env: {
        VESTRA_PACKAGE_STORE: resolve(root, "store"),
        VESTRA_PACKAGE_PROOF: root,
      },
    },
    null,
    2,
  ),
);
const host = spawn(
  binary,
  [
    ...(native ? [] : ["--headless"]),
    "--silent",
    "--seconds",
    "75",
    "--plugin",
    manager,
  ],
  {
    env: {
      ...process.env,
      VESTRA_PACKAGE_STORE: resolve(root, "store"),
      VESTRA_PACKAGE_PROOF: root,
    },
    stdio: ["ignore", "ignore", "pipe"],
  },
);
let stderr = "";
host.stderr.on("data", (b) => {
  stderr += b;
});
let finished = false;
const closed = new Promise((resolve) =>
  host.once("close", (code, signal) => resolve({ code, signal })),
);
host.once("error", (e) => {
  stderr += String(e);
});
for (let i = 0; i < 1800; i++) {
  try {
    const result = JSON.parse(
      await readFile(resolve(root, "result.json"), "utf8"),
    );
    console.log(JSON.stringify({ root, ...result }));
    finished = true;
    host.kill("SIGTERM");
    const exit = await closed;
    await writeFile(resolve(root, "stderr.log"), stderr);
    console.log(JSON.stringify({ exit }));
    process.exitCode = result.passed ? 0 : 1;
    break;
  } catch {}
  await new Promise((r) => setTimeout(r, 50));
}

if (!finished) {
  host.kill("SIGTERM");
  const exit = await closed;
  await writeFile(resolve(root, "stderr.log"), stderr);
  console.error(
    JSON.stringify({ error: "no terminal result before deadline", root, exit }),
  );
  process.exitCode = 1;
}
