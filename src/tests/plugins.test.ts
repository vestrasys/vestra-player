// 仅用于 Slint 实验；本版本测试针对临时协议，与最终成品 API 会有很大差异。
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { record } from "../sdk/validation.ts";
import type { UiProperties, PlaybackState } from "../sdk/types.ts";

const runtime = fileURLToPath(
  new URL("../runtime/plugin-host.ts", import.meta.url),
);
interface Message {
  type: string;
  id?: number;
  method?: string;
  error: string;
  params: {
    properties: UiProperties;
    enabled?: boolean;
    sigma?: number;
    [key: string]: unknown;
  };
}
function decodeMessage(line: string): Message {
  const raw = record(JSON.parse(line));
  const params = raw.params === undefined ? {} : record(raw.params);
  const properties =
    params.properties === undefined ? {} : record(params.properties);
  const ui: UiProperties = {};
  for (const [key, value] of Object.entries(properties)) {
    if (
      typeof value !== "string" &&
      typeof value !== "number" &&
      typeof value !== "boolean"
    )
      throw new Error("invalid UI test response");
    ui[key] = value;
  }
  const decodedParams = {
    ...params,
    ...(params.properties === undefined ? {} : { properties: ui }),
    ...(typeof params.enabled === "boolean" ? { enabled: params.enabled } : {}),
    ...(typeof params.sigma === "number" ? { sigma: params.sigma } : {}),
  };
  if (params.properties === undefined)
    Object.defineProperty(decodedParams, "properties", {
      value: ui,
      enumerable: false,
    });
  return {
    type: String(raw.type),
    ...(typeof raw.id === "number" ? { id: raw.id } : {}),
    ...(typeof raw.method === "string" ? { method: raw.method } : {}),
    error: typeof raw.error === "string" ? raw.error : "",
    params: decodedParams as Message["params"],
  };
}

function startPlugin(
  name: string,
  handle: (message: Message) => unknown | Promise<unknown>,
) {
  const plugin = fileURLToPath(
    new URL(`../plugins/${name}/index.ts`, import.meta.url),
  );
  const child = spawn(process.execPath, [runtime, plugin], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const messages: Message[] = [];
  const observers = new Set<() => void>();
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", async (line) => {
    const message = decodeMessage(line);
    messages.push(message);
    for (const observer of observers) observer();
    if (message.type !== "request") return;
    try {
      const result = await handle(message);
      child.stdin.write(
        JSON.stringify({ type: "response", id: message.id, result }) + "\n",
      );
    } catch (error) {
      child.stdin.write(
        JSON.stringify({
          type: "response",
          id: message.id,
          error: String(error),
        }) + "\n",
      );
    }
  });
  const exit = new Promise<number | null>((resolve) =>
    child.once("close", resolve),
  );
  function waitFor(
    predicate: (message: Message) => boolean,
    after = 0,
    timeoutMs = 5_000,
  ): Promise<Message> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        observers.delete(check);
        reject(
          new Error(`未收到预期消息；${stderr}\n${JSON.stringify(messages)}`),
        );
      }, timeoutMs);
      function check() {
        const match = messages.slice(after).find(predicate);
        if (match) {
          clearTimeout(timer);
          observers.delete(check);
          resolve(match);
        }
      }
      observers.add(check);
      check();
    });
  }
  return {
    messages,
    diagnostics: () => stderr,
    waitFor,
    exit,
    send(message: Record<string, unknown>) {
      child.stdin.write(JSON.stringify(message) + "\n");
    },
    async stop() {
      child.stdin.write(JSON.stringify({ type: "shutdown" }) + "\n");
      const timer = setTimeout(() => child.kill("SIGKILL"), 2_000);
      const code = await exit;
      clearTimeout(timer);
      assert.equal(code, 0, stderr);
    },
    kill() {
      child.kill("SIGKILL");
    },
  };
}

const initial: PlaybackState = {
  path: null,
  paused: true,
  duration: 120,
  position: 0,
  gaussianEnabled: false,
  sigma: 2,
};
const request = (method: string) => (message: Message) =>
  message.type === "request" && message.method === method;

test("文件插件经真实子进程打开媒体、取消、播放和跳转，并响应宿主状态", async (t) => {
  let cancelled = true;
  const plugin = startPlugin("file-playback", (message) => {
    if (message.method === "dialogs.openMedia")
      return cancelled ? null : "/tmp/视频.mp4";
    return null;
  });
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  plugin.send({ type: "action", name: "open", value: 0 });
  await plugin.waitFor(request("dialogs.openMedia"));
  plugin.send({ type: "action", name: "toggle-pause", value: 0 });
  await plugin.waitFor(request("player.togglePause"));
  assert.equal(
    plugin.messages.some(request("player.load")),
    false,
    "取消选择不得播放",
  );
  cancelled = false;
  plugin.send({ type: "action", name: "open", value: 0 });
  const load = await plugin.waitFor(request("player.load"));
  assert.deepEqual(load.params, { path: "/tmp/视频.mp4" });
  plugin.send({ type: "action", name: "seek", value: 45 });
  assert.deepEqual((await plugin.waitFor(request("player.seek"))).params, {
    seconds: 45,
  });
  plugin.send({
    type: "playback",
    state: { ...initial, path: "/tmp/视频.mp4", paused: false, position: 45 },
  });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) && message.params.properties.position === 45,
  );
  await plugin.stop();
});

test("高斯插件保留未应用参数，配置后端，后端失败时恢复真实状态，卸载时撤销滤镜", async (t) => {
  let fail = false;
  let state = { ...initial };
  const plugin = startPlugin("gaussian-blur", (message) => {
    if (message.method === "player.snapshot") return state;
    if (message.method === "player.filters.setGaussian") {
      if (fail) throw new Error("后端不支持滤镜");
      state = {
        ...state,
        gaussianEnabled: message.params.enabled === true,
        sigma: Number(message.params.sigma),
      };
    }
    return null;
  });
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  plugin.send({ type: "action", name: "sigma", value: 7 });
  plugin.send({ type: "playback", state });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) && message.params.properties.sigma === 7,
  );
  plugin.send({ type: "action", name: "enabled", value: 1 });
  const enabled = await plugin.waitFor(request("player.filters.setGaussian"));
  assert.deepEqual(enabled.params, { enabled: true, sigma: 7 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.status === "高斯模糊已启用",
  );
  fail = true;
  const after = plugin.messages.length;
  plugin.send({ type: "action", name: "sigma", value: 10 });
  plugin.send({ type: "action", name: "apply", value: 0 });
  await plugin.waitFor(request("player.snapshot"), after);
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) && message.params.properties.sigma === 7,
    after,
  );
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      String(message.params.properties.status).includes("后端不支持滤镜"),
    after,
  );
  fail = false;
  const beforeStop = plugin.messages.length;
  await plugin.stop();
  assert.ok(
    plugin.messages
      .slice(beforeStop)
      .some(
        (message) =>
          request("player.filters.setGaussian")(message) &&
          message.params.enabled === false,
      ),
  );
});

test("两个独立插件可并存，关闭其中一个后另一个继续处理命令", async (t) => {
  const file = startPlugin("file-playback", () => null);
  const blur = startPlugin("gaussian-blur", () => null);
  t.after(() => {
    file.kill();
    blur.kill();
  });
  await Promise.all([
    file.waitFor((message) => message.type === "ready"),
    blur.waitFor((message) => message.type === "ready"),
  ]);
  await blur.stop();
  file.send({ type: "action", name: "toggle-pause", value: 0 });
  await file.waitFor(request("player.togglePause"));
  await file.stop();
});

test("宿主拒绝初始化时，插件报告启动失败并退出，不虚报 ready", async (t) => {
  const plugin = startPlugin("file-playback", () => {
    throw new Error("无法设置 Slint 属性");
  });
  t.after(() => plugin.kill());
  const fatal = await plugin.waitFor((message) => message.type === "fatal");
  assert.match(fatal.error, /无法设置 Slint 属性/);
  assert.equal(await plugin.exit, 1);
  assert.equal(
    plugin.messages.some((message) => message.type === "ready"),
    false,
  );
});

test("播放列表支持取消、去重、切换播放及隐藏重显后保留条目", async (t) => {
  let picked: string | null = null;
  const plugin = startPlugin("playlist", (message) =>
    message.method === "dialogs.openMedia" ? picked : null,
  );
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  let after = plugin.messages.length;
  plugin.send({ type: "action", name: "add", value: 0 });
  await plugin.waitFor(request("dialogs.openMedia"), after);
  // 后续toggle串行执行，确保取消分支已完成；不会新增条目。
  plugin.send({ type: "action", name: "toggle", value: 0 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.visible === false,
    after,
  );
  assert.equal(
    plugin.messages
      .slice(after)
      .some(
        (message) =>
          request("ui.patch")(message) &&
          Number(message.params.properties.count) > 0,
      ),
    false,
  );

  for (const [path, count] of [
    ["/tmp/一.mp4", 1],
    ["/tmp/一.mp4", 1],
    ["/tmp/二.mp4", 2],
  ] as const) {
    picked = path;
    after = plugin.messages.length;
    plugin.send({ type: "action", name: "add", value: 0 });
    await plugin.waitFor(
      (message) =>
        request("ui.patch")(message) &&
        message.params.properties.count === count,
      after,
    );
  }
  after = plugin.messages.length;
  plugin.send({ type: "action", name: "play-index", value: 1 });
  assert.deepEqual(
    (await plugin.waitFor(request("player.load"), after)).params,
    { path: "/tmp/二.mp4" },
  );
  plugin.send({ type: "playback", state: { ...initial, path: "/tmp/二.mp4" } });
  const playing = await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      String(message.params.properties["items-text"]).includes("▶2. 二.mp4"),
    after,
  );
  assert.equal(playing.params.properties.count, 2);
  assert.equal(
    "visible" in playing.params.properties,
    false,
    "状态变化不应自动重显侧栏",
  );

  after = plugin.messages.length;
  plugin.send({ type: "action", name: "show", value: 0 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.visible === true,
    after,
  );
  plugin.send({ type: "action", name: "hide", value: 0 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.visible === false,
    after,
  );
  after = plugin.messages.length;
  plugin.send({ type: "action", name: "toggle", value: 0 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.visible === true,
    after,
  );
  plugin.send({ type: "action", name: "play-index", value: 0 });
  assert.deepEqual(
    (await plugin.waitFor(request("player.load"), after)).params,
    { path: "/tmp/一.mp4" },
  );
  await plugin.stop();
});

test("播放列表三项分页不限制总容量，非法条目不会触发播放", async (t) => {
  let picked = "";
  const plugin = startPlugin("playlist", (message) =>
    message.method === "dialogs.openMedia" ? picked : null,
  );
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  for (let index = 0; index < 4; index++) {
    picked = `/tmp/${index}.mp4`;
    const after = plugin.messages.length;
    plugin.send({ type: "action", name: "add", value: 0 });
    await plugin.waitFor(
      (message) =>
        request("ui.patch")(message) &&
        message.params.properties.count === index + 1,
      after,
    );
  }
  let after = plugin.messages.length;
  plugin.send({ type: "action", name: "previous-page", value: 0 });
  await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) &&
      message.params.properties.page === 0 &&
      message.params.properties["has-next"] === true,
    after,
  );
  after = plugin.messages.length;
  plugin.send({ type: "action", name: "play-index", value: 4 });
  plugin.send({ type: "action", name: "play-index", value: -1 });
  plugin.send({ type: "action", name: "play-index", value: 1.5 });
  plugin.send({ type: "action", name: "next-page", value: 0 });
  const lastPage = await plugin.waitFor(
    (message) =>
      request("ui.patch")(message) && message.params.properties.page === 1,
    after,
  );
  assert.equal(lastPage.params.properties["items-text"], " 4. 3.mp4");
  assert.equal(lastPage.params.properties["has-next"], false);
  assert.equal(
    plugin.messages.slice(after).some(request("player.load")),
    false,
  );
  plugin.send({ type: "action", name: "play-index", value: 3 });
  assert.deepEqual(
    (await plugin.waitFor(request("player.load"), after)).params,
    { path: "/tmp/3.mp4" },
  );
  await plugin.stop();
});

test(
  "交互对话框等待超过普通30秒RPC期限后仍由真实回复完成",
  { timeout: 40_000 },
  async (t) => {
    const started = Date.now();
    const plugin = startPlugin("file-playback", async (message) => {
      if (message.method === "dialogs.openMedia") {
        await new Promise((resolve) => setTimeout(resolve, 31_200));
        return null;
      }
      return null;
    });
    t.after(() => plugin.kill());
    await plugin.waitFor((message) => message.type === "ready");
    plugin.send({ type: "action", name: "open", value: 0 });
    await plugin.waitFor(request("dialogs.openMedia"));
    plugin.send({ type: "action", name: "toggle-pause", value: 0 });
    await plugin.waitFor(request("player.togglePause"), 0, 35_000);
    assert.ok(
      Date.now() - started >= 31_200,
      "动作不得因原30秒超时提前结束选择流程",
    );
    assert.equal(
      plugin.messages.some(
        (message) =>
          request("ui.patch")(message) &&
          String(message.params.properties.status).includes("宿主请求超时"),
      ),
      false,
    );
    await plugin.stop();
  },
);

test("停止插件会拒绝仍等待用户的对话框而不遗留计时器或进程", async (t) => {
  const plugin = startPlugin("file-playback", (message) =>
    message.method === "dialogs.openMedia" ? new Promise(() => {}) : null,
  );
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  plugin.send({ type: "action", name: "open", value: 0 });
  await plugin.waitFor(request("dialogs.openMedia"));
  const after = plugin.messages.length;
  await plugin.stop();
  assert.equal(plugin.diagnostics(), "", "正常停止不得误报操作失败堆栈");
  assert.equal(
    plugin.messages.slice(after).some(request("ui.patch")),
    false,
    "transport 停止后不得再尝试更新错误界面",
  );
});

test("等待对话框期间的播放状态快照合并为最新值，不回放过期进度", async (t) => {
  let release: ((value: null) => void) | undefined;
  const plugin = startPlugin("file-playback", (message) =>
    message.method === "dialogs.openMedia"
      ? new Promise<null>((resolve) => {
          release = resolve;
        })
      : null,
  );
  t.after(() => plugin.kill());
  await plugin.waitFor((message) => message.type === "ready");
  plugin.send({ type: "action", name: "open", value: 0 });
  await plugin.waitFor(request("dialogs.openMedia"));
  const after = plugin.messages.length;
  for (let position = 1; position <= 100; position++)
    plugin.send({
      type: "playback",
      state: { ...initial, path: "/tmp/busy.mp4", paused: false, position },
    });
  plugin.send({ type: "action", name: "toggle-pause", value: 0 });
  assert.ok(release);
  release(null);
  await plugin.waitFor(request("player.togglePause"), after);
  const positions = plugin.messages
    .slice(after)
    .filter(
      (message) =>
        request("ui.patch")(message) &&
        typeof message.params.properties.position === "number" &&
        message.params.properties.position > 0,
    )
    .map((message) => message.params.properties.position);
  assert.deepEqual(
    positions,
    [100],
    "playback snapshots are current state, not a replay log",
  );
  await plugin.stop();
});
