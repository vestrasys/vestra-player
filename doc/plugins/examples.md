# 插件示例

按功能分类的完整示例，适合已经读过[入门教程](tutorial.md)、想照着实现某种功能时查阅。每个方法的精确定义见 [API 参考](reference.md)，设计取舍见 [SDK 设计说明](../architecture/sdk-design.md)。

每个示例都是完整的插件入口（或其中一个模块），只省略插件自己的业务函数（从 `./model.ts` 等导入）。所有示例都经过 `tsc` 严格检查；“画面调节”是 `src/plugins/video/` 的缩写版，完整代码见该目录。

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
    order: 263,
    bindings: ["Space"],
    enabled: () => player.state.can.pause || player.state.phase === "ended",
    checked: () => (player.state.phase === "ready" ? player.state.paused : null),
    // 以插件看到的状态为条件：若刚被别处改变，以 code "stale" 失败且什么都不做
    run: () => (player.state.phase === "ended" ? player.restart() : player.setPaused(!player.state.paused)),
  });
  plugin.commands.add({
    id: "open",
    path: ["文件", "打开媒体…"],
    order: 202,
    run: async () => {
      const path = await plugin.dialogs.openFile({ title: "打开媒体" });
      if (path) await player.open(path);
    },
  });
  plugin.commands.add({
    id: "bar-visible",
    path: ["视图", "显示播放栏"],
    order: 502,
    checked: () => plugin.panels.bottom.state?.preferred ?? true,
    run: () => plugin.panels.bottom.toggle(),
  });
  bar.on("volumeChanged", (volume: number) => void player.setOutput({ volume, muted: false }));
}
```

`order` 决定菜单里的位置（同级升序，子菜单跟着它最靠前的命令），`bindings` 是默认快捷键，菜单里的快捷键文字由它生成。命令的 `enabled`/`checked`/`busy`/`reason` 可以是值或函数；函数在每个更新轮重新求值，变化才发给宿主（修订号由 SDK 维护）。`setOutput` 连续调用时只保留最新值，并保证先设音量再取消静音。

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

`startPlugin` 读取插件目录的 `manifest.json`，用内存中的 `TestHost` 运行真实的 SDK 逻辑；更多控制见 [API 参考 §14](reference.md#14-测试工具)。

## 13. 菜单结构与快捷键：顶层命令、分组、单选、按住重复、不进菜单的命令

完整代码见 `src/examples/core-ui/`。

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  plugin.commands.add({ id: "open", path: ["打开文件…"], group: "open", order: 100,
    bindings: ["Ctrl+O", "F3"], run: () => openFile() });          // 顶层直接命令
  for (const [i, rate] of [0.5, 1, 2].entries())
    plugin.commands.add({ id: `rate-${rate}`, path: ["播放", "速度", `${rate}倍`], order: 230 + i,
      radio: true, checked: () => player.state.rate === rate, run: () => player.setRate(rate) });
  plugin.commands.add({ id: "volume-up", path: ["声音", "音量 +"], bindings: ["ArrowUp", "Mouse:WheelUp"],
    repeat: true, run: () => player.setOutput({ volume: Math.min(200, player.state.volume + 5) }) });
  plugin.commands.add({ id: "select-all", title: "全选", focus: "panel", bindings: ["Ctrl+A"],
    run: () => selectAll() });                                      // 不进主菜单，只在面板有焦点时
}
```

相邻两项 `group` 不同时菜单画分隔线；推荐的顶层分组与编号段见 [API 参考 §4](reference.md#4-命令-plugincommands)。同一输入被两个命令声明时先加载的插件生效，另一个写日志（测试里看 `host.conflicts`）。

## 14. 屏幕提示与信息层

播放信息插件 `src/plugins/info/` 的核心：

```ts
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();                 // 视频左上角的只读文字块
  let shown = false, timer: ReturnType<typeof setInterval> | undefined;
  const render = async () => {
    const stats = await plugin.player.stats();      // 编码、分辨率、帧数、丢帧……
    if (shown) panel.set(`${stats.video?.codec ?? "-"} · 帧 ${stats.framesShown}`);
  };
  plugin.commands.add({ id: "info.toggle", path: ["视图", "播放信息"], order: 503, bindings: ["Tab"],
    checked: () => shown,
    run: () => {
      shown = !shown; clearInterval(timer);
      if (!shown) return panel.set(null);
      void render(); timer = setInterval(() => void render(), 1000);
    } });
  plugin.onDispose(() => clearInterval(timer));
}
```

一次性的操作反馈用 `await plugin.osd.show("音量: 85%")`：一行，2 秒后淡出，新的替换旧的。两者都需要能力 `ui.osd`。

## 15. 播放序列：一个插件提供「下一个/上一个」，任何插件调用

```ts
// 播放列表：登记为提供者（同时只能有一个）
plugin.player.provideSequence({
  canNext: () => !!queue.neighbor(1),
  canPrevious: () => !!queue.neighbor(-1),
  next: () => playNeighbor(1),
  previous: () => playNeighbor(-1),
});

// 任何插件（例如底栏的右键「下一个」、PgDn）
plugin.commands.add({ id: "next", path: ["播放", "下一个"], bindings: ["PageDown", "Mouse:X2"],
  enabled: () => plugin.player.state.can.next, run: () => plugin.player.next() });
```

插件之间不直接调用对方的命令；`next()` 由播放器转给提供者。没有提供者时 `can.next` 为 `false`，调用以 `unavailable` 失败。

## 16. 侧栏：任何插件开关、分离为浮动窗口；面板右键菜单

```ts
// 任何插件：读取与切换侧栏（不需要知道是谁挂的）
const sidebar = plugin.panels.sidebar;
bar.bind(() => ({ listShown: sidebar.state?.preferred ?? false }));
bar.on("toggleList", () => sidebar.toggle());
bar.on("detachList", () => sidebar.setDetached(!sidebar.state?.detached));

// 挂载侧栏的插件：title 是浮动窗口标题；右键弹出本插件命令
const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "播放列表" });
const play = plugin.commands.add({ id: "play", title: "播放", focus: "panel", bindings: ["Enter"], run: playSelected });
const remove = plugin.commands.add({ id: "remove", title: "移除所选", focus: "panel", bindings: ["Delete"], run: removeSelected });
list.on("contextMenu", (x: number, y: number) => list.showMenu([play, null, remove], { x, y }));
```

```slint
// ui/sidebar.slint 中，右键位置换成窗口坐标交给插件
TouchArea {
    pointer-event(event) => {
        if event.button == PointerEventButton.right && event.kind == PointerEventKind.down {
            Bridge.context-menu((self.absolute-position.x + self.mouse-x) / 1px,
                                (self.absolute-position.y + self.mouse-y) / 1px);
        }
    }
}
```

分离后浮动窗口里是同一个面板组件，属性和回调不变。完整示例见 `src/examples/panel-menu/`（面板菜单、面板焦点按键、序列提供者）和 `src/examples/core-ui/`（侧栏开关与分离）。

## 17. 主题与控件

```slint
import { Theme } from "@vestra/theme.slint";
import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint";

export component Picture inherits Window {
    width: 301px; background: Theme.bg-panel;
    in-out property <float> brightness;
    callback reset();
    HorizontalLayout {
        padding: Theme.dialog-padding; spacing: 8px;
        ToolTipArea { text: "−100 到 100"; width: 65px;
            Text { text: "亮度"; color: Theme.text; font-size: Theme.font-body; } }
        Slider { panel: true; minimum: -1; maximum: 1; value <=> root.brightness; }
        Button { text: "默认"; small: true; clicked => { root.reset(); } }
    }
}
```

不改代码的插件也会变成深色：播放器用 `fluent-dark` 风格编译所有插件 Slint。新写的界面用 `Theme` 的令牌，不要写死颜色。

## 18. 外部工具：用 ffprobe 读文件信息

```ts
export default async function activate(plugin: Plugin) {
  plugin.commands.add({
    id: "probe", path: ["视频", "文件信息…"], order: 420,
    enabled: () => !!plugin.player.state.source && !plugin.player.state.source.includes("://"),
    async run() {
      if (!(await plugin.tools.available("ffprobe")))
        return plugin.osd.show("没有找到 ffprobe");
      const { stdout } = await plugin.tools.run(
        "ffprobe",
        ["-v", "error", "-print_format", "json", "-show_streams", plugin.player.state.source!],
        { timeoutMs: 15_000 },
      );
      const streams = JSON.parse(new TextDecoder().decode(stdout)).streams;
      await plugin.osd.show(`${streams.length} 路流`);
    },
  });
}
```

超时、`signal`、插件停用都会结束子进程；失败时 `code` 为 `timeout` / `failed`（`message` 是 stderr 最后一行）/ `unavailable`。测试里用 `host.tools.handle("ffprobe", (args) => ({ stdout: "{…}" }))` 注入结果。

## 19. 设置文件：读、改、自动保存

```ts
type Settings = { folder: string | null; step: number };
const isSettings = (v: unknown): v is Settings =>
  typeof (v as Settings)?.step === "number";

export default async function activate(plugin: Plugin) {
  const settings = plugin.storage.json<Settings>("settings", {
    defaults: { folder: null, step: 5 },
    validate: isSettings,
  });
  plugin.commands.add({
    id: "step-10", path: ["播放", "步长 10 秒"], order: 260, radio: true,
    checked: () => settings.value.step === 10,
    run: () => settings.save({ ...settings.value, step: 10 }),
  });
}
```

`save` 立即更新 `value`，300 ms 后写入（临时文件 + 改名）；插件停用前自动写完。读坏的文件改名为 `settings.json.bad-<时间戳>` 留给用户，插件拿到默认值。

## 20. 剪贴板、阻止休眠、在 Finder 中显示

```ts
await plugin.clipboard.writeText(basename(plugin.player.state.source!)); // 需要 clipboard
const link = await plugin.clipboard.readText();                        // 预填「打开链接」
await plugin.clipboard.writeImage("/Users/me/Pictures/shot.png");     // PNG/JPEG
await plugin.power.preventSleep("playing");                            // 需要 playback.control
await plugin.system.reveal("/Users/me/Movies/a.mkv");                  // 在 Finder 中选中
```

## 21. 核心动作：底栏按钮执行播放器自己的命令

```ts
export default async function activate(plugin: Plugin) {
  const bar = plugin.panels.mount("bottom", "ui/bar.slint");
  bar.on("playPause", () => plugin.host.run("play-pause"));   // 与 Space、菜单同一行为和提示
  bar.on("back", () => plugin.host.run("seek-backward"));
  bar.on("forward", () => plugin.host.run("seek-forward"));
  bar.on("mute", () => plugin.host.run("mute"));
  bar.on("stop", () => plugin.host.run("stop"));
  bar.bind(() => ({
    paused: plugin.player.state.paused,
    canPlay: plugin.player.state.phase !== "empty",           // 按钮状态仍从 player.state 推出
  }));
  await plugin.player.setStepSeek({ seconds: 10 });           // ← / → 一次 10 秒
}
```

命令本身由播放器注册（菜单、快捷键、用户改绑都归核心），插件停用后 Space、←/→ 照常可用。

## 22. 主窗口：按视频尺寸调整窗口、置顶

```ts
export default async function activate(plugin: Plugin) { // window.control、playback.read、commands、ui.osd
  for (const scale of [1, 2]) {
    plugin.commands.add({
      id: `size-${scale}`, path: ["屏幕", `${scale}倍`], order: 710 + scale,
      bindings: [scale === 1 ? "2" : "4"],
      enabled: () => !plugin.window.state.fullscreen && plugin.player.state.phase === "ready",
      reason: () => (plugin.window.state.fullscreen ? "全屏时不可用" : "没有视频"),
      async run() {
        const { video } = await plugin.player.stats();
        if (!video?.displayWidth || !video.displayHeight) return;
        const applied = await plugin.window.setVideoArea({
          width: video.displayWidth * scale,
          height: video.displayHeight * scale,
        });
        await plugin.osd.show(`窗口: ${scale}倍${applied.clamped ? "（已按屏幕缩小）" : ""}`);
      },
    });
  }
  await plugin.window.setAlwaysOnTop(true); // 插件停用时播放器自动取消
}
```

`window.state` 不需要能力，命令的启用条件可以直接读它；全屏时 `setVideoArea` 以 `unavailable` 失败。

## 23. 确认框：清空前询问，记住「不再询问」

```ts
const ask = plugin.storage.json("confirm", { defaults: { clear: true } });
plugin.commands.add({
  id: "clear", path: ["播放列表", "清空"], order: 840,
  async run() {
    if (ask.value.clear) {
      const { confirmed, checked } = await plugin.dialogs.confirm({
        message: "清空播放列表？", detail: "列表中的全部项目将被移除。",
        confirm: "清空", destructive: true, checkbox: "不再询问",
      });
      if (!confirmed) return;
      if (checked) ask.save({ clear: false });
    }
    queue.clear();
  },
});
```

测试里先 `host.dialogAnswers.push({ confirmed: true, checked: false })` 再执行命令；`host.confirms` 是发出的选项。

## 24. 章节与 A-B 循环

```ts
const { chapters } = await plugin.player.chapters();         // playback.read，按需读取
chapters.forEach((chapter, i) =>
  menu.commands.add({
    id: `chapter.${i}`, path: ["播放", "章节", chapter.title ?? `章节 ${i + 1}`],
    order: 260 + i, run: () => plugin.player.seek(chapter.start),
  }));
await plugin.player.setLoop({ a: 12.5, b: 30, count: 3 });   // playback.control
plugin.player.state.loop;                                    // { a: 12.5, b: 30, count: 3 }
await plugin.player.setLoop(null);                           // 取消；换文件时播放器也会清除
```

## 25. 快捷键设置：列出、改绑、冲突、恢复

```ts
export default async function activate(plugin: Plugin) { // bindings.manage、ui.window
  const keys = plugin.windows.create("ui/keys.slint");
  let rows: Json[] = [];
  const reload = async () => {
    const { commands } = await plugin.bindings.list();
    rows = commands.map((c) => ({
      plugin: c.plugin, command: c.command, title: c.title,
      keys: c.labels.join(", "),
      conflict: c.shadowed.map((s) => `${s.input} 被 ${s.by.plugin}/${s.by.command} 占用`).join("；"),
    }));
  };
  keys.bind(() => ({ rows }));
  plugin.bindings.onChange(reload);                         // 插件加载、停用、改绑后
  keys.on("rebind", (plugin_: string, command: string, input: string) =>
    plugin.bindings.set({ plugin: plugin_, command }, [input])); // "host" 是播放器自己的命令
  keys.on("unbind", (plugin_: string, command: string) =>
    plugin.bindings.set({ plugin: plugin_, command }, []));
  keys.on("resetAll", () => plugin.bindings.reset());
  await reload();
}
```

`set` 写入用户覆盖层（跨启动保存），立即生效并优先于所有默认绑定；输入写法同 §4.1，可以是 `Mouse:LeftClick`。测试里 `host.bindings.overlay` 是覆盖层，`host.press` 按它匹配。

## 26. 插件管理与音频输出

```ts
const list = await plugin.packages.list();                 // 不需要能力
plugin.packages.onChange((plugins) => render(plugins));
await plugin.packages.setEnabled("vestra.video", false);    // plugins.manage；重启后仍停用
const change = await plugin.packages.uninstall("community.counter", { removeData: true });
await change.done;                                         // "committed"
const devices = await plugin.player.audioDevices();         // [{ id: "coreaudio/…", name: "耳机" }]
await plugin.player.setAudioDevice(devices[1]!.id);         // 拔出时回到系统默认，插回后切回
plugin.player.state.audioOutput;                            // { requested, active }
```

## 27. 运行时图片：封面显示在背景层

```ts
// ui/art.slint：export global Art { in property <image> cover; in property <string> caption; }
//              export component Backdrop inherits Rectangle { Image { source: Art.cover; … } }
export default async function activate(plugin: Plugin) { // ui.mount、playback.read、events
  const art = plugin.panels.mount("backdrop", "ui/art.slint"); // 没有视频帧时画在视频区
  const logo = plugin.ui.image(fileURLToPath(new URL("logo.png", import.meta.url))); // 包内文件也可以（固定的图也可在 Slint 里 @image-url）
  let cover: ImageRef = logo;
  art.bind(() => ({ cover, caption: plugin.player.state.source ?? "Vestra" }));
  plugin.player.onChange(async (state) => {
    const file = join(plugin.dataDirectory, "covers", `${state.sourceId}.jpg`);
    cover = (await extractCover(state.source, file)) ? plugin.ui.image(file) : logo; // 文件变了就重新调用
  });
  plugin.panels.backdrop.onChange((s) => console.log("背景层显示：", s?.effective));
  art.ready.catch(() => {}); // 第二个背景层：conflict
}
```

图片只能作为属性的整个值（不能放进结构体或列表）；文件必须是 PNG/JPEG，在本插件的数据目录或包目录下。读取失败（`invalid` / `missing`）交给 `onError`，属性保持原值。

## 28. 时间轴标记：在进度条上发布书签

```ts
export default async function activate(plugin: Plugin) { // timeline.marks、playback.read、events
  const layer = plugin.timeline.layer();                   // 每个插件一层
  const publish = async () => {
    const seen = plugin.player.state;
    const { chapters } = await plugin.player.chapters();
    layer.set([
      ...chapters.map((c) => ({ time: c.start, kind: "chapter" as const, title: c.title ?? undefined })),
      ...bookmarksOf(seen.source).map((b) => ({ time: b.time, kind: "bookmark" as const, title: b.name })),
    ].slice(0, 1000), { ifCurrent: seen });                // 期间换了来源：丢弃
  };
  plugin.player.onChange((state) => { if (state.phase === "ready") void publish(); });
  // 读取方（底栏）：plugin.timeline.marks / onChange，按 kind 配色
}
```

## 29. 输入链接组件 `UrlInputRow`

```slint
import { Theme } from "@vestra/theme.slint";
import { UrlInputRow } from "@vestra/widgets.slint";
export component OpenUrl inherits Window {
    in-out property <string> url;
    callback open(string);
    callback cancel();
    background: Theme.bg-bar;
    init => { row.focus-input(); }
    VerticalLayout {
        padding: Theme.dialog-padding;
        row := UrlInputRow { text <=> root.url; placeholder: "https://…"; accepted(text) => { root.open(text); } cancelled => { root.cancel(); } }
    }
}
```

## 30. 设置页：在「选项」窗口里放一页，菜单直接打开它

```ts
// ui/look.slint：export global Look { in-out property <string> theme; callback pick(string); }
//               export component LookPage inherits Rectangle { height: 120px; … }
export default async function activate(plugin: Plugin) { // ui.settings、ui.theme、commands、events
  const page = plugin.settings.page("ui/look.slint", {
    id: "appearance", category: "appearance", title: "外观", order: 10,
  });
  page.bind(() => ({ theme: plugin.appearance.theme }));       // 别的插件切换时也跟着变
  page.on("pick", (theme: "vestra" | "classic") => plugin.appearance.set(theme));
  plugin.commands.add({
    id: "options", path: ["设置", "选项…"], order: 830, bindings: ["F5"],
    run: () => plugin.settings.show(),                          // 上次的页面
  });
  plugin.commands.add({
    id: "look", path: ["设置", "外观…"], order: 831,
    run: () => plugin.settings.show("appearance"),              // 本插件的这一页
  });
}
```

页面宽 440；插件停用时这一页从窗口里消失，启用后随注册回来。

## 31. 拖放：视频区按扩展名接收，列表面板接收落点

```ts
export default async function activate(plugin: Plugin) { // ui.drop、ui.mount、playback.control
  plugin.drops.provide({
    extensions: ["mkv", "mp4", "mp3", "flac", "m3u8", "folder"], // 小写、不带点；文件夹用 "folder"
    drop: async ({ paths, modifiers }) => {
      const added = await queue.add(paths);                   // 文件夹由自己展开
      if (!modifiers.alt) await plugin.player.open(added[0]!);  // 按住 Option 只加入
    },
  });
  const list = plugin.panels.mount("sidebar", "ui/sidebar.slint");
  list.onDrop(({ paths, at }) => {                           // 面板坐标：相对面板左上角
    const row = Math.floor((at.y - HEADER + scrollY) / ROW);  // scrollY 由面板回调回传
    return queue.insert(row, paths);                         // 插到落点所在行之前
  });
}
```

同一扩展名先登记的插件生效；没有插件登记的文件拖进来什么也不发生。

## 32. 倍帧：按原帧率决定是否加 `frameBlend`

```ts
export default async function activate(plugin: Plugin) { // processing.video、playback.read、commands、ui.osd
  const smooth = plugin.video.filter("smooth");
  let on = false;
  let sourceFps: number | null = null;                        // 打开倍帧之前读到的原帧率
  const apply = async () => {
    if (!on) return smooth.set([]);
    sourceFps ??= (await plugin.player.stats()).video?.fps ?? null;
    if (sourceFps === null || sourceFps >= 50) return smooth.set([]); // 已经够流畅：不加节点
    smooth.set([{ kind: "frameBlend", fps: Math.min(Math.round(sourceFps * 2), 60) }]);
  };
  plugin.commands.add({
    id: "blend", path: ["画面", "倍帧"], checked: () => on,
    run: () => { on = !on; void apply(); },
  });
  smooth.onStatus((s) => {
    if (s.state === "failed") void plugin.osd.show(`倍帧不可用：${s.error.message}`);
  });
}
```

整条视频图里只能有一个 `frameBlend`（别的插件已经加了时以 `invalid` 失败），播放器总把它放在所有效果之后。打开后 `stats().video.fps` 读到的是新帧率，所以原帧率要在打开之前读（换来源时把 `sourceFps` 清空，先 `set([])` 再读）。
