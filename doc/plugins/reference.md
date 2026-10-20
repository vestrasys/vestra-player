# 插件 API 参考

本文是插件 API 的权威规格：每个模块、方法的签名、参数、返回值、错误和使用约束。入门请先读 [入门教程](tutorial.md)，按功能找写法看 [插件示例](examples.md)，设计取舍见 [SDK 设计说明](../architecture/sdk-design.md)。

类型定义的源头是 `src/sdk/plugin/types.ts`。本文与它不一致时以代码为准，并应修正本文。

- 协议版本：`vestra.experimental/0.1`
- 运行环境：每个插件一个 Node ≥ 22.18 进程，可以使用 Node 标准库、文件系统、网络和 npm 依赖。插件是受信任的，manifest 能力是契约声明，不是安全沙箱。

## 1. 概览

### 1.1 入口

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  // 创建句柄、绑定回调、声明命令
}
```

- 入口模块必须**默认导出** `activate`。命名导出的 `activate` 会被拒绝（`runtime.failed`）。
- 插件只能 `import type` SDK。`plugin` 对象由运行器提供，打包后的插件不依赖本仓库代码（构建工具会检查）。
- 插件不返回清理函数，停用时的清理由 SDK 和播放器完成（§3）。

### 1.2 句柄

`plugin.commands.add`、`windows.create`、`panels.mount`、`video.filter`、`player.open`、`media.capture` 等返回**句柄**：代表一个资源的对象，后续操作和回调都在句柄上。所有句柄都实现 `Disposable`：

```ts
interface Disposable { dispose(): Promise<void> }
```

句柄属于创建它的作用域，作用域释放时自动释放（§3）。重复 `dispose()` 无害。

### 1.3 更新轮

`bind(() => props)` 和命令选项里的函数（`enabled: () => …`）是**绑定**。SDK 在下列时机执行一轮更新：重新求值所有绑定，只把变化的值发给播放器。

- SDK 调用的任何插件处理函数（命令 `run`、窗口和面板回调、`onChange`、`onStatus`、`onEnd`、`onStop` 等）同步返回之后，以及它返回的 promise 结束之后；
- 播放器状态、字幕状态、面板可见性、滤镜状态、导出任务状态变化之后；
- 插件调用 `plugin.update()` 时。

同一微任务内的多次请求合并为一轮。每个目标同一时间只有一个请求在途，期间的变化合并为下一次发送。

### 1.4 名称转换

属性名和回调名在 TS 中可写 camelCase，SDK 转为 Slint 的 kebab-case：`canMark` → `can-mark`，`toggleEffect` → `toggle-effect`。规则是「每个大写字母变成 `-` 加小写」，已是 kebab-case 的名字原样使用。结构体字段（如列表行的 `rowId`）不转换。

## 2. manifest 与能力

```json
{
  "id": "community.bookmark",
  "version": "0.1.0",
  "apiVersion": "vestra.experimental/0.1",
  "entry": "main.ts",
  "capabilities": ["commands", "ui.window"]
}
```

| 字段 | 规则 |
| --- | --- |
| `id` | 非空且合法的标识，全局唯一；建议 `作者.插件名` |
| `version` | 三段非负整数 `x.y.z`；构建工具写入 `dist/manifest.json` |
| `apiVersion` | 固定 `vestra.experimental/0.1` |
| `entry` | 包内相对路径；开发时可为 `.ts`，构建后为 `.js` |
| `capabilities` | 只能是下表中的值 |

未知字段、未知能力、越出包目录的路径（含符号链接）都会被拒绝。

调用未声明能力的功能，以 `code: "capability"` 失败。

| capability | 开放的 API |
| --- | --- |
| `commands` | `commands.add` |
| `ui.window` | `windows.create` |
| `ui.mount` | `panels.mount` |
| `ui.dialog` | `dialogs.*` |
| `playback.read` | `player.state` 启动时读取、`player.refresh`；滤镜读取处理状态 |
| `events` | `player.state` / `onChange` 实时更新、面板可见性、字幕状态刷新 |
| `playback.control` | `player.open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput` |
| `producer.av` | `player.openStream` |
| `processing.video` | `video.filter`（另需 `playback.read`） |
| `processing.audio` | `audio.filter`（另需 `playback.read`） |
| `subtitles.read` | `subtitles.state`、`onChange`、`refresh` |
| `subtitles.control` | `subtitles.select`、`load`、`remove`、`setStyle` |
| `media.read` / `media.control` | `media.capture`、`Snapshot.export`、`ExportJob`（`processed` 截图另需 `playback.read`、`subtitles.read`） |
| `source.resolve` | `resolvers.add` |
| `plugins.manage` | `packages.*` |

没有 `events` 时，`player.state` 只在启动和操作完成后更新，`onChange` 不会因外部变化触发。

## 3. 生命周期与作用域

### 3.1 激活

1. 播放器为插件启动一个 Node 进程，运行器加载入口模块。
2. 有 `playback.read` 时先读一次播放状态，有 `events` 时订阅状态事件。
3. 调用 `activate(plugin)`。
4. 等待激活期间发起的所有注册（命令、面板、解析器、事件订阅）成功，然后报告就绪。

`activate` 抛错，或任一注册失败，插件激活失败。插件代码里不需要 `await` 这些注册。激活之后再创建的句柄注册失败时，交给所在作用域的 `onError`。

### 3.2 停用

播放器先撤销该插件在播放器侧的全部资源：菜单、窗口、面板、滤镜、字幕层、快照、导出任务、解析器、音视频流。然后运行器拒绝新请求，挂起的请求以 `code: "disabled"` 失败，释放根作用域（只做本地清理：计时器、中止信号、套接字）。300 ms 内未退出的进程会被结束。

插件停用时，它用 `player.open` 打开的媒体**继续播放**。

### 3.3 `Scope`

`Plugin` 是根作用域，`plugin.scope()` 创建子作用域。两者有完全相同的模块（`commands`、`windows`、`player`…）；通过子作用域创建的句柄只属于子作用域。

```ts
interface Scope extends Disposable {
  readonly commands, windows, panels, dialogs, player, video, audio,
           subtitles, media, resolvers, packages;
  scope(): Scope;
  add<T extends Disposable>(item: T): T;
  onDispose(cleanup: () => void | Promise<void>): Disposable;
  onError(listener: (error: PluginError) => void): Disposable;
  update(): void;
}
interface Plugin extends Scope { readonly id: string }   // manifest id
```

| 成员 | 说明 |
| --- | --- |
| `scope()` | 子作用域。`child.dispose()` 按创建的逆序释放其中所有句柄和更深的子作用域 |
| `add(item)` | 把一个已有的 `Disposable` 交给本作用域管理，返回原对象 |
| `onDispose(fn)` | 作用域释放时调用 `fn`（如 `clearInterval`）；返回的句柄可提前取消 |
| `onError(fn)` | 接收本作用域内处理函数抛出的错误（包括 async 函数的 reject）。没有监听时交给父作用域，最终写入插件日志 |
| `update()` | 立即安排一轮更新（§1.3）。在 SDK 调用之外（自己的计时器、长任务中途）改了状态时调用 |
| `dispose()` | 释放本作用域。释放从不抛错，失败只写日志 |

各句柄释放时做什么：

| 句柄 | 释放效果 |
| --- | --- |
| `Command` | 不再响应；菜单项保留但禁用（原因「已停用」） |
| `Window` | 关闭窗口 |
| `Panel` | 卸载面板 |
| `VideoFilter` / `AudioFilter` | 若应用过，替换为空链（只撤销自己的效果） |
| `Snapshot` | 释放冻结的输入 |
| `ExportJob` | 未结束则取消 |
| `Stream` | 停止生成；若仍是当前来源则关闭 |
| `Resolver` | 注销并中止进行中的解析 |
| `PackageTrial` | 停止试载并等待清理确认 |
| 监听（`on`、`bind`、`onChange`…） | 退订 |
| `Media` | **不**关闭媒体；需要时显式 `media.close()` |

## 4. 命令 `plugin.commands`

需要 `commands`。

```ts
commands.add(options: CommandOptions): Command

interface CommandOptions {
  id: string;                         // 插件内唯一
  path: string[];                     // 菜单路径，最后一项是菜单项
  title?: string;                     // 菜单文字，默认取 path 最后一项
  shortcut?: string;
  enabled?: Bindable<boolean>;        // 默认 true
  checked?: Bindable<boolean | null>; // null：无勾选标记（默认）
  busy?: Bindable<boolean>;           // 默认 false
  reason?: Bindable<string | null>;   // 禁用或忙碌时显示的原因
  run(): unknown;
}
type Bindable<T> = T | (() => T);

interface Command extends Disposable {
  readonly id: string;
  set(state: Partial<{ enabled; checked; busy; reason }>): void; // 值或函数，最后写入生效
  readonly ready: Promise<void>;  // 播放器已注册该命令
}
```

- `path` 中同名的上级分组与其他插件合并，组成共享的右键菜单。顶层分组惯例：文件、播放、视频、音频、字幕、视图、模拟器、插件。
- `shortcut`：`Space`、`ArrowLeft`、`ArrowRight` 会在视频画面获得焦点时触发命令；其他值只在菜单中显示。
- 状态字段为函数时，每轮更新重新求值，变化才发送。
- 命令禁用或忙碌时，播放器不会调用 `run`。`run` 抛出的错误交给 `onError`。

## 5. 播放器 `plugin.player`

### 5.1 状态

```ts
interface Player {
  readonly state: PlayerState;     // 最新状态；不会倒退（过期事件已丢弃）
  onChange(listener: (state: PlayerState) => void): Disposable;
  refresh(): Promise<PlayerState>; // 主动从播放器读取
  isCurrent(state: PlayerState): boolean; // 自 state 以来来源是否没被改变
  // 控制方法见 5.2
}

interface PlayerState {
  readonly sourceId: number;   // 每次打开来源都变化，即使是同一文件
  readonly kind: "media" | "stream";
  readonly phase: "empty" | "loading" | "ready" | "ended" | "error";
  readonly source: string | null;    // 路径或 URL
  readonly paused: boolean;
  readonly position: number | null;  // 秒
  readonly duration: number | null;  // 秒
  readonly rate: number;
  readonly volume: number;           // 0–200
  readonly muted: boolean;
  readonly endReason: "natural" | "stopped" | "replaced" | "unknown" | null;
  readonly error: string | null;     // phase 为 error 时的原因
  readonly can: { pause; seek; rate; restart; close; snapshot: boolean };
  readonly audioTracks: AudioTrack[] | null;
}

interface AudioTrack {
  id: number; selected: boolean;
  external: boolean;     // 外部音轨不能导出
  exportable: boolean;
  codec: string | null; title: string | null; language: string | null;
}
```

`PlayerState` 是不可变快照，可以保存下来，用作之后操作的条件（§5.3）。

### 5.2 控制

需要 `playback.control`（`openStream` 需要 `producer.av`）。

| 方法 | 说明 |
| --- | --- |
| `open(source, { paused?, signal?, ifCurrent? })` → `Promise<Media>` | 打开路径或 URL，替换当前来源。默认**无条件**。播放器接受加载后 resolve（不等就绪，就绪用 `media.ready()`） |
| `setPaused(paused, opts?)` | 暂停/继续 |
| `seek(seconds, opts?)` | 跳转 |
| `setRate(rate, opts?)` | 倍速 |
| `restart(opts?)` | 从头播放（播完后的「重播」） |
| `close(opts?)` | 关闭当前来源 |
| `scrub()` → `Scrub` | 开始一次拖动手势 |
| `setOutput({ volume?, muted? })` → `Promise<boolean>` | 设备音量（0–200）/静音。最后写入生效，先设音量再设静音；`true` 表示本次已生效，`false` 表示被更新的值取代 |
| `openStream(options)` → `Promise<Stream>` | 由插件生成画面和声音的来源（模拟器等） |

`opts` 为 `{ ifCurrent?: PlayerState }`。

**`Media`**：`open` 返回的、本插件打开的来源。

```ts
interface Media {
  readonly current: boolean;  // 仍是播放器的来源（重播仍算）
  ready(options?: { signal?: AbortSignal }): Promise<void>;
  onEnd(listener: (state: PlayerState) => void): Disposable; // 每次自然播完调用一次
  close(): Promise<void>;     // 仍是当前来源时关闭，否则什么都不做
}
```

`ready()` 在开始播放时 resolve，失败时以 `failed`（载入失败，`message` 为原因）、`replaced`（被别的来源替换）或 `ended` reject；`signal` 中止时先关闭这个媒体，再以 `AbortError` reject。

**`Scrub`**：一次拖动手势。手势内每次跳转都以上一次跳转的结果为条件，只保留最新目标；期间任何外部改变（另一插件打开文件、用户暂停）都会结束手势。

```ts
interface Scrub {
  seek(seconds: number): Promise<boolean>; // true：此目标已生效；false：被取代或手势已结束
  readonly active: boolean;
  end(): void;
}
```

**`openStream`**：

```ts
openStream(options: {
  width: number; height: number;
  frame(): { video: Uint8Array /* RGBA */; audio: Float32Array /* 48 kHz 立体声 */ };
  signal?: AbortSignal;
}): Promise<Stream>

interface Stream extends Disposable {
  readonly running: boolean;
  onStop(listener: (error: PluginError | null) => void): Disposable; // null：被替换或被关闭
  stop(): Promise<void>; // 停止生成；仍是当前来源时关闭
}
```

`frame()` 由 SDK 按音频时钟调用，不会并发调用，暂停时不调用。预填充、激活、暂停与输入确认都由 SDK 处理。

### 5.3 条件提交（`ifCurrent`）

**控制操作作用于插件看到的状态。**

- `setPaused`、`seek`、`setRate`、`restart`、`close`、`media.capture` 默认以当前的 `player.state` 为条件。如果在此之后别的插件或用户改变了来源（打开、暂停、跳转都算），调用以 `code: "stale"` 失败，且**什么都不做**。
- SDK 在自己的操作完成后刷新 `player.state`，所以本插件连续的操作不会互相判为过期。
- `player.open` 默认无条件（用户发起的打开就该替换当前来源）。

需要绑定到更早的状态时，保存 `PlayerState` 并传 `{ ifCurrent: state }`：

```ts
// 播完自动下一项：只在播完之后没人改过来源时推进
media.onEnd((ended) => player.open(next, { ifCurrent: ended }));

// 截图：冻结用户确认之前看到的来源
const seen = player.state;
const path = await plugin.dialogs.saveFile({ fileName: "a.png" });
await plugin.media.capture({ ifCurrent: seen });
```

只判断不提交时用 `player.isCurrent(state)`。

## 6. 窗口与面板

### 6.1 共同接口 `View`

```ts
interface View {
  on(callback: string, handler: (...args) => unknown): Disposable;
  set(props: Props): void;
  bind(props: () => Props): Disposable;
}
type Props = Record<string, Json>; // Json：null、布尔、数字、字符串、数组、对象
```

| 方法 | 说明 |
| --- | --- |
| `on(name, fn)` | 处理 Slint 回调，`fn` 收到回调参数。名字可写 camelCase |
| `bind(fn)` | 保持属性等于 `fn()` 的结果；每轮更新重新求值，**值变化时才发送**，所以用户在界面上改过的 in-out 属性不会被无关的更新覆盖 |
| `set(props)` | 立即写入；即使值与上次相同也发送一次，可用来把被用户改动的控件「拉回」模型值 |

写入失败时：绑定值在下次变化前不重发；`set` 的值退避重试（0.25 秒起翻倍，最长 30 秒）；连续失败只报告一次错误。

### 6.2 工具窗口 `plugin.windows`

需要 `ui.window`。

```ts
windows.create(file: string, options?: { id?: string }): Window

interface Window extends View, Disposable {
  show(): Promise<void>;  // 首次打开，或重新显示；并发调用共用一次打开
  hide(): Promise<void>;
  close(): Promise<void>; // 关闭；之后 show() 用最新属性重新打开
  readonly opened: boolean;
}
```

- `create` 不会打开窗口。第一次 `show()` 时，用当前绑定的属性作为初始值打开，不会先闪一下默认值。
- **回调必须在第一次 `show()` 之前用 `on` 绑定**，之后再绑定新的回调名会抛错。
- 窗口 id 默认取文件名（`ui/picture.slint` → `picture`）；同一插件用同一文件开多个窗口时，用 `options.id` 区分，id 重复会抛错。

### 6.3 面板 `plugin.panels`

需要 `ui.mount`。

```ts
panels.mount(slot: "bottom" | "sidebar", file: string,
             options?: { component?: string; global?: string }): Panel

interface Panel extends View, Disposable {
  readonly ready: Promise<void>;                 // 已挂载
  readonly visibility: PanelVisibility | null;
  onVisibility(listener: (v: PanelVisibility) => void): Disposable;
  setVisible(visible: boolean): Promise<void>;  // 设置用户偏好
  toggle(): Promise<void>;                       // 按播放器当前值翻转偏好
  setInteraction(locks: Partial<{ focused; dragging; popupOpen: boolean }>): void;
}

interface PanelVisibility {
  preferred: boolean;   // 用户的选择
  effective: boolean;   // 现在是否显示
  reason: "userHidden" | "space" | "fullscreen" | "idle" | null;
}
```

- 面板文件导出**一个 component 和一个 global**。属性和回调声明在 global 上（桥接对象），component 负责布局。名字由 SDK 从文件推断；文件导出多个时用 `options` 指定。
- `setInteraction`：用户正在操作面板（输入框聚焦、拖动、弹出菜单打开）时设为 `true`，防止底栏自动隐藏。合并写入，最后写入生效。
- 面板转发 global 上的所有回调，没有「先绑定再显示」的限制。

### 6.4 Slint 文件规则

- 工具窗口文件恰好导出一个组件（通常 `inherits Window`）；不带 `export` 的辅助组件不限。
- 插件 → 界面用 `in` / `in-out property`，界面 → 插件用 `callback`。回调参数和属性值必须能用 JSON 表示（基本类型、数组、结构体），回调返回 `void`。
- 会被插件改值的输入控件用双向绑定（`value <=> root.x`、`checked <=> root.x`）。单向的 `value: root.x` 在用户操作控件后会断开，之后插件改值，控件不会跟着动。属性须为 `in-out`。
- `in` 属性只能单向绑定时，用 `changed x => { slider.value = root.x; }` 在模型变化时回推。

## 7. 文件对话框 `plugin.dialogs`

需要 `ui.dialog`。系统原生对话框；没有时长限制，用户选择或取消才结束。

| 方法 | 返回 |
| --- | --- |
| `openFile({ title? })` | 路径；取消为 `null` |
| `openFiles({ title? })` | 路径数组；取消为 `[]` |
| `openDirectory({ title? })` | 路径；取消为 `null` |
| `saveFile({ title?, fileName? })` | 路径；取消为 `null` |

## 8. 视频与音频滤镜

需要 `processing.video` / `processing.audio`，以及 `playback.read`。

```ts
plugin.video.filter(id: string): VideoFilter  // Filter<VideoNode[]>
plugin.audio.filter(id: string): AudioFilter  // Filter<{ nodes: AudioNode[]; delayMs?: number }>

interface Filter<T> extends Disposable {
  set(value: T): void;          // 期望的处理链；最后写入生效；空链 = 撤销本插件的效果
  readonly status: FilterStatus;
  onStatus(listener: (s: FilterStatus) => void): Disposable;
  retry(): void;                // 重新检查并再次应用当前值
}

type FilterStatus =
  | { state: "waiting" }   // 没有就绪的媒体；值已保留，媒体就绪后自动应用
  | { state: "applying" }
  | { state: "applied" }   // 播放器已接受（不保证下一帧立即可见）
  | { state: "failed"; error: PluginError };
```

- 每个插件拥有自己的处理链，多个插件的链按稳定顺序串联；撤销或释放只影响自己的链。
- 换片源后，SDK 自动把当前值重新应用到新来源。
- 拖动滑块时连续调用 `set`：同一时间只有一个请求在途，只保留最新值。
- `failed` 之后不会自动重试，直到下一次 `set()`、`retry()` 或来源变化。`error.code === "backend-uncertain"` 表示播放器后端暂时阻塞了该阶段。

节点类型与取值范围（超出范围以 `invalid` 失败）：

| 节点 | 字段与范围 |
| --- | --- |
| `{ kind: "eq", brightness, contrast, saturation, gamma }` | brightness −1–1，contrast 0–2，saturation 0–2，gamma 0.5–2 |
| `{ kind: "gblur", sigma, steps: 3 }` | sigma 0–10 |
| `{ kind: "volume", gainDb }` | −60–0 |
| `{ kind: "equalizer", frequency, q: 1, gainDb }` | frequency 20–20000，gainDb −6–6 |
| `{ kind: "treble", frequency: 3000, slope: 0.5, gainDb }` | gainDb 0–6 |
| `{ kind: "dynaudnorm", enabled, maxGain, peak }` | maxGain 1–4，peak 0.5–0.95 |
| 音频 `delayMs` | −2000–2000，10 的倍数 |

## 9. 字幕 `plugin.subtitles`

读取需要 `subtitles.read`，修改需要 `subtitles.control`。

```ts
interface Subtitles {
  readonly state: SubtitlesState | null;  // 首次读取前为 null
  onChange(listener: (s: SubtitlesState) => void): Disposable;
  refresh(): Promise<SubtitlesState>;
  select(track: SubtitleTrack | null, opts?): Promise<SubtitlesState>; // null = 关闭字幕
  load(srt: Uint8Array, opts?: { ifCurrent?; signal? }): Promise<SubtitleTrack>;
  remove(track: SubtitleTrack, opts?): Promise<SubtitlesState>;       // 只能删本插件载入的
  setStyle(style: SubtitleStyle | null, opts?): Promise<SubtitlesState>; // null = 撤销本插件的样式层
}

interface SubtitlesState {
  readonly sourceId: number;
  readonly phase: string;
  readonly tracks: SubtitleTrack[];
  readonly selected: SubtitleTrack | null;
  readonly fonts: { id: string; family: string }[];
  readonly style: SubtitleStyle | null;    // 生效的样式（来自最后设置的插件）
  readonly trackConfirmation: "waiting" | "applied" | "uncertain";
  readonly styleConfirmation: "waiting" | "applied" | "uncertain";
  readonly error: string | null;
}

interface SubtitleTrack {
  id: string;              // 不透明字符串；比较用 id，不要解析格式
  kind: "embedded" | "external";
  embeddedId?: number;     // 内嵌轨在容器中的编号
  title: string; language: string | null;
  styleSupported: boolean; styleReason: string | null;
  owned: boolean;          // 由本插件载入，可以 remove
}

type SubtitleStyle = {
  fontId: string;          // 取自 state.fonts
  fontSize: number;        // 16–72
  color: "white" | "yellow" | "cyan";
  outline: number;         // 0–4
  marginPercent: number;   // 2–15
  offsetMs: number;        // −10000–10000
};
```

- `select`、`remove`、`setStyle` 默认以提交前读到的当前选择为条件；`opts.ifCurrent` 传入 `SubtitlesState` 可绑定到更早的选择。
- `load` 上传 UTF-8 SRT 的字节、校验、提交并选中。`signal` 在提交之前中止会丢弃上传并以 `AbortError` reject；提交发出后不再中止。
- 样式按 1080p 参考高度缩放。

## 10. 截图与导出 `plugin.media`

需要 `media.read`、`media.control`。

```ts
media.capture(options?: {
  at?: number;                          // 来源时间；默认截图那一刻的位置
  processed?: boolean;                  // 包含插件效果（以及字幕，除非 subtitles: false）
  subtitles?: boolean;
  audio?: "selected" | "none" | number; // 随快照冻结的音轨
  ifCurrent?: PlayerState;
}): Promise<Snapshot>

interface Snapshot extends Disposable {   // 冻结的不可变输入；dispose 释放
  readonly kind: "raw" | "processed";
  readonly time: number; readonly source: string; readonly duration: number;
  readonly audioTrack: number | null;
  readonly presentation: { width: number; height: number } | null;
  readonly processingRevision: number | null;
  export(options:
    | { format: "png"; destination: string; sampleTime?: number }
    | { format: "mp4"; destination: string; end: number }): Promise<ExportJob>;
}

interface ExportJob extends Disposable {
  readonly state: JobState;
  onChange(listener: (s: JobState) => void): Disposable;
  readonly done: Promise<JobState>;   // 终态：completed / failed / cancelled
  cancel(): Promise<"requested" | "tooLate" | "terminal">;
  refresh(): Promise<JobState>;
}
```

- `JobState.status`：`queued`、`running`、`cancelling`、`completed`、`failed`、`cancelled`。终态时 `outcome` 为 `{ destination }`、`{ error: { code, message } }` 或 `{ cancelled: true }`。
- 导出在播放器的离线工作进程中独立解码，不影响播放。目标文件已存在时不覆盖。
- 同一快照可多次导出；用完 `dispose()`。

## 11. 网络地址解析 `plugin.resolvers`

需要 `source.resolve`。把网页地址解析成可播放的流地址；用户（或任何插件）`player.open` 匹配的 URL 时，播放器调用对应解析器。

```ts
resolvers.add(options: {
  id: string;
  schemes: ("http" | "https")[];
  hosts: string[];    // 精确主机名
  resolve(url: string, signal: AbortSignal): Promise<ResolvedMedia>;
}): Resolver          // { ready: Promise<void> } & Disposable

type ResolvedMedia = {
  kind: "media";
  originalUrl: string;
  title?: string;
  primary: { url: string; headers?: Record<string, string> };
  audio?: { url: string; headers?: Record<string, string> }; // 音视频分离时
};
```

- 一个 scheme + 主机组合只能有一个解析器。
- 打开被取消或被替换时 `signal` 中止。
- 两路流的 `headers` 必须相同，不同会明确失败。

## 12. 插件包管理 `plugin.packages`

需要 `plugins.manage`。用于插件工坊这类安装其他插件的插件。

```ts
interface Packages {
  validate(directory: string): Promise<PackageCandidate>; // 校验目录中的包
  rollback(id: string): Promise<PackageChange>;           // 回退到上一版本
}
interface PackageCandidate {
  readonly directory: string; readonly digest: string;
  readonly manifest: PackageManifest; readonly checks: string[];
  trial(): Promise<PackageTrial>;     // 与已安装插件并行临时加载
  install(): Promise<PackageChange>;  // 安装，替换同 id 的已安装版本
}
interface PackageTrial extends Disposable {
  readonly status: string;
  stop(): Promise<void>;  // 停止并等待清理确认
}
interface PackageChange {
  readonly status: "preparing" | "activating" | "restoring" | "committed"
                 | "cancelled" | "failed" | "failedDegraded" | "tooLate";
  readonly message: string | null;
  onChange(listener: (status) => void): Disposable;
  readonly done: Promise<status>;
  cancel(): Promise<void>;
}
```

- 安装和回退从不重复提交：请求 id 与预期修订号由 SDK 处理，答复丢失时只按同一请求查询。
- `trial.stop()` 以 `code: "cleanup"` 失败表示播放器停止了试载但未确认清理；再次调用会重新请求停止。

## 13. 错误

失败一律是普通 `Error`，带稳定的 `code`（类型 `PluginError = Error & { code: string }`）。按 `code` 判断，不要按 `message` 文本判断。

| code | 含义 |
| --- | --- |
| `stale` | 条件不再成立（来源或字幕选择已被改变），**什么都没做** |
| `capability` | manifest 未声明所需能力 |
| `invalid` | 参数不合法（如滤镜值超出范围） |
| `busy`、`conflict`、`unavailable`、`unsupported` | 播放器拒绝接纳，未执行 |
| `backend` | 载入或解析失败，`message` 为后端原因 |
| `disabled` | 插件正在停用 |
| `replaced` | 已被接受，但完成前被另一个来源替换（`open`、`openStream`、`media.ready`） |
| `failed`、`ended` | `media.ready()`：载入失败 / 已播完 |
| `aborted` | `signal` 中止（`error.name === "AbortError"`） |
| `timeout` | `openStream` 启动超时；`error.stage` 为 `"prefill"`（首批缓冲未在 10 秒内备好）或 `"activation"`（选中后 10 秒内未激活） |
| `stream`、`device` | `openStream`：生成端报错 / 音频设备未就绪 |
| `backend-uncertain` | 滤镜阶段被播放器后端阻塞，`filter.retry()` 重新检查 |
| `cleanup` | `trial.stop()` 未确认清理，可再次调用 |
| `unknown`、`rejected`、`expired` | 传输异常后 SDK 查询仍无法确认结果（正常情况下不会出现） |

处理函数里抛出的错误交给所在作用域的 `onError`。忘记 `await` 的 promise 被拒绝时只写日志，不会让插件进程崩溃。

请求没有客户端超时：管道有序，播放器对每个请求都答复或停止插件；超过 10 秒的请求只写一条日志并继续等待。

## 14. 测试工具

```ts
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";

const host = new TestHost();
const { stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
```

`TestHost` 是内存中的假播放器，说真实协议，SDK 的全部逻辑照常运行。`startPlugin` 读取插件目录的 `manifest.json`。

| 类别 | 成员 |
| --- | --- |
| 操作 | `host.command(id).run()`（禁用或忙碌时同样拒绝）；`host.window(id).emit(callback, ...args)`；`host.mounted().emit(...)`；`host.dialogAnswers.push([path] \| null)` |
| 模拟来源 | `host.openElsewhere(path, phase?)`（另一个插件打开了媒体）；`host.setSource({ phase, position, duration, … })` |
| 断言 | `host.window(id).props`（kebab-case 名）、`.visible`；`host.command(id).state`；`host.sent(method)`；`host.source`；`host.processing`、`host.subtitles`、`host.snapshots`、`host.jobs`、`host.producers` |
| 注入故障 | `host.handle(method, fn)`：抛 `new HostError(code, message)` 模拟播放器拒绝，抛普通 `Error` 模拟答复丢失；`host.reset(method)` 恢复 |
| 同步 | `await host.settle(ms?)`：等到没有在途请求（可额外等待计时器） |
| 结束 | `await stop()` |

语义是简化的：没有解码，没有 Slint 渲染，导出任务每次查询前进一步。

## 15. 构建工具

在 `src/` 下：

```
npm run plugin -- check <名字|路径…>   # 严格类型检查（含 tests/）
npm run plugin -- test  <名字|路径…>   # node --test tests/*.test.{ts,mjs}
npm run plugin -- build <名字|路径…> [--version x.y.z]
npm run plugins:build                   # 构建 src/plugins/ 下全部插件
```

- 名字指 `src/plugins/<名字>`；含 `/` 的参数是相对 `src/` 的路径（如 `examples/bookmark`）。`--all` 只覆盖 `src/plugins/`。
- `build` 先 `check`，然后生成 `dist/`：`.ts` 去掉类型、相对导入改为 `.js`；`.slint`、`.mjs`、数据文件、`licenses/` 原样复制；跳过 `tests/`、`node_modules/`、`vendor/`、`*.md`、`tsconfig*.json`、`package*.json`；写入 `dist/manifest.json`（入口改为 `.js`，写入版本）。版本取 `--version`，否则 manifest，否则 `package.json`。`dist/` 不提交。
- 有 npm 依赖时，插件 `package.json` 的 `"vestra"` 字段：
  - `"bundle": true` 或 `{ "external": [...] }`：用插件自己的 esbuild（devDependency）把入口及依赖打包成一个文件。**有运行时依赖就必须打包**，否则安装后找不到依赖包。
  - `"assets": ["vendor"]`：原样复制额外的文件或目录。
- 开发时不需要构建：`cargo run --bin vestra-host -- --plugin <插件目录>`（在 `src/host/` 下，可重复 `--plugin`）直接运行 `main.ts`。
