# SDK v2 使用示例

设计说明见 [sdk-v2.md](sdk-v2.md)。每个示例都是完整的插件入口（或其中一个模块），只省略插件自己的业务函数（从 `./model.ts` 等导入）。所有示例都经过 `tsc` 严格检查；“画面调节”是 `src/plugins/video/` 的缩写版，完整代码见该目录。

阅读约定：

- `plugin.xxx.create/add/mount/filter(...)` 返回**句柄**；回调绑在句柄上，句柄随插件停用自动释放，不需要清理函数。
- `bind(() => ({...}))` 和命令的 `enabled: () => ...` 是“期望状态”：SDK 在每次处理完事件后重新求值，只把变化发给宿主。插件状态就是普通变量。
- 属性名、回调名用 camelCase，SDK 转成 `.slint` 里的 kebab-case。

## 1. 画面调节：连续控制 + 处理资源 + 工具窗口

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";
import { defaults, describe, nodesFor, presets } from "./model.ts";

export default async function activate(plugin: Plugin) {
  let picture = { ...defaults };
  let enabled = true;
  const filter = plugin.video.filter("picture"); // 本插件自己的视频处理链
  const window = plugin.windows.create("picture.slint");
  const apply = () => filter.set(enabled ? nodesFor(picture) : []);

  window.bind(() => ({
    ...picture,
    effectEnabled: enabled,
    status: describe(filter.status, plugin.player.state),
  }));
  window.on("adjust", (field: "brightness" | "contrast", value: number) => {
    picture[field] = value;
    apply(); // 拖动滑块时连续调用：SDK 只保留一个在途请求和最新值
  });
  window.on("preset", (name: string) => {
    picture = { ...(presets[name] ?? defaults) };
    apply();
  });
  window.on("toggleEffect", (on: boolean) => {
    enabled = on;
    apply(); // 空链 = 只撤销本插件的效果
  });
  window.on("retry", () => filter.retry());
  window.on("hideTool", () => window.hide());

  plugin.commands.add({ id: "video.picture", path: ["视频", "画面调节…"], run: () => window.show() });
  apply();
}
```

`filter.status` 是 `waiting`（没有就绪媒体，值已保留，就绪后自动应用）、`applying`、`applied` 或 `failed`（带 `error.message`）。修订号、来源代次、答复丢失后的查询、宿主阻塞（`backend-uncertain`）都在 SDK 里。

## 2. 基础播放栏：挂载到底栏、显示来源状态、拖动跳转

```ts
import type { Plugin, Scrub } from "../../sdk/plugin/index.ts";
import { time } from "./model.ts";

export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const bar = plugin.panels.mount("bottom", "ui/bar.slint"); // 组件与桥接 global 从文件中推断
  let scrub: Scrub | undefined;
  let draft: number | null = null; // 拖动中显示的位置

  bar.bind(() => {
    const { position, duration, phase, paused, can } = player.state;
    const shown = draft ?? position;
    return {
      timeLabel: `${time(shown)} / ${time(duration)}`,
      progress: duration && shown !== null ? shown / duration : 0,
      canSeek: can.seek && !!duration,
      pauseLabel: phase === "ended" ? "重播" : paused ? "播放" : "暂停",
    };
  });
  bar.on("dragging", (dragging: boolean) => {
    bar.setInteraction({ dragging }); // 拖动时底栏不自动隐藏
    if (dragging) scrub = player.scrub();
  });
  bar.on("seekChanged", async (fraction: number) => {
    const target = (draft = fraction * (player.state.duration ?? 0));
    // true：这个目标已生效；false：被更新的目标取代，或别人改变了来源（手势结束）
    if ((await scrub?.seek(target)) && draft === target) draft = null;
  });
}
```

`player.state` 总是最新且不会倒退（过期事件已被丢弃）。`scrub()` 是一个手势：同一手势里的每次跳转都以上一次跳转的结果为条件，期间任何外部操作（另一个插件打开文件、用户在菜单里暂停）都会让手势失效，不会把来源拖回旧位置。

## 3. 基础播放栏：命令启用状态随播放状态变化

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const bar = plugin.panels.mount("bottom", "ui/bar.slint");

  plugin.commands.add({
    id: "toggle-pause",
    path: ["播放", "播放/暂停"],
    shortcut: "Space",
    enabled: () => player.state.can.pause || player.state.phase === "ended",
    checked: () => (player.state.phase === "ready" ? player.state.paused : null),
    // 以插件看到的状态为条件：若刚被别处改变，以 code "stale" 失败且什么都不做
    run: () => (player.state.phase === "ended" ? player.restart() : player.setPaused(!player.state.paused)),
  });
  plugin.commands.add({
    id: "open",
    path: ["文件", "打开媒体…"],
    run: async () => {
      const path = await plugin.dialogs.openFile({ title: "打开媒体" });
      if (path) await player.open(path);
    },
  });
  plugin.commands.add({
    id: "bar-visible",
    path: ["视图", "显示播放栏"],
    checked: () => bar.visibility?.preferred ?? true,
    run: () => bar.toggle(),
  });
  bar.on("volumeChanged", (volume: number) => void player.setOutput({ volume, muted: false }));
}
```

命令的 `enabled`/`checked`/`busy`/`reason` 可以是值或函数；函数在每个更新轮重新求值，变化才发给宿主（修订号由 SDK 维护）。`setOutput` 连续调用时只保留最新值，并保证先设音量再取消静音。

## 4. 播放列表：侧栏列表模型、选中、自动推进

```ts
import type { Disposable, Media, Plugin } from "../../sdk/plugin/index.ts";
import type { Row } from "./queue.ts";

export default async function activate(plugin: Plugin) {
  let rows: Row[] = [];
  let selected: string | null = null;
  let playing: { row: Row; media: Media; end: Disposable } | undefined;
  let message = "添加文件或链接；选择后点击播放";

  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint");
  list.bind(() => ({
    rows: rows.map((row, i) => ({ rowId: row.id, label: `${i + 1}. ${row.title}`, selected: row.id === selected })),
    message,
  }));
  list.on("selectRow", (id: string) => (selected = id));
  list.on("playRow", (id: string) => play(rows.find((row) => row.id === id)));

  async function play(row: Row | undefined, options = {}) {
    if (!row) return;
    await playing?.end.dispose();
    const media = await plugin.player.open(row.source, options);
    // 只在“播完之后没人改过来源”时推进；用户刚好另开了文件则以 stale 失败
    const end = media.onEnd((ended) => {
      const next = rows[rows.indexOf(row) + 1];
      if (next) void play(next, { ifCurrent: ended });
      else message = "列表已播完";
    });
    playing = { row, media, end };
  }
  plugin.player.onChange(() => {
    if (playing && !playing.media.current) {
      playing = undefined;
      message = "已切换到列表外来源";
    }
  });
}
```

`player.open` 返回 `Media`：`current` 表示它还是播放器的来源（重播仍算），`onEnd` 每次自然播完触发一次。插件停用时正在播放的媒体继续播放。

## 5. 截图导出：绑定来源快照的任务、进度、取消

```ts
import type { ExportJob, Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("capture.slint");
  let job: ExportJob | undefined;
  let status = "";
  window.bind(() => ({ status, cancellable: !!job }));

  window.on("exportImage", async (processed: boolean) => {
    const seen = plugin.player.state; // 用户确认时看到的来源
    const destination = await plugin.dialogs.saveFile({ title: "保存当前画面", fileName: "capture.png" });
    if (!destination) return;
    // 选择保存位置期间来源若被改变，以 "stale" 失败：不会截到别的媒体
    const snapshot = await plugin.media.capture({ ifCurrent: seen, processed });
    try {
      job = await snapshot.export({ format: "png", destination });
      job.onChange((state) => (status = `导出 ${state.status}`));
      const done = await job.done;
      status = done.status === "completed" ? `已保存：${destination}` : `导出 ${done.status}`;
    } finally {
      job = undefined;
      await snapshot.dispose(); // 释放冻结的输入；也可以保留以便重试
    }
  });
  window.on("cancelTask", () => job?.cancel());
  plugin.onError((error) => (status = `导出未接受：${error.message}`));
  plugin.commands.add({ id: "capture.still", path: ["视频", "截图与导出", "保存当前画面…"], run: () => window.show() });
}
```

`processed: true` 时 SDK 冻结当前处理配置与字幕选择（并确认它们属于同一来源）。任务状态由 SDK 轮询，`done` 在完成/失败/取消时结束。

## 6. NES：插件生成的音视频输出与停止

```ts
import { readFile } from "node:fs/promises";
import type { Plugin, Stream } from "../../sdk/plugin/index.ts";
import { NesCore } from "./core.ts";

export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("nes.slint");
  let stream: Stream | undefined;
  let status = "Choose an iNES ROM to start.";
  window.bind(() => ({ status, running: !!stream?.running }));

  async function openRom() {
    const path = await plugin.dialogs.openFile({ title: "Open NES ROM" });
    if (!path) return;
    const core = new NesCore(await readFile(path));
    await stream?.stop();
    // SDK 负责预填充、激活、暂停与输入确认、按音频时钟节拍调用 frame()
    stream = await plugin.player.openStream({ width: 256, height: 240, frame: () => core.frame() });
    stream.onStop((error) => (status = error ? `NES stopped: ${error.message}` : "NES source replaced."));
    status = `Running ${path}`;
  }
  const stop = async () => {
    await stream?.stop(); // 只关闭仍属于自己的来源，然后释放
    status = "Stopped.";
  };
  window.on("openRom", openRom);
  window.on("stop", stop);
  plugin.onError((error) => (status = error.message));
  plugin.commands.add({ id: "nes.open", path: ["模拟器", "打开 NES ROM…"], run: async () => { await window.show(); await openRom(); } });
  plugin.commands.add({ id: "nes.stop", path: ["模拟器", "停止 NES"], run: stop });
}
```

`frame()` 返回 `{ video: RGBA 像素, audio: 48 kHz 立体声 f32 }`，不会被并发调用，暂停时不会被调用。

## 7. 字幕：外部文件载入、选择、样式

```ts
import { readFile } from "node:fs/promises";
import type { Plugin, SubtitlesState } from "../../sdk/plugin/index.ts";
import { styleWith } from "./model.ts";

export default async function activate(plugin: Plugin) {
  const { subtitles } = plugin;
  const window = plugin.windows.create("ui/workbench.slint");
  let status = "选择字幕轨或加载 UTF-8 SRT。";
  window.bind(() => ({
    statusText: status,
    trackLabels: ["关闭字幕", ...(subtitles.state?.tracks.map((t) => t.title) ?? [])],
    canRemove: !!subtitles.state?.selected?.owned,
  }));

  window.on("loadFile", async () => {
    const path = await plugin.dialogs.openFile({ title: "加载 UTF-8 SRT 字幕" });
    if (!path) return void (status = "已取消选择字幕文件。");
    const track = await subtitles.load(await readFile(path)); // 分块上传、校验、提交并选中
    status = `SRT 已加载：${track.title}`;
  });
  window.on("selectTrack", (index: number) =>
    subtitles.select(index === 0 ? null : (subtitles.state?.tracks[index - 1] ?? null)),
  );
  let editing: SubtitlesState | null = null;
  window.on("changeSize", (fontSize: number) => {
    editing ??= subtitles.state; // 只作用于开始调整时选中的轨
    void subtitles.setStyle(styleWith({ fontSize }), editing ? { ifCurrent: editing } : {});
  });
  plugin.onError((error) => (status = error.message));
  plugin.commands.add({ id: "workbench", path: ["字幕", "字幕工作台…"], run: () => window.show() });
}
```

`subtitles.state` 在来源变化与后端事件后由 SDK 刷新，操作的结果状态也会立即生效；比较字幕轨用 `track.id`，内嵌轨的轨道号是 `track.embeddedId`，`track.owned` 表示可由本插件移除（`subtitles.remove(track)`）。`load(bytes, { signal })` 可在上传途中取消（丢弃上传、不提交）。

## 8. 音频工作台：带延迟的音频处理、按音轨复位

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";
import { neutral, nodesFor } from "./model.ts";

export default async function activate(plugin: Plugin) {
  let settings = neutral();
  let track = "";
  const filter = plugin.audio.filter("workbench");
  const apply = () => filter.set({ nodes: nodesFor(settings), delayMs: settings.delayMs });

  // 设置属于一个来源和一条音轨：任一变化就复位（从未应用过时不会发请求）
  plugin.player.onChange((state) => {
    const current = `${state.sourceId}:${state.audioTracks?.filter((t) => t.selected).map((t) => t.id)}`;
    if (current === track) return;
    track = current;
    settings = neutral();
    apply();
  });
  const ready = () => plugin.player.state.phase === "ready";
  plugin.commands.add({
    id: "audio.reset",
    path: ["音频", "复位音频处理"],
    enabled: ready,
    busy: () => filter.status.state === "applying",
    reason: () => (ready() ? null : "没有选中的音轨"),
    run: () => {
      settings = neutral();
      apply();
    },
  });
}
```

`sourceId` 在每次打开来源时变化（即使是同一文件），用于判断“是不是另一次载入”。

## 9. 网络媒体：解析器、打开、等待就绪、取消（Bilibili / YouTube）

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";
import { extract } from "./extractor.ts";

export default async function activate(plugin: Plugin) {
  plugin.resolvers.add({
    id: "bilibili",
    schemes: ["https"],
    hosts: ["www.bilibili.com", "b23.tv"],
    resolve: async (url, signal) => {
      const found = await extract(url, signal);
      return { kind: "media", originalUrl: url, title: found.title, primary: { url: found.video }, audio: { url: found.audio } };
    },
  });
  const window = plugin.windows.create("dialog.slint");
  let running: AbortController | undefined;
  let status = "输入 Bilibili 视频链接。";
  window.bind(() => ({ status, busy: !!running }));
  window.on("play", async (url: string) => {
    running = new AbortController();
    const signal = AbortSignal.any([running.signal, AbortSignal.timeout(60_000)]);
    try {
      status = "正在解析视频和音频…";
      const media = await plugin.player.open(url, { signal }); // 由上面的解析器解析
      await media.ready({ signal }); // 中止时 SDK 关闭这个媒体（若仍是当前来源）
      status = "视频已就绪。";
    } catch (error) {
      status = (error as Error).name === "AbortError" ? "已取消。" : (error as Error).message;
    } finally {
      running = undefined;
    }
  });
  window.on("cancel", () => running?.abort());
}
```

`media.ready()` 失败时 `code` 为 `failed`（载入失败，`message` 为后端原因）、`replaced`（被别的来源替换）或 `ended`。

## 10. 插件工坊：校验、试载、安装

```ts
import type { PackageCandidate, PackageTrial, Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("workshop.slint");
  let candidate: PackageCandidate | undefined;
  let trial: PackageTrial | undefined;
  let status = "选择工作区并输入需求后生成";
  window.bind(() => ({ statusText: status, canTrial: !!candidate && !trial, canStop: !!trial }));

  window.on("validate", async (directory: string) => {
    candidate = await plugin.packages.validate(directory);
    status = `候选已通过宿主校验 ${candidate.digest}`;
  });
  window.on("startTrial", async () => {
    trial = await candidate!.trial(); // 等到试载启动完成
    status = `试载：${trial.status}`;
  });
  window.on("stopTrial", async () => {
    await trial?.stop(); // 等宿主确认清理
    trial = undefined;
    status = "试载已清理";
  });
  window.on("install", async () => {
    const change = await candidate!.install(); // 预期修订号与请求 id 由 SDK 处理，从不重发
    change.onChange((next) => (status = `包操作：${next}`));
    status = `包操作：${await change.done}`;
  });
  plugin.onError((error) => (status = `错误：${error.message}`));
}
```

## 11. 拆分模块、子作用域与手动更新

```ts
import type { Plugin, Scope, Window } from "../../sdk/plugin/index.ts";

/** 功能模块只拿到它需要的作用域或句柄，而不是整个插件。 */
function linkDialog(scope: Scope, add: (url: string) => void): Window {
  const window = scope.windows.create("ui/link.slint");
  let message = "输入 HTTP(S) 链接；添加只选择，不自动播放";
  window.bind(() => ({ message }));
  window.on("addLink", (url: string) => {
    add(url);
    message = "链接已添加";
    void window.hide();
  });
  return window;
}

export default async function activate(plugin: Plugin) {
  const links: string[] = [];
  const feature = plugin.scope(); // feature.dispose() 一次释放下面创建的全部句柄
  const dialog = linkDialog(feature, (url) => links.push(url));
  feature.commands.add({ id: "playlist.link", path: ["文件", "播放列表", "添加链接…"], run: () => dialog.show() });

  const timer = setInterval(() => plugin.update(), 1500); // 在 SDK 处理函数之外改了状态时手动触发更新
  plugin.onDispose(() => clearInterval(timer));
}
```

## 12. 测试

```ts
import assert from "node:assert/strict";
import test from "node:test";
import { HostError, startPlugin } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

test("a refused change is shown and can be retried", async () => {
  const { host, stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url) });
  host.openElsewhere("/movie.mp4"); // 另一个插件打开了媒体
  await host.command("video.picture").run();
  host.handle("processing.replace", () => {
    throw new HostError("unsupported", "no video stream");
  });
  host.window("picture").emit("adjust", "gamma", 20);
  await host.settle();
  assert.match(String(host.window("picture").props.status), /^画面处理失败/);
  host.reset("processing.replace");
  host.window("picture").emit("retry");
  await host.settle();
  assert.equal(host.window("picture").props.status, "画面设置已接受");
  await stop();
});
```

`startPlugin` 读取插件目录的 `manifest.json`，用内存中的 `TestHost` 运行真实的 SDK 逻辑；更多控制见 [sdk-v2.md](sdk-v2.md) 第 10 节。
