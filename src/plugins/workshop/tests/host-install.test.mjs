// End to end with a real player: a scaffolded project, packed by the
// toolchain, validated and installed by a headless player exactly as
// delivery.ts does it (packages.validateArchive → install with source
// "workshop"), in a throwaway profile.
//
// Needs a built player: set VESTRA_HOST_EXE to vestra-host[.exe] (for example
// a staged release, C:\vestra\stage-…\VestraPlayer-…\vestra-host.exe).
// Skipped without it: the plugin test jobs have no player binary.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { Toolchain } from "../toolchain.ts";
import { dataDirectory } from "./fixture.mjs";

const plugin = path.resolve(import.meta.dirname, "..");
const exe = process.env.VESTRA_HOST_EXE;
const skip = !exe ? "set VESTRA_HOST_EXE to a vestra-host binary to run it" : !existsSync(exe) ? `VESTRA_HOST_EXE does not exist: ${exe}` : false;

/** A dev plugin (--plugin) that installs `file` and writes what happened to `out` as JSON lines. */
const HELPER = (file, out, id) => `import fs from "node:fs";
const say = (record) => fs.appendFileSync(${JSON.stringify(out)}, JSON.stringify(record) + "\\n");
export default async function activate(plugin) {
  setTimeout(async () => {
    try {
      const candidate = await plugin.packages.validateArchive(${JSON.stringify(file)});
      say({ step: "validated", id: candidate.manifest.id, checks: candidate.checks });
      const change = await candidate.install({ source: { kind: "workshop" } });
      say({ step: "installed", status: await change.done });
      for (let i = 0; i < 50; i++) {
        const entry = (await plugin.packages.list()).find((p) => p.id === ${JSON.stringify(id)});
        if (entry && entry.state !== "starting") return say({ step: "listed", state: entry.state, source: entry.source, error: entry.error });
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
      say({ step: "listed", state: "timeout" });
    } catch (error) {
      say({ step: "failed", code: error?.code, message: String(error?.message ?? error) });
    }
  }, 500);
}
`;

test("a scaffolded plugin, packed by the toolchain, installs into a real headless player and runs", { skip, timeout: 300_000 }, async () => {
  const data = await dataDirectory();
  const session = { dir: path.join(data, "sessions", "s-host"), work: path.join(data, "sessions", "s-host", "work") };
  await fs.mkdir(session.work, { recursive: true });
  const toolchain = new Toolchain(plugin, data);
  const id = "local.demo";
  for (const [step, result] of [
    ["scaffold", await toolchain.scaffold(session, { name: "demo", title: "演示" }, "")],
    ["typecheck", await toolchain.typecheck(session, id)],
    ["test", await toolchain.test(session, id)],
  ])
    assert.ok(result.ok, `${step}: ${result.text}`);
  const packed = await toolchain.pack(session, id, "");
  assert.ok(packed.ok && packed.file, packed.text);

  // A throwaway profile: nothing of the user's player is read or changed.
  const home = path.join(data, "profile");
  const helper = path.join(data, "helper");
  const out = path.join(data, "result.jsonl");
  await fs.mkdir(helper, { recursive: true });
  await fs.writeFile(
    path.join(helper, "manifest.json"),
    JSON.stringify({ id: "test.installer", version: "0.1.0", apiVersion: "vestra/1", entry: "main.js", name: "Installer" }),
  );
  await fs.writeFile(path.join(helper, "main.js"), HELPER(packed.file, out, id));
  const env = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    APPDATA: path.join(home, "Roaming"),
    LOCALAPPDATA: path.join(home, "Local"),
    TEMP: path.join(home, "Temp"),
    TMP: path.join(home, "Temp"),
    TMPDIR: path.join(home, "Temp"),
    VESTRA_PACKAGE_STORE: path.join(home, "packages"),
    VESTRA_PLUGIN_DATA: path.join(home, "plugin-data"),
  };
  for (const dir of [env.APPDATA, env.LOCALAPPDATA, env.TEMP]) await fs.mkdir(dir, { recursive: true });
  const seconds = process.platform === "win32" ? 45 : 20;
  const log = [];
  const status = await new Promise((resolve, reject) => {
    const child = spawn(exe, ["--headless", "--seconds", String(seconds), "--plugin", helper], { env, stdio: ["ignore", "pipe", "pipe"] });
    for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk) => log.push(String(chunk)));
    const timer = setTimeout(() => child.kill(), (seconds + 60) * 1000);
    child.on("error", reject);
    child.on("close", (code) => (clearTimeout(timer), resolve(code)));
  });
  const records = existsSync(out) ? (await fs.readFile(out, "utf8")).trim().split("\n").map((line) => JSON.parse(line)) : [];
  const where = `player exit ${status}; records ${JSON.stringify(records)}\n${log.join("").slice(-4000)}`;
  assert.deepEqual(
    records.map((r) => r.step),
    ["validated", "installed", "listed"],
    where,
  );
  assert.equal(records[0].id, id, where);
  assert.ok(records[0].checks.includes("slintCompile"), where);
  assert.equal(records[1].status, "committed", where);
  assert.equal(records[2].state, "running", where);
  assert.deepEqual(records[2].source, { kind: "workshop" }, where);
});
