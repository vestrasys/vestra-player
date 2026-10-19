// 实验契约回归：真实 Rust/Slint 编译与 Node worker，不模拟宿主能力。
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { TestHost } from "./support/host.ts";
import { createContext } from "../sdk/context.ts";
const sourceRoot = fileURLToPath(new URL("../", import.meta.url));

test("SDK rejects non-JSON values before transport", async () => {
  let calls = 0;
  const { context } = createContext(async <T>() => {
    calls++;
    return undefined as T;
  });
  await assert.rejects(context.player.seek(Infinity), /non-finite/);
  await assert.rejects(context.ui.patch({ value: NaN }), /non-finite/);
  assert.equal(calls, 0);
});

test("Rust boundary rejects unknown fields, invalid filters, stale sessions and partial frames", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  await assert.rejects(
    host.request("effects.set", {
      key: "bad",
      kind: "video",
      filter: "eq",
      owner: "pretend",
    }),
    /unsupported field/,
  );
  await assert.rejects(
    host.request("effects.set", {
      key: "bad",
      kind: "video",
      filter: "nonexistent_filter",
    }),
    /filter rejected/,
  );
  assert.deepEqual(await host.request("effects.list"), []);
  await assert.rejects(
    host.request("sources.open", {
      key: "source",
      width: 2,
      height: 2,
      sampleRate: -1,
    }),
    /sample.?rate|integer/i,
  );
  const first = await host.request<{ sessionId: string }>("sources.open", {
    key: "source",
    width: 2,
    height: 2,
  });
  const valid = {
    key: "source",
    sessionId: first.sessionId,
    rgb: Buffer.alloc(12, 60).toString("base64"),
  };
  await assert.rejects(
    host.request("sources.present", { ...valid, rgb: "AA==" }),
    /size mismatch/,
  );
  await assert.rejects(
    host.request("sources.present", { ...valid, pcm: "AA==" }),
    /PCM/,
  );
  let state = await host.request<{ source: { frames: number } }>(
    "host.inspect",
  );
  assert.equal(state.source.frames, 0, "rejected PCM cannot commit RGB");
  await host.request("sources.setPaused", { key: "source", paused: true });
  await assert.rejects(host.request("sources.present", valid), /paused/);
  await host.request("sources.setPaused", { key: "source", paused: false });
  await host.request("sources.present", valid);
  state = await host.request("host.inspect");
  assert.equal(state.source.frames, 1);
  const playback = await host.request<{
    duration: null;
    path: null;
    sourceKind: string;
    paused: boolean;
  }>("player.snapshot");
  assert.equal(playback.path, null);
  assert.equal(playback.duration, null);
  assert.equal(playback.sourceKind, "external");
  assert.equal(playback.paused, false);
  await host.request("sources.open", { key: "source", width: 2, height: 2 });
  await assert.rejects(host.request("sources.present", valid), /stale session/);
  await host.request("sources.close", { key: "source" });
});

async function candidate(
  root: string,
  folder: string,
  version: string,
  broken = false,
) {
  const directory = join(root, folder);
  await mkdir(directory, { recursive: true });
  await writeFile(
    join(directory, "package.json"),
    JSON.stringify({ type: "module" }),
  );
  await writeFile(
    join(directory, "manifest.json"),
    JSON.stringify({
      id: "contract-generated",
      name: "Contract generated",
      experimental: true,
      entry: "index.ts",
      commands: [{ id: "increment", title: "Increment", menu: ["Tools"] }],
      ui: {
        file: "ui.slint",
        component: "ContractPanel",
        global: "ContractState",
      },
    }),
  );
  await writeFile(
    join(directory, "ui.slint"),
    "// 实验界面，非最终 API。\nexport global ContractState {in-out property<string> status; in-out property<int> count; callback action(string,float);}\nexport component ContractPanel inherits Rectangle {Text {text:ContractState.status;}}",
  );
  await writeFile(
    join(directory, "index.ts"),
    `// 实验生成包，非最终 API。\nimport type {PluginContext} from ${JSON.stringify(join(sourceRoot, "sdk/types.ts"))};\nexport async function activate(ctx:PluginContext){let count=0;await ctx.ui.patch(${broken ? "{unknown: true}" : `{status:${JSON.stringify(version)},count:0}`});ctx.onAction('increment',async()=>{await ctx.ui.patch({count:++count});});}`,
  );
  return join(directory, "manifest.json");
}

test("generated packages compile, activate, execute commands and roll back failed replacement", async (t) => {
  const cache = join(sourceRoot, ".cache");
  await mkdir(cache, { recursive: true });
  const root = await mkdtemp(join(cache, "contract-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const host = new TestHost();
  t.after(() => host.close());
  const good = await candidate(root, "good", "working");
  const checked = await host.request<{ valid: boolean; diagnostics: string[] }>(
    "plugins.validate",
    { manifest: good },
  );
  assert.equal(checked.valid, true, checked.diagnostics.join("\n"));
  await host.request("plugins.install", { manifest: good });
  await host.action("contract-generated", "increment");
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (p) => p.id === "contract-generated" && p.properties.count === 1,
    ),
  );
  const bad = await candidate(root, "bad", "broken", true);
  await assert.rejects(
    host.request("plugins.install", { manifest: bad }),
    /unknown Slint property/,
  );
  assert.equal(
    (await host.inspect()).plugins.filter((p) => p.id === "contract-generated")
      .length,
    1,
  );
  await host.action("contract-generated", "increment");
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (p) => p.properties.count === 2 && p.properties.status === "working",
    ),
  );
  await writeFile(
    join(root, "bad/index.ts"),
    'export const invalid: number = "type error";',
  );
  const invalid = await host.request<{ valid: boolean; diagnostics: string[] }>(
    "plugins.validate",
    { manifest: bad },
  );
  assert.equal(invalid.valid, false);
  assert.match(invalid.diagnostics.join("\n"), /not assignable/);
  await host.request("plugins.remove", { id: "contract-generated" });
  assert.deepEqual((await host.inspect()).plugins, []);
  assert.equal((await readdir(root)).length, 2);
});

test("legacy Gaussian worker releases its real backend effect when unloaded", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  await host.load(join(sourceRoot, "plugins/gaussian-blur/manifest.json"));
  await host.until(async () =>
    (await host.inspect()).plugins.some(
      (p) =>
        p.id === "gaussian-blur" && typeof p.properties.status === "string",
    ),
  );
  await host.action("gaussian-blur", "enabled", 1);
  await host.until(
    async () =>
      (await host.request<{ gaussianEnabled: boolean }>("player.snapshot"))
        .gaussianEnabled,
  );
  await host.unload("gaussian-blur");
  assert.equal(
    (await host.request<{ gaussianEnabled: boolean }>("player.snapshot"))
      .gaussianEnabled,
    false,
  );
});
