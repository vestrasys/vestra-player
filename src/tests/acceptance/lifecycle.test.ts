// 独立实验验收：断言公共契约与可观察资源，不复用插件业务实现。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../support/host.ts";

const subtitles = fileURLToPath(
  new URL("../../plugins/subtitle-workbench/manifest.json", import.meta.url),
);

test("I2: unloading a real plugin completes its asynchronous filesystem disposer", async (t) => {
  const temporary = await mkdtemp(
    join(tmpdir(), "vestra-lifecycle-acceptance-"),
  );
  t.after(() => rm(temporary, { recursive: true, force: true }));
  const before = new Set(await readdir(temporary));
  const priorTemporary = process.env.TMPDIR;
  process.env.TMPDIR = temporary;
  let host: TestHost;
  try {
    host = new TestHost();
  } finally {
    if (priorTemporary === undefined) delete process.env.TMPDIR;
    else process.env.TMPDIR = priorTemporary;
  }
  t.after(() => host.close());
  await host.load(subtitles);
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (plugin) => plugin.properties.status === "字幕工作台就绪",
    ),
  );
  const created = (await readdir(temporary)).filter(
    (entry) => entry.startsWith("vestra-subtitles-") && !before.has(entry),
  );
  t.after(async () => {
    for (const entry of created)
      await rm(join(temporary, entry), { recursive: true, force: true });
  });
  assert.equal(created.length, 1, "the worker must really create its cache");
  await host.unload("subtitle-workbench");
  // Unload may drain asynchronously, but it must finish within a bounded interval.
  const deadline = Date.now() + 2_000;
  let remaining = created;
  while (remaining.length && Date.now() < deadline) {
    const after = new Set(await readdir(temporary));
    remaining = created.filter((entry) => after.has(entry));
    if (remaining.length)
      await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.deepEqual(remaining, [], "plugin disposer must remove its cache");
});

test("C1: explicit sample rates cannot silently fall back on fractional values", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  await assert.rejects(
    host.request("sources.open", {
      key: "acceptance-source",
      width: 2,
      height: 2,
      sampleRate: 44100.5,
    }),
    /sample.?rate|integer/i,
  );
  assert.equal((await host.inspect()).source, null);
});

test("C1: public removal rejects a plugin not installed through management", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  await host.load(subtitles);
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (plugin) => plugin.properties.status === "字幕工作台就绪",
    ),
  );
  await assert.rejects(
    host.request("plugins.remove", { id: "subtitle-workbench" }),
    /installed|generated|managed/i,
  );
  assert.equal((await host.inspect()).plugins.length, 1);
  await host.unload("subtitle-workbench");
});
