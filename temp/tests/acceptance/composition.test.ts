// 独立实验验收：真实插件组合顺序、owner 清理和异常进程退出。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TestHost } from "../support/host.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const probes = [
  "video-adjust",
  "audio-workbench",
  "subtitle-workbench",
  "capture-export",
  "nes",
  "plugin-workshop",
];

for (const order of [probes, [...probes].reverse()]) {
  test(`I2: loaded in ${order.join(" → ")}, removing video leaves audio usable`, async (t) => {
    const host = new TestHost();
    t.after(() => host.close());
    for (const id of order) {
      await host.load(join(root, "plugins", id, "manifest.json"));
      await host.until(async () =>
        (await host.inspect()).plugins.some(
          (plugin) =>
            plugin.id === id && typeof plugin.properties.status === "string",
        ),
      );
    }
    assert.equal((await host.inspect()).plugins.length, probes.length);
    await host.action("audio-workbench", "preset-dialogue");
    await host.action("video-adjust", "brightness-up");
    await host.until(async () => {
      const effects = await host.request<{ owner: string }[]>("effects.list");
      return (
        effects.some((effect) => effect.owner === "audio-workbench") &&
        effects.some((effect) => effect.owner === "video-adjust")
      );
    });
    await host.action("plugin-workshop", "generate", 3);
    await host.until(
      async () =>
        (await host.inspect()).plugins.some(
          (plugin) =>
            plugin.id === "plugin-workshop" &&
            plugin.properties.phase === "installed",
        ),
      30_000,
    );
    await host.action("workshop-generated-counter", "increment");
    await host.until(async () =>
      (await host.inspect()).plugins.some(
        (plugin) =>
          plugin.id === "workshop-generated-counter" &&
          plugin.properties.count === 3,
      ),
    );
    await host.unload("video-adjust");
    const remaining = await host.request<{ owner: string }[]>("effects.list");
    assert.deepEqual(
      remaining.map((effect) => effect.owner),
      ["audio-workbench"],
    );
    await host.action("audio-workbench", "preset-music");
    await host.until(async () =>
      (await host.inspect()).plugins.some(
        (plugin) =>
          plugin.id === "audio-workbench" && plugin.properties.bass === 5,
      ),
    );
    for (const id of order.filter((id) => id !== "video-adjust"))
      await host.unload(id);
    await host.action("workshop-generated-counter", "increment");
    await host.until(async () =>
      (await host.inspect()).plugins.some(
        (plugin) =>
          plugin.id === "workshop-generated-counter" &&
          plugin.properties.count === 6,
      ),
    );
    await host.unload("workshop-generated-counter");
    const final = await host.request<{
      plugins: unknown[];
      resources: {
        effects: number;
        jobs: number;
        snapshots: number;
        source: boolean;
      };
    }>("host.inspect");
    assert.deepEqual(final.plugins, []);
    assert.deepEqual(final.resources, {
      effects: 0,
      jobs: 0,
      snapshots: 0,
      source: false,
      managedAssets: 0,
    });
  });
}

test("I2: abrupt worker exit releases its effect, source and running media job", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "vestra-acceptance-crash-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const host = new TestHost();
  t.after(() => host.close());
  const output = join(directory, "unfinished.mp4");
  await writeFile(join(directory, "package.json"), '{"type":"module"}');
  await writeFile(
    join(directory, "manifest.json"),
    JSON.stringify({
      id: "acceptance-crash",
      name: "Acceptance crash fixture",
      experimental: true,
      entry: "index.ts",
      ui: { file: "ui.slint", component: "CrashPanel", global: "CrashState" },
    }),
  );
  await writeFile(
    join(directory, "ui.slint"),
    "// 实验故障注入，非成品。\nexport global CrashState { in-out property<string> status; callback action(string,float); }\nexport component CrashPanel inherits Rectangle { Text { text: CrashState.status; } }",
  );
  await writeFile(
    join(directory, "index.ts"),
    `// 实验故障注入；与成品 API 差异很大。
import type { PluginContext } from ${JSON.stringify(join(root, "sdk/types.ts"))};
export async function activate(ctx: PluginContext) {
  await ctx.effects.set({key:"owned",kind:"audio",filter:"volume=0.7"});
  await ctx.sources.open({key:"owned",width:2,height:2});
  await ctx.media.start({input:${JSON.stringify(join(root, "samples/sample.mp4"))},output:${JSON.stringify(output)},format:"clip",videoFilter:"scale=3840:2160,gblur=sigma=10:steps=3"});
  ctx.onAction("crash",()=>{process.exit(9);});
  await ctx.ui.patch({status:"ready"});
}
`,
  );
  await host.load(join(directory, "manifest.json"));
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (plugin) => plugin.properties.status === "ready",
    ),
  );
  const before = await host.request<{
    resources: { effects: number; jobs: number; source: boolean };
  }>("host.inspect");
  assert.equal(before.resources.effects, 1);
  assert.equal(before.resources.jobs, 1);
  assert.equal(before.resources.source, true);
  await host.action("acceptance-crash", "crash");
  await host.until(async () => (await host.inspect()).plugins.length === 0);
  const after = await host.request<{
    resources: {
      effects: number;
      jobs: number;
      snapshots: number;
      source: boolean;
    };
  }>("host.inspect");
  assert.deepEqual(after.resources, {
    effects: 0,
    jobs: 0,
    snapshots: 0,
    source: false,
    managedAssets: 0,
  });
  assert(
    !(await readdir(directory)).some(
      (entry) => entry.startsWith(".vestra-") || entry === "unfinished.mp4",
    ),
  );
});
