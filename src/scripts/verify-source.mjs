import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import assert from "node:assert/strict";
const source = resolve(import.meta.dirname, "..");
const work = await mkdtemp(resolve(tmpdir(), "vestra-source-"));
function run(exe, args, extra = {}) {
  const r = spawnSync(exe, args, { encoding: "utf8", ...extra });
  if (r.status !== 0) throw Error(`${exe} ${r.status}\n${r.stderr}`);
  return r;
}
const media = resolve(work, "short.mkv");
run("ffmpeg", [
  "-v",
  "error",
  "-f",
  "lavfi",
  "-i",
  "testsrc2=size=320x180:rate=30",
  "-f",
  "lavfi",
  "-i",
  "sine=sample_rate=48000",
  "-t",
  "0.7",
  "-c:v",
  "libx264",
  "-preset",
  "ultrafast",
  "-c:a",
  "pcm_s16le",
  media,
]);
await writeFile(
  resolve(work, "manifest.json"),
  JSON.stringify({
    id: "test.source",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.ts",
    capabilities: ["playback.read", "playback.control", "events"],
  }),
);
let code = await readFile(resolve(source, "tests/source-session.ts"), "utf8");
code = code
  .replace('"../sdk/index.ts"', JSON.stringify(resolve(source, "sdk/index.ts")))
  .replace(
    '"../sdk/source.ts"',
    JSON.stringify(resolve(source, "sdk/source.ts")),
  );
await writeFile(resolve(work, "main.ts"), code);
const result = resolve(work, "result.json");
const r = run(
  resolve(source, "host/target/debug/vestra-host"),
  ["--headless", "--silent", "--seconds", "5", "--plugin", work],
  {
    env: {
      ...process.env,
      VESTRA_SOURCE_MEDIA: media,
      VESTRA_SOURCE_RESULT: result,
    },
    timeout: 10000,
  },
);
await writeFile(resolve(work, "host.stderr.log"), r.stderr);
assert.match(r.stderr, /SOURCE_INTEGRATION_PASS/);
const data = JSON.parse(await readFile(result, "utf8"));
assert.equal(data.pass, true);
console.log(JSON.stringify({ work, ...data }, null, 2));
