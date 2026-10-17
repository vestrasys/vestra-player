// 实验 GUI 场景：真实 Slint 回调与截图，不代表成品 API 或物理输入自动化。
import { execFile } from "node:child_process";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import assert from "node:assert/strict";
import { testRom } from "../tests/probes/nes/rom.ts";

const exec = promisify(execFile);
const root = fileURLToPath(new URL("../../", import.meta.url));
const directory = resolve(
  process.env.VESTRA_GUI_EVIDENCE ??
    resolve(root, ".cache/probe-evidence/combined-gui"),
);
await mkdir(directory, { recursive: true });
const media = resolve(directory, "sample.mp4");
const captions = resolve(directory, "sample.srt");
const rom = resolve(directory, "sample.nes");
const capture = resolve(directory, "captured.png");
await rm(capture, { force: true });
await exec(
  "ffmpeg",
  [
    "-v",
    "error",
    "-y",
    "-f",
    "lavfi",
    "-i",
    "testsrc2=size=640x360:rate=15:duration=60",
    "-f",
    "lavfi",
    "-i",
    "sine=frequency=440:sample_rate=48000:duration=60",
    "-c:v",
    "libx264",
    "-preset",
    "ultrafast",
    "-c:a",
    "aac",
    "-shortest",
    media,
  ],
  { timeout: 60_000 },
);
await writeFile(
  captions,
  "1\n00:00:00,000 --> 00:01:00,000\nNative Slint / real captions\n",
);
await writeFile(rom, testRom());
type Step = Record<string, unknown>;
const steps: Step[] = [];
const load = (plugin: string) =>
  steps.push({
    op: "load",
    manifest: resolve(root, "src/plugins", plugin, "manifest.json"),
  });
const wait = (plugin: string, name: string, value: unknown) =>
  steps.push({ op: "wait-property", plugin, name, value });
const menu = (plugin: string, command: string) =>
  steps.push({ op: "menu", plugin, command });
const delay = () => steps.push({ op: "delay", milliseconds: 350 });
const snapshot = (name: string) => steps.push({ op: "snapshot", name });
const dialogs = (...paths: string[]) => steps.push({ op: "dialogs", paths });

load("video-adjust");
wait("video-adjust", "status", "画面调节就绪");
steps.push({
  op: "request",
  plugin: "video-adjust",
  method: "player.load",
  params: { path: media },
});
steps.push({ op: "wait-frames", minimum: 2 });
menu("video-adjust", "brightness-up");
wait("video-adjust", "brightness", 0.05);
delay();
snapshot("video-adjust");
load("audio-workbench");
wait("audio-workbench", "status", "音频工作台就绪；未修改当前音频");
menu("audio-workbench", "preset-dialogue");
wait("audio-workbench", "voice", true);
delay();
snapshot("audio-workbench");
load("subtitle-workbench");
dialogs(captions);
menu("subtitle-workbench", "open");
wait("subtitle-workbench", "cue-count", 1);
delay();
snapshot("subtitle-workbench");
load("capture-export");
dialogs(capture);
menu("capture-export", "capture-processed");
wait("capture-export", "completed-count", 1);
wait("capture-export", "busy", false);
delay();
snapshot("capture-export");
// Existing panel switches must preserve state and must not compile another window.
steps.push({ op: "panel", plugin: "video-adjust" });
wait("video-adjust", "brightness", 0.05);
delay();
snapshot("video-return");
load("plugin-workshop");
wait(
  "plugin-workshop",
  "status",
  "Pi Agent 已就绪 · 本地 mock · 无 API key / 无网络",
);
menu("plugin-workshop", "generate");
wait("plugin-workshop", "phase", "installed");
wait("plugin-workshop", "busy", false);
steps.push({ op: "panel", plugin: "plugin-workshop" });
delay();
snapshot("plugin-workshop");
steps.push({ op: "panel", plugin: "workshop-generated-counter" });
wait("workshop-generated-counter", "count", 0);
steps.push({
  op: "action",
  plugin: "workshop-generated-counter",
  name: "increment",
  value: 0,
});
wait("workshop-generated-counter", "count", 1);
delay();
snapshot("generated-counter");
load("nes");
dialogs(rom);
menu("nes", "open-rom");
wait("nes", "loaded", true);
delay();
menu("nes", "toggle-pause");
wait("nes", "paused", true);
delay();
snapshot("nes-released");
steps.push({ op: "input", key: "z", pressed: true });
menu("nes", "toggle-pause");
wait("nes", "paused", false);
delay();
menu("nes", "toggle-pause");
wait("nes", "paused", true);
delay();
snapshot("nes-pressed");
steps.push({ op: "input", key: "z", pressed: false });
menu("nes", "close-rom");
wait("nes", "loaded", false);
delay();
snapshot("nes-closed");

const scenario = resolve(directory, "scenario.json");
await writeFile(
  scenario,
  JSON.stringify({ output: directory, steps }, null, 2),
);
const binary =
  process.env.VESTRA_HOST ??
  resolve(root, ".cache/probe-target/debug/vestra-slint-experimental1");
await exec(binary, ["--ui-scenario", scenario], {
  env: {
    ...process.env,
    SLINT_BACKEND: process.env.SLINT_BACKEND ?? "winit-software",
  },
  timeout: 120_000,
  maxBuffer: 8_000_000,
});
const report: unknown = JSON.parse(
  await readFile(resolve(directory, "report.json"), "utf8"),
);
assert.ok(
  report &&
    typeof report === "object" &&
    "passed" in report &&
    report.passed === true,
  "GUI scenario did not pass",
);
assert.equal(
  (await readFile(capture)).subarray(0, 8).toString("hex"),
  "89504e470d0a1a0a",
);
assert.ok("observations" in report && Array.isArray(report.observations));
const observations: unknown[] = report.observations;
const initial = observations[0];
assert.ok(initial && typeof initial === "object" && "playback" in initial);
const initialPlayback = initial.playback;
assert.ok(
  initialPlayback &&
    typeof initialPlayback === "object" &&
    "sourceKind" in initialPlayback,
);
assert.equal(
  initialPlayback.sourceKind,
  "none",
  "empty GUI host must not claim a file source",
);
function observation(name: string): Record<string, unknown> {
  const index = steps.findIndex(
    (step) => step.op === "snapshot" && step.name === name,
  );
  const value = observations.find(
    (entry) =>
      entry &&
      typeof entry === "object" &&
      "step" in entry &&
      entry.step === index,
  );
  assert.ok(value && typeof value === "object");
  return Object.fromEntries(Object.entries(value));
}
const before = observation("capture-export");
const after = observation("video-return");
assert.equal(
  after.windowGeneration,
  before.windowGeneration,
  "panel switch replaced native window",
);
assert.equal(
  after.compositionCount,
  before.compositionCount,
  "panel switch compiled Slint",
);
async function videoPixel(name: string): Promise<number[]> {
  const result = await exec(
    "ffmpeg",
    [
      "-v",
      "error",
      "-i",
      resolve(directory, `${name}.png`),
      "-vf",
      "format=rgb24,crop=1:1:iw/2:ih/4",
      "-frames:v",
      "1",
      "-f",
      "rawvideo",
      "-pix_fmt",
      "rgb24",
      "pipe:1",
    ],
    { encoding: "buffer" },
  );
  assert.equal(result.stdout.length, 3);
  return [...result.stdout];
}
const released = await videoPixel("nes-released");
const pressed = await videoPixel("nes-pressed");
assert.notDeepEqual(
  pressed,
  released,
  "NES input did not change native video pixels",
);
await writeFile(
  resolve(directory, "assertions.json"),
  JSON.stringify(
    {
      experimental: true,
      passed: true,
      panelSwitchRecompiled: false,
      nesReleasedRgb: released,
      nesPressedRgb: pressed,
      scope:
        "six probes plus Pi-generated plugin in one native window; callback input, not physical keyboard",
    },
    null,
    2,
  ),
);
console.log(`真实六探针及生成插件 GUI 场景与截图：${directory}`);
