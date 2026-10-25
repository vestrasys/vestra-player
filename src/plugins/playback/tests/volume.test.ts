import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { OUTPUT_SAVE_MS } from "../output-memory.ts";

const directory = new URL("..", import.meta.url);
const start = (dataDirectory: string, host = new TestHost()) => startPlugin(activate, { directory, host, dataDirectory });
const file = (data: string) => join(data, "output.json");
const read = (data: string) => {
  try {
    return JSON.parse(readFileSync(file(data), "utf8")) as Record<string, unknown>;
  } catch {
    return null;
  }
};
async function until(predicate: () => boolean, ms = 2000) {
  const deadline = Date.now() + ms;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const output = (host: TestHost) => host.source.output;

async function withData(name: string, body: (data: string) => Promise<void>) {
  const data = mkdtempSync(join(tmpdir(), `playback-${name}-`));
  try {
    await body(data);
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
}

test("volume: the bar's volume comes back after a restart, before anything is opened", () =>
  withData("volume", async (data) => {
    const first = await start(data);
    first.host.mounted().emit("volume-released", 35);
    await until(() => read(data)?.volume === 35);
    assert.deepEqual(read(data), { volume: 35, muted: false });
    await first.stop();

    const second = await start(data);
    await until(() => output(second.host).volume === 35);
    assert.equal(second.host.mounted().props.volume, 35);
    assert.equal(second.plugin.player.state.phase, "empty", "nothing is opened");
    // The first file plays at the remembered volume.
    second.host.openElsewhere("/movie.mp4");
    second.host.setSource({ duration: 100, position: 0 });
    await second.host.settle();
    assert.equal(second.plugin.player.state.volume, 35);
    await second.stop();
  }));

test("volume: mute is remembered with the volume", () =>
  withData("mute", async (data) => {
    const first = await start(data);
    first.host.mounted().emit("volume-released", 40);
    await until(() => output(first.host).volume === 40);
    first.host.mounted().emit("action", "mute");
    await until(() => read(data)?.muted === true);
    assert.deepEqual(read(data), { volume: 40, muted: true });
    await first.stop();

    const second = await start(data);
    await until(() => output(second.host).muted);
    assert.deepEqual(output(second.host), { volume: 40, muted: true });
    assert.equal(second.host.mounted().props.muted, true);
    // Unmuting is remembered too.
    second.host.mounted().emit("action", "mute");
    await until(() => read(data)?.muted === false);
    await second.stop();
  }));

test("volume: a drag is written once, when the slider is released", () =>
  withData("drag", async (data) => {
    const app = await start(data);
    const bar = app.host.mounted();
    for (const value of [90, 80, 70, 60]) {
      bar.emit("volume-changed", value);
      await pause(OUTPUT_SAVE_MS / 2);
    }
    await pause(OUTPUT_SAVE_MS + 200);
    assert.equal(output(app.host).volume, 60, "the player follows the drag");
    assert.equal(existsSync(file(data)), false, "nothing is written while the slider is held");
    bar.emit("volume-released", 55);
    await until(() => read(data)?.volume === 55);
    const written = statSync(file(data)).mtimeMs;
    await pause(OUTPUT_SAVE_MS + 200);
    assert.equal(statSync(file(data)).mtimeMs, written, "one write for the whole drag");
    await app.stop();
  }));

test("volume: applied once at start; later changes by others are adopted and saved (above 100 too)", () =>
  withData("external", async (data) => {
    writeFileSync(file(data), JSON.stringify({ volume: 30, muted: false }));
    const app = await start(data);
    await until(() => output(app.host).volume === 30);
    // The volume keys or another plugin: kept, not fought.
    app.host.setSource({ output: { volume: 150, muted: false } });
    await until(() => read(data)?.volume === 150);
    await app.host.settle(OUTPUT_SAVE_MS);
    assert.equal(output(app.host).volume, 150);
    assert.equal(app.host.sent("playback.set").filter((p) => p.property === "volume").length, 1);
    await app.stop();

    const second = await start(data);
    await until(() => output(second.host).volume === 150);
    assert.equal(second.host.mounted().props.volume, 100, "the slider shows a boost at its right end");
    await second.stop();
  }));

test("volume: nothing remembered yet sets nothing and writes nothing", () =>
  withData("first-run", async (data) => {
    const app = await start(data);
    await app.host.settle(OUTPUT_SAVE_MS + 100);
    assert.equal(app.host.sent("playback.set").length, 0);
    assert.equal(existsSync(file(data)), false);
    await app.stop();
  }));

test("volume: out-of-range values are clamped to the player's range; bad fields are ignored", () =>
  withData("range", async (data) => {
    writeFileSync(file(data), JSON.stringify({ volume: 999, muted: "yes" }));
    const high = await start(data);
    await until(() => output(high.host).volume === 200);
    assert.equal(output(high.host).muted, false);
    await high.stop();

    writeFileSync(file(data), JSON.stringify({ volume: -5, muted: true }));
    const low = await start(data);
    await until(() => output(low.host).muted);
    assert.equal(output(low.host).volume, 0);
    await low.stop();

    writeFileSync(file(data), JSON.stringify({ volume: "loud" }));
    const bad = await start(data);
    await bad.host.settle(50);
    assert.equal(bad.host.sent("playback.set").length, 0);
    await bad.stop();
  }));

test("volume: a broken file is set aside and the player's volume stands", () =>
  withData("corrupt", async (data) => {
    for (const text of ["{ not json", "[1, 2]", "42"]) {
      writeFileSync(file(data), text);
      const app = await start(data);
      await app.host.settle(50);
      assert.equal(app.host.sent("playback.set").length, 0, text);
      assert.deepEqual(output(app.host), { volume: 100, muted: false });
      assert.equal(existsSync(file(data)), false, "the broken file is renamed");
      // The next change is saved normally.
      app.host.mounted().emit("volume-released", 25);
      await until(() => read(data)?.volume === 25);
      await app.stop();
      for (const name of readdirSync(data)) rmSync(join(data, name));
    }
  }));
