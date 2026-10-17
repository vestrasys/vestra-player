// 实验 N1/N2：真实 Node 插件 + Rust 来源宿主，不替代 ROM 执行或帧/音频传输。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../../support/host.ts";
import { testRom } from "./rom.ts";
interface Inspection {
  playback: { duration: number | null; sourceKind: string; paused: boolean };
  source: null | {
    frames: number;
    audioBytes: number;
    frameBytes: number;
    frameHash: string;
    paused: boolean;
  };
  plugins: { id: string; properties: Record<string, unknown> }[];
}
const manifest = fileURLToPath(
  new URL("../../../plugins/nes/manifest.json", import.meta.url),
);
test("N1/N2 isolated worker: ROM dialog, input, pause/resume, disk state and unload", async () => {
  const directory = await mkdtemp(join(tmpdir(), "vestra-nes-"));
  const path = join(directory, "self-authored.nes");
  const save = join(directory, "saved.json");
  await writeFile(path, testRom());
  const host = new TestHost();
  const inspect = () => host.request<Inspection>("host.inspect");
  const action = async (name: string) => {
    await host.action("nes", name);
  };
  try {
    await host.load(manifest);
    await host.until(async () =>
      (await inspect()).plugins.some(
        (p) => p.id === "nes" && typeof p.properties.status === "string",
      ),
    );
    await host.request("host.dialogs", { paths: [path] });
    await action("open-rom");
    await host.until(async () => {
      const s = (await inspect()).source;
      return !!s && s.frames > 10 && s.audioBytes > 0;
    });
    const running = await inspect();
    assert.equal(running.playback.duration, null);
    assert.equal(running.playback.sourceKind, "external");
    assert.equal(running.source?.frameBytes, 256 * 240 * 3);
    const released = running.source?.frameHash;
    await host.request("host.input", {
      plugin: "nes",
      key: "z",
      pressed: true,
    });
    await host.until(
      async () => (await inspect()).source?.frameHash !== released,
    );
    await action("toggle-pause");
    await host.until(async () => (await inspect()).playback.paused);
    const paused = await inspect();
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert.equal((await inspect()).source?.frames, paused.source?.frames);
    await host.request("host.dialogs", { paths: [save] });
    await action("save-state");
    await host.until(async () => {
      try {
        return JSON.parse(await readFile(save, "utf8")).version === 1;
      } catch {
        return false;
      }
    });
    await host.request("host.input", {
      plugin: "nes",
      key: "z",
      pressed: false,
    });
    await action("toggle-pause");
    await host.until(
      async () => (await inspect()).source?.frameHash === released,
    );
    await action("toggle-pause");
    await host.until(async () => (await inspect()).playback.paused);
    await host.request("host.dialogs", { paths: [save] });
    await action("load-state");
    await host.until(async () =>
      String((await inspect()).plugins[0].properties.status).includes(
        "状态已恢复",
      ),
    );
    await action("toggle-pause");
    await host.until(
      async () =>
        (await inspect()).source?.frameHash === paused.source?.frameHash,
    );
    await host.unload("nes");
    assert.equal((await inspect()).source, null);
  } catch (error) {
    console.error(JSON.stringify(await inspect()));
    throw error;
  } finally {
    await host.close();
    await rm(directory, { recursive: true, force: true });
  }
});
