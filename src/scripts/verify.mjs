import { spawnSync } from "node:child_process";
import { mkdtemp, writeFile, readFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const source = resolve(import.meta.dirname, "..");
const work = await mkdtemp(resolve(tmpdir(), "vestra-stage-a-"));
const media = resolve(work, "clock.mp4");
function command(executable, args, options = {}) {
  const result = spawnSync(executable, args, { encoding: "utf8", ...options });
  if (result.status !== 0)
    throw new Error(
      `${executable} exited ${result.status}\n${result.stdout}\n${result.stderr}`,
    );
  return result;
}
command("ffmpeg", [
  "-v",
  "error",
  "-f",
  "lavfi",
  "-i",
  "testsrc2=size=640x360:rate=30",
  "-f",
  "lavfi",
  "-i",
  "sine=frequency=440:sample_rate=48000",
  "-t",
  "24",
  "-c:v",
  "libx264",
  "-preset",
  "ultrafast",
  "-c:a",
  "aac",
  media,
]);
const fixture = resolve(work, "fixture"),
  helper = resolve(work, "helper"),
  stalled = resolve(work, "stalled");
await mkdir(fixture);
await mkdir(helper);
await mkdir(stalled);
const manifest = (id, capabilities) => ({
  id,
  apiVersion: "vestra.experimental/0.1",
  entry: "main.ts",
  capabilities,
});
await writeFile(
  resolve(fixture, "manifest.json"),
  JSON.stringify(
    manifest("test.session", [
      "playback.read",
      "playback.control",
      "events",
      "plugins.manage",
    ]),
  ),
);
// Standalone package imports the SDK only; this test's source is copied into its package.
let fixtureCode = await readFile(resolve(source, "tests/session.ts"), "utf8");
fixtureCode = fixtureCode.replace(
  '"../sdk/index.ts"',
  JSON.stringify(resolve(source, "sdk/index.ts")),
);
await writeFile(resolve(fixture, "main.ts"), fixtureCode);
await writeFile(
  resolve(helper, "manifest.json"),
  JSON.stringify(manifest("test.helper", [])),
);
await writeFile(
  resolve(helper, "main.ts"),
  'export function activate() { console.error("HELPER_STARTED"); return () => console.error("HELPER_DISPOSED"); }\n',
);
await writeFile(
  resolve(stalled, "manifest.json"),
  JSON.stringify(manifest("test.stalled", ["playback.read"])),
);
await writeFile(
  resolve(stalled, "main.ts"),
  `import {writeFileSync} from "node:fs";
export async function activate() {
  writeFileSync(process.env.VESTRA_STALLED_PID,String(process.pid));
  process.stdin.pause();
  setInterval(() => {}, 1000); // Ensure forced teardown, rather than Node exiting an unsettled await.
  for (let i=1;i<=6000;i++) process.stdout.write(JSON.stringify({id:i,method:"playback.state",params:{}})+"\\n");
  await new Promise(()=>{});
}`,
);
const report = resolve(work, "result.json");
const result = command(
  resolve(source, "host/target/debug/vestra-host"),
  ["--headless", "--silent", "--seconds", "16", "--plugin", fixture],
  {
    env: {
      ...process.env,
      VESTRA_TEST_MEDIA: media,
      VESTRA_TEST_HELPER: helper,
      VESTRA_TEST_RESULT: report,
      VESTRA_TEST_STALLED: stalled,
      VESTRA_STALLED_PID: resolve(work, "stalled.pid"),
    },
    timeout: 25000,
  },
);
assert.match(result.stderr, /HELPER_STARTED/);
assert.match(result.stderr, /HELPER_DISPOSED/);
assert.match(result.stderr, /SESSION_INTEGRATION_PASS/);
const evidence = JSON.parse(await readFile(report, "utf8"));
assert.equal(evidence.pass, true);
const stalledPid = Number(await readFile(resolve(work, "stalled.pid"), "utf8"));
assert.throws(
  () => process.kill(stalledPid, 0),
  (error) => error.code === "ESRCH",
  "stalled child must be killed/reaped",
);
await writeFile(resolve(work, "host.stderr.log"), result.stderr);
console.log(
  JSON.stringify({ work, stalledReaped: true, ...evidence }, null, 2),
);
