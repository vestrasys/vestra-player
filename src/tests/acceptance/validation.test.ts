// 独立实验边界验收：显式错误类型不得被当作可选字段缺失。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../support/host.ts";

test("C1: optional fields and headless dialogs obey the same typed contract", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  const directory = await mkdtemp(join(tmpdir(), "vestra-validation-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const input = fileURLToPath(
    new URL("../../samples/sample.mp4", import.meta.url),
  );
  await t.test(
    "a malformed session fence cannot silently become persistent",
    async () => {
      await assert.rejects(
        host.request("effects.set", {
          key: "invalid-session",
          kind: "audio",
          filter: "volume=0.8",
          sessionId: 42,
        }),
      );
    },
  );
  await t.test(
    "an explicitly invalid task filter cannot disappear",
    async () => {
      await assert.rejects(
        host.request("media.start", {
          input,
          output: join(directory, "invalid.png"),
          format: "image",
          videoFilter: 42,
        }),
      );
    },
  );
  await t.test(
    "a malformed PCM field cannot silently commit a frame",
    async () => {
      const { sessionId } = await host.request<{ sessionId: string }>(
        "sources.open",
        { key: "source", width: 2, height: 2 },
      );
      await assert.rejects(
        host.request("sources.present", {
          key: "source",
          sessionId,
          rgb: Buffer.alloc(12).toString("base64"),
          pcm: 42,
        }),
      );
    },
  );
  await t.test(
    "dialog fixtures cannot bypass public parameter validation",
    async () => {
      await assert.rejects(
        host.request("dialogs.openFile", { title: 42, extensions: [42] }),
      );
    },
  );
  await t.test(
    "a healthy task status conforms to its optional error type",
    async () => {
      const job = await host.request<{ id: string }>("media.start", {
        input,
        output: join(directory, "healthy.png"),
        format: "image",
      });
      const status = await host.request<{ error?: unknown }>("media.status", {
        id: job.id,
      });
      assert(
        status.error === undefined || typeof status.error === "string",
        "JobState.error?: string does not permit null",
      );
      await host.request("media.cancel", { id: job.id });
    },
  );
});

test("C1: independently valid plugins cannot collide in the shared UI namespace", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "vestra-ui-collision-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const host = new TestHost();
  t.after(() => host.close());
  for (const id of ["namespace-first", "namespace-second"]) {
    const packageDirectory = join(directory, id);
    await mkdir(packageDirectory);
    await writeFile(
      join(packageDirectory, "package.json"),
      '{"type":"module"}',
    );
    await writeFile(
      join(packageDirectory, "index.ts"),
      "// 实验 fixture；非最终 API。\nexport async function activate() {}\n",
    );
    await writeFile(
      join(packageDirectory, "ui.slint"),
      "// 实验 fixture；非最终 API。\nexport global ConflictingState { callback action(string,float); }\nexport component ConflictingPanel inherits Rectangle {}\n",
    );
    await writeFile(
      join(packageDirectory, "manifest.json"),
      JSON.stringify({
        id,
        name: id,
        experimental: true,
        entry: "index.ts",
        ui: {
          file: "ui.slint",
          component: "ConflictingPanel",
          global: "ConflictingState",
        },
      }),
    );
  }
  await host.load(join(directory, "namespace-first", "manifest.json"));
  await assert.rejects(
    host.load(join(directory, "namespace-second", "manifest.json")),
    /duplicate|conflict|global|component|namespace/i,
  );
  assert.equal((await host.inspect()).plugins.length, 1);
});
