// 实验功能审计：真实 Rust/Node/FFmpeg；路径 fixture 仅证明 F/B 层，不代替原生 UI 验收。
import assert from "node:assert/strict";
import test from "node:test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  mkdir,
  readFile,
  writeFile,
  copyFile,
  readdir,
} from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../support/host.ts";
import { testRom } from "../probes/nes/rom.ts";

const directory = fileURLToPath(
  new URL("../../../.cache/evidence/functional-audit/", import.meta.url),
);
const exec = promisify(execFile);
const names = [
  "file-playback",
  "gaussian-blur",
  "playlist",
  "video-adjust",
  "audio-workbench",
  "subtitle-workbench",
  "capture-export",
  "nes",
  "plugin-workshop",
];
async function prepare() {
  await mkdir(directory, { recursive: true });
  const input = join(directory, "source.mp4");
  await exec("ffmpeg", [
    "-v",
    "error",
    "-y",
    "-f",
    "lavfi",
    "-i",
    "testsrc2=size=160x90:rate=12:duration=8",
    "-f",
    "lavfi",
    "-i",
    "sine=frequency=440:duration=8",
    "-c:v",
    "libx264",
    "-preset",
    "ultrafast",
    "-c:a",
    "aac",
    "-shortest",
    input,
  ]);
  return input;
}
async function properties(host: TestHost, id: string) {
  return (
    (await host.inspect()).plugins.find((p) => p.id === id)?.properties ?? {}
  );
}
async function loaded(host: TestHost, name: string) {
  await host.load(
    fileURLToPath(
      new URL(`../../plugins/${name}/manifest.json`, import.meta.url),
    ),
  );
  await host.until(
    async () => typeof (await properties(host, name)).status === "string",
  );
}
async function pick(
  host: TestHost,
  plugin: string,
  action: string,
  path: string | null,
) {
  await host.request("host.dialogs", { paths: [path] });
  await host.action(plugin, action);
}
async function state(
  host: TestHost,
  plugin: string,
  key: string,
  expected: unknown,
) {
  await host.until(
    async () => (await properties(host, plugin))[key] === expected,
  );
}
async function report(
  name: string,
  host: TestHost,
  details: Record<string, unknown>,
) {
  await writeFile(
    join(directory, `${name}.json`),
    JSON.stringify(
      {
        evidence: "F/B (not native UI)",
        ...details,
        snapshot: await host.inspect(),
      },
      null,
      2,
    ),
  );
}

test(
  "F/B: 九插件同载，真实播放列表跨页与恢复、暂停seek、高斯draft和owner清理",
  { timeout: 90000 },
  async (t) => {
    const input = await prepare();
    const host = new TestHost();
    t.after(() => host.close());
    for (const name of names) await loaded(host, name);
    assert.equal((await host.inspect()).plugins.length, 9);
    await pick(host, "playlist", "add", null);
    await host.action("playlist", "show");
    await state(host, "playlist", "visible", true);
    assert.equal((await properties(host, "playlist")).count, 0);
    const paths: string[] = [];
    for (let index = 0; index < 7; index++) {
      const path = join(directory, `playlist-${index}.mp4`);
      await copyFile(input, path);
      paths.push(path);
      await pick(host, "playlist", "add", path);
      await state(host, "playlist", "count", index + 1);
    }
    assert.equal((await properties(host, "playlist")).page, 2);
    await pick(host, "playlist", "add", paths[3]);
    await state(host, "playlist", "page", 1);
    assert.equal((await properties(host, "playlist")).count, 7);
    await host.action("playlist", "play-index", 4);
    await host.until(
      async () => (await host.inspect()).playback.path === paths[4],
    );
    await host.until(async () =>
      String((await properties(host, "playlist"))["items-text"]).includes(
        "▶5.",
      ),
    );
    await host.action("file-playback", "toggle-pause");
    await host.until(
      async () => (await host.inspect()).playback.paused === true,
    );
    await host.action("file-playback", "seek", 3);
    await host.until(
      async () => (await host.inspect()).playback.position === 3,
    );
    assert.equal((await host.inspect()).playback.paused, true);
    await host.action("gaussian-blur", "enabled", 1);
    await host.until(
      async () => (await host.inspect()).playback.gaussianEnabled === true,
    );
    await host.action("gaussian-blur", "sigma", 9);
    await host.action("file-playback", "seek", 4);
    await state(host, "gaussian-blur", "sigma", 9);
    assert.equal((await host.inspect()).playback.sigma, 2, "草稿不应提前应用");
    await host.action("gaussian-blur", "apply");
    await host.until(async () => (await host.inspect()).playback.sigma === 9);
    await host.action("video-adjust", "preset-bright");
    await host.until(
      async () =>
        Number((await properties(host, "video-adjust")).brightness) > 0,
    );
    await host.action("audio-workbench", "preset-dialogue");
    await state(host, "audio-workbench", "voice", true);
    const captions = join(directory, "combined.srt");
    await writeFile(
      captions,
      "1\n00:00:00,000 --> 00:00:08,000\nNINE PLUGIN COMPOSITION\n",
    );
    await pick(host, "subtitle-workbench", "open", captions);
    await state(host, "subtitle-workbench", "cue-count", 1);
    const compositionPixels: Buffer[] = [];
    const compositionFiles: string[] = [];
    for (const action of ["capture-raw", "capture-processed"]) {
      const output = join(directory, `combined-${action}-${Date.now()}.png`);
      const previous = (await properties(host, "capture-export"))[
        "operation-id"
      ];
      await pick(host, "capture-export", action, output);
      await host.until(async () => {
        const p = await properties(host, "capture-export");
        return p["operation-id"] !== previous && p.busy === false;
      });
      assert.match(
        String((await properties(host, "capture-export")).status),
        /已完成/,
      );
      compositionFiles.push(output);
      compositionPixels.push(
        (
          await exec(
            "ffmpeg",
            [
              "-v",
              "error",
              "-i",
              output,
              "-frames:v",
              "1",
              "-pix_fmt",
              "rgb24",
              "-f",
              "rawvideo",
              "pipe:1",
            ],
            { encoding: "buffer" },
          )
        ).stdout,
      );
    }
    assert.equal(compositionPixels[0].length, 160 * 90 * 3);
    assert.equal(compositionPixels[1].length, compositionPixels[0].length);
    const compositionDifference =
      compositionPixels[0].reduce(
        (sum, value, index) =>
          sum + Math.abs(value - compositionPixels[1][index]),
        0,
      ) / compositionPixels[0].length;
    assert.ok(
      compositionDifference > 5,
      "旧高斯与新画面/字幕链实际组合处理，非仅状态字段",
    );
    await host.action("playlist", "hide");
    await state(host, "playlist", "visible", false);
    await host.action("playlist", "toggle");
    await state(host, "playlist", "visible", true);
    assert.equal((await properties(host, "playlist")).page, 1);
    await host.unload("gaussian-blur");
    assert.equal((await host.inspect()).playback.gaussianEnabled, false);
    assert.ok(Number((await properties(host, "video-adjust")).brightness) > 0);
    const beforeBad = (await host.inspect()).playback;
    const bad = join(directory, "broken.mp4");
    await writeFile(bad, "broken");
    await pick(host, "file-playback", "open", bad);
    await host.until(async () =>
      /失败|invalid|moov|error/i.test(
        String((await properties(host, "file-playback")).status),
      ),
    );
    const failure = (await properties(host, "file-playback")).status;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 300));
    assert.equal(
      (await properties(host, "file-playback")).status,
      failure,
      "播放状态定时广播不能清除操作错误",
    );
    assert.equal(
      (await host.inspect()).playback.path,
      beforeBad.path,
      "损坏媒体不能丢弃原会话",
    );
    await pick(host, "file-playback", "open", paths[6]);
    await host.until(
      async () => (await host.inspect()).playback.path === paths[6],
    );
    await state(host, "file-playback", "status", paths[6]);
    await report("legacy-nine", host, {
      playlistCount: 7,
      draftSigma: 9,
      malformedLoadPreservedPath: beforeBad.path,
      compositionFiles,
      compositionDifference,
    });
  },
);

test(
  "F/B: 导出参数严格边界、错误扩展名、尾部burst保留成功文件与取消选择",
  { timeout: 60000 },
  async (t) => {
    const input = await prepare();
    const host = new TestHost();
    t.after(() => host.close());
    await loaded(host, "capture-export");
    const plugin = "capture-export";
    const observed: Record<string, unknown>[] = [];
    for (const [action, good, invalid] of [
      ["burst-count", 3, 3.5],
      ["burst-interval", 0.1, 0.09],
      ["clip-duration", 0.1, 600.1],
      ["include-subtitles", 0, 2],
    ] as const) {
      await host.action(plugin, action, good);
      await state(
        host,
        plugin,
        action,
        action === "include-subtitles" ? false : good,
      );
      await host.action(plugin, action, invalid);
      await host.until(async () =>
        String((await properties(host, plugin)).status).includes("操作失败"),
      );
      assert.equal(
        (await properties(host, plugin))[action],
        action === "include-subtitles" ? false : good,
      );
      observed.push({
        action,
        good,
        invalid,
        error: (await properties(host, plugin)).status,
      });
    }
    await host.request("player.load", { path: input });
    await host.request("player.togglePause");
    await pick(host, plugin, "capture-raw", join(directory, "wrong.jpg"));
    await host.until(
      async () =>
        (await properties(host, plugin)).busy === false &&
        String((await properties(host, plugin)).status).includes(".png"),
    );
    await pick(host, plugin, "capture-raw", null);
    await host.until(
      async () =>
        (await properties(host, plugin)).busy === false &&
        String((await properties(host, plugin)).status).includes("取消选择"),
    );
    await host.action(plugin, "burst-count", 3);
    await state(host, plugin, "burst-count", 3);
    await host.action(plugin, "burst-interval", 0.5);
    await state(host, plugin, "burst-interval", 0.5);
    await host.request("player.seek", { seconds: 7.5 });
    const prefix = `tail-${Date.now()}`;
    await pick(host, plugin, "capture-burst", join(directory, `${prefix}.png`));
    await host.until(
      async () =>
        (await properties(host, plugin)).busy === false &&
        String((await properties(host, plugin)).status).includes(
          "超出媒体时长",
        ),
    );
    assert.equal((await properties(host, plugin))["completed-count"], 1);
    const images = (await readdir(directory)).filter((path) =>
      path.startsWith(prefix),
    );
    assert.deepEqual(images, [`${prefix}-001.png`]);
    const decoded = await exec("ffmpeg", [
      "-v",
      "error",
      "-i",
      join(directory, images[0]),
      "-frames:v",
      "1",
      "-f",
      "null",
      "-",
    ]);
    assert.equal(decoded.stderr, "");
    await report("capture-boundaries", host, {
      observed,
      tailFiles: images,
    });
  },
);

test(
  "F/B: 工坊需求文件取消、损坏JSON、未知字段、超长、恢复和无历史回退",
  { timeout: 60000 },
  async (t) => {
    await mkdir(directory, { recursive: true });
    const host = new TestHost();
    t.after(() => host.close());
    const plugin = "plugin-workshop";
    await loaded(host, plugin);
    await host.until(
      async () => (await properties(host, plugin)).phase === "idle",
    );
    const request = (await properties(host, plugin)).request;
    for (const [name, contents, expected] of [
      ["request-broken.json", "{", "SyntaxError"],
      [
        "request-extra.json",
        JSON.stringify({ prompt: "counter", increment: 1, extra: true }),
        "字段",
      ],
      ["request-large.json", " ".repeat(8193), "超过"],
    ]) {
      const path = join(directory, name);
      await writeFile(path, contents);
      await pick(host, plugin, "request-file", path);
      await host.until(async () =>
        String((await properties(host, plugin)).status).includes(expected),
      );
      assert.equal((await properties(host, plugin)).request, request);
      assert.equal((await properties(host, plugin)).busy, false);
    }
    const good = join(directory, "request-good.json");
    await writeFile(
      good,
      JSON.stringify({ prompt: "功能审计加四计数器", increment: 4 }),
    );
    await pick(host, plugin, "request-file", good);
    await state(host, plugin, "request", "功能审计加四计数器");
    await pick(host, plugin, "request-file", null);
    await host.action(plugin, "rollback");
    await host.until(async () =>
      String((await properties(host, plugin)).status).includes(
        "没有可回退版本",
      ),
    );
    assert.equal(
      (await properties(host, plugin)).request,
      "功能审计加四计数器",
    );
    await report("workshop-request-errors", host, {
      preservedRequestAfterCancel: "功能审计加四计数器",
    });
  },
);

test(
  "F/B: 字幕导航循环、正负delay、字号颜色与坏SRT不破坏原轨",
  { timeout: 60000 },
  async (t) => {
    const input = await prepare();
    const host = new TestHost();
    t.after(() => host.close());
    await loaded(host, "subtitle-workbench");
    await host.request("player.load", { path: input });
    await host.request("player.togglePause");
    const srt = join(directory, "multi.srt");
    await writeFile(
      srt,
      "1\n00:00:01,000 --> 00:00:02,000\nFIRST\n\n2\n00:00:03,000 --> 00:00:04,000\nSECOND\n\n3\n00:00:05,000 --> 00:00:06,000\nTHIRD\n",
    );
    await pick(host, "subtitle-workbench", "open", srt);
    await state(host, "subtitle-workbench", "cue-count", 3);
    const observed: number[] = [];
    for (const expected of [3, 5, 1]) {
      await host.action("subtitle-workbench", "next-cue");
      await host.until(
        async () => (await host.inspect()).playback.position === expected,
      );
      observed.push(Number((await host.inspect()).playback.position));
      assert.equal((await host.inspect()).playback.paused, true);
    }
    for (const delay of [0.5, -0.5]) {
      await host.action("subtitle-workbench", "delay", delay);
      await state(host, "subtitle-workbench", "delay", delay);
    }
    await host.action("subtitle-workbench", "font-size", 64);
    await state(host, "subtitle-workbench", "font-size", 64);
    await host.action("subtitle-workbench", "yellow");
    await state(host, "subtitle-workbench", "yellow", true);
    const bad = join(directory, "bad.srt");
    await writeFile(bad, "not srt");
    await pick(host, "subtitle-workbench", "open", bad);
    await host.until(async () =>
      String((await properties(host, "subtitle-workbench")).status).includes(
        "字幕缺少",
      ),
    );
    assert.equal(
      (await properties(host, "subtitle-workbench"))["cue-count"],
      3,
    );
    await host.action("subtitle-workbench", "next-cue");
    await host.until(
      async () => (await host.inspect()).playback.position === 2.5,
    );
    await report("subtitle-navigation", host, {
      observed,
      finalDelayedSeek: 2.5,
    });
  },
);

test(
  "F/B: NES坏ROM/异ROM与损坏存档/取消/拒绝覆盖保持真实会话",
  { timeout: 60000 },
  async (t) => {
    await mkdir(directory, { recursive: true });
    const path = join(directory, "audit.nes");
    const saved = join(directory, `nes-${Date.now()}.json`);
    const bad = join(directory, "bad.nes");
    await writeFile(path, testRom());
    await writeFile(bad, "bad rom");
    const host = new TestHost();
    t.after(() => host.close());
    await loaded(host, "nes");
    await pick(host, "nes", "open-rom", path);
    await host.until(async () => (await host.inspect()).source !== null);
    await host.action("nes", "toggle-pause");
    await host.until(
      async () => (await host.inspect()).playback.paused === true,
    );
    const session = (await host.inspect()).playback.sessionId;
    await pick(host, "nes", "open-rom", bad);
    await host.until(async () =>
      String((await properties(host, "nes")).status).includes("Not an iNES"),
    );
    assert.equal((await host.inspect()).playback.sessionId, session);
    await pick(host, "nes", "save-state", saved);
    await host.until(async () =>
      String((await properties(host, "nes")).status).includes("状态已保存"),
    );
    const original = await readFile(saved, "utf8");
    await pick(host, "nes", "save-state", saved);
    await host.until(async () =>
      String((await properties(host, "nes")).status).includes("EEXIST"),
    );
    assert.equal(await readFile(saved, "utf8"), original);
    const wrongRom: unknown = JSON.parse(original);
    assert.ok(wrongRom && typeof wrongRom === "object");
    const wrong = join(directory, "wrong-state.json");
    await writeFile(
      wrong,
      JSON.stringify({ ...wrongRom, rom: "different-rom" }),
    );
    for (const [filename, contents] of [
      ["invalid-state.json", "{"],
      ["empty-state.json", JSON.stringify({ version: 1, rom: "", state: {} })],
    ]) {
      await writeFile(join(directory, filename), contents);
    }
    for (const invalid of [
      wrong,
      join(directory, "invalid-state.json"),
      join(directory, "empty-state.json"),
    ]) {
      const previous = (await properties(host, "nes")).status;
      await pick(host, "nes", "load-state", invalid);
      await host.until(async () => {
        const status = String((await properties(host, "nes")).status);
        return status !== previous && status.includes("操作失败");
      });
      assert.equal((await host.inspect()).playback.sessionId, session);
      assert.equal((await host.inspect()).playback.paused, true);
    }
    await pick(host, "nes", "load-state", saved);
    await host.until(async () =>
      String((await properties(host, "nes")).status).includes("状态已恢复"),
    );
    await pick(host, "nes", "load-state", null);
    await host.action("nes", "close-rom");
    await host.until(async () => (await host.inspect()).source === null);
    await report("nes-negative", host, {
      sessionPreserved: session,
      savedBytes: Buffer.byteLength(original),
    });
  },
);
