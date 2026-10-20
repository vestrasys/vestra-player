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
| `commands` | `commands.add`、`host.run` |
| `ui.window` | `windows.create` |
| `ui.mount` | `panels.mount`、`panels.bottom` / `panels.sidebar`（任何插件读写两个面板位置） |
| `ui.dialog` | `dialogs.*`（含 `dialogs.confirm`） |
| `playback.read` | `player.state` 启动时读取、`player.refresh`、`player.stats`、`player.chapters`、`player.audioDevices`；滤镜读取处理状态 |
| `events` | `player.state` / `onChange` 实时更新、面板可见性、字幕状态刷新 |
| `playback.control` | `player.open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput`、`provideSequence`、`next`、`previous`、`addAudioTrack`、`selectAudioTrack`、`step`、`setStepSeek`、`setLoop`、`setAudioDevice`；`power.preventSleep` |
| `producer.av` | `player.openStream` |
| `processing.video` | `video.filter`（另需 `playback.read`） |
| `processing.audio` | `audio.filter`（另需 `playback.read`） |
| `subtitles.read` | `subtitles.state`、`onChange`、`refresh` |
| `subtitles.control` | `subtitles.select`、`load`、`remove`、`setStyle` |
| `media.read` / `media.control` | `media.capture`、`Snapshot.export`、`ExportJob`（`processed` 截图另需 `playback.read`、`subtitles.read`） |
| `source.resolve` | `resolvers.add` |
| `plugins.manage` | `packages.validate`、`rollback`、`setEnabled`、`uninstall`（`list`、`onChange` 不需要能力） |
| `ui.osd` | `osd.show`、`osd.panel` |
| `clipboard` | `clipboard.readText`、`writeText`、`writeImage` |
| `window.control` | `window.setVideoArea`、`setAlwaysOnTop`、`setCursorAutoHide` |
| `bindings.manage` | `bindings.*`（§4.2） |

`tools`、`storage`、`system`（§16）在插件进程内执行，不需要能力。读取 `window.state` / `onChange`（§6.6）与 `plugin.app`（§3.5）不需要能力。

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
           subtitles, media, resolvers, packages, osd,
           host, clipboard, power, tools, storage, system, window;
  readonly app: AppInfo;           // §3.5
  scope(): Scope;
  add<T extends Disposable>(item: T): T;
  onDispose(cleanup: () => void | Promise<void>): Disposable;
  onError(listener: (error: PluginError) => void): Disposable;
  update(): void;
}
interface Plugin extends Scope {
  readonly id: string;             // manifest id
  readonly dataDirectory: string;  // 本插件可写的数据目录，见 3.4
}
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
| `Command` | 从菜单和绑定表中删除；同一个 id 可以再次 `add` |
| `Window` | 关闭窗口 |
| `Panel` | 卸载面板 |
| `VideoFilter` / `AudioFilter` | 若应用过，替换为空链（只撤销自己的效果） |
| `Snapshot` | 释放冻结的输入 |
| `ExportJob` | 未结束则取消 |
| `Stream` | 停止生成；若仍是当前来源则关闭 |
| `Resolver` | 注销并中止进行中的解析 |
| `PackageTrial` | 停止试载并等待清理确认 |
| `JsonStore` | 写完待写的值 |
| 进行中的 `tools.run` | 结束子进程（以 `aborted` 失败） |
| 监听（`on`、`bind`、`onChange`…） | 退订 |
| `Media` | **不**关闭媒体；需要时显式 `media.close()` |

### 3.4 数据目录 `plugin.dataDirectory`

本插件自己的可写目录（设置、记住的文件夹等），不需要能力。

- 位置：`~/Library/Application Support/Vestra/plugin-data/<manifest id>/`（环境变量 `VESTRA_PLUGIN_DATA` 可改根目录），播放器在启动插件前创建。插件停用、重新加载、升级、回退时都保留；卸载时只有 `uninstall(id, { removeData: true })` 才删除。别的插件（如插件管理）按同一规则 `<数据根>/<id>` 找到某个插件的数据目录：数据根是 `plugin.dataDirectory` 的上一级（`dirname(plugin.dataDirectory)`）。
- 试载的插件拿到一个临时目录，试载停止时删除，不会碰正式安装版本的数据。
- JSON 设置文件用 `plugin.storage.json`（§16.4），它已处理默认值、读坏改名、去抖与原子写入。
- 存什么、什么格式由插件决定。自己写文件时先写临时文件再改名（`writeFileSync(tmp)` → `renameSync(tmp, path)`），播放器中途退出也不会留下写了一半的文件；读不到或读坏时用默认值。
- 播放器没能提供目录时（例如创建失败），读取这个属性抛出 `code: "unavailable"`。
- 测试：`TestHost` 为每个测试提供一个空的临时目录（`host.dataDirectory`，`stop()` 时删除）；要测试「重启后恢复」，给 `startPlugin` 传 `{ dataDirectory }` 指定同一个目录。

### 3.5 应用信息 `plugin.app`

```ts
interface AppInfo {
  readonly version: string;          // 播放器版本（宿主 Cargo 包版本）
  readonly build: string | null;     // 构建时的源码短哈希；构建时取不到为 null
  readonly components: {
    readonly mpv: string | null;     // libmpv 的 mpv-version，如 "mpv v0.41.0"
    readonly libav: Record<string, string>; // { ffmpeg: <ffmpeg-version> }
    readonly slint: string;          // 链接的 Slint 版本，如 "1.16.1"
  };
}
```

- 同步只读、不需要能力，子作用域相同。播放器启动插件进程时随握手交给它（不经请求），整个进程内不变。
- `libav` 只有 libmpv 报告的 `ffmpeg-version`（键 `ffmpeg`）：libmpv 0.41 不报告 libavcodec 等各库的版本，需要时用 `tools.run("ffmpeg", ["-version"])` 读取。
- 测试：`host.app` 在 `startPlugin` 之前设置。

## 4. 命令 `plugin.commands`

需要 `commands`。

```ts
commands.add(options: CommandOptions): Command

interface CommandOptions {
  id: string;                         // 插件内唯一
  path?: string[];                    // 菜单路径，1–8 段，最后一项是菜单项；1 段 = 顶层直接命令；省略 = 不进主菜单
  title?: string;                     // 菜单文字，默认取 path 最后一项；省略 path 时必填
  order?: number;                     // 同级排序，小的在前；整数，±1 000 000 内，默认 0
  group?: string;                     // 分组名，相邻两项分组不同时画分隔线；默认 "domain"
  radio?: boolean;                    // checked 画成单选圆点而不是 ✓
  bindings?: string[];                // 默认按键/鼠标绑定，见 4.1
  focus?: "panel";                    // 绑定只在本插件面板获得焦点时生效
  repeat?: boolean;                   // 按住不放时随系统自动重复触发；默认 false
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

- 菜单排序：同级按 `order` 升序；子菜单（路径中间的一段）排在它下面最靠前的命令的位置，并取该命令的 `group`。`order` 相同时按插件 id、再按同一插件的声明顺序，**不按标题排序**。`group` 只决定分隔线，不参与排序：一个分组要连在一起，它的 `order` 也要连续。
- 推荐顶层骨架（[doc/product/menus.md](../product/menus.md) 维护顶层编号）：`open` 100–199（打开、关闭）、`domain` 200–699（播放、字幕、视频、声音、模拟器等子菜单）、`screen` 700–799（宿主「全屏」700）、`windows` 800–899（播放列表、播放信息等窗口入口）、`exit` 900–999（宿主「退出」990）。
- 同一路径不能注册两次，也不能既是菜单项又是子菜单（`conflict`）。路径、`order`、`group`、绑定格式不合法时 `commands.add` 立即抛出 `invalid`。
- 菜单项右侧的快捷键文字由绑定表生成（第一个生效的键盘绑定）。
- 状态字段为函数时，每轮更新重新求值，变化才发送。
- `dispose()` 把命令从菜单和绑定表中删除，id 随即可以重新使用（紧接着 `add` 同一个 id 也可以，SDK 会等删除完成再注册）。标题、路径不能原地改变：会变的菜单项（字幕轨列表、预设、单选组）放进一个子作用域，变化时释放整个子作用域再按新内容添加，位置由 `order` 保持不变：

  ```ts
  let tracks = plugin.scope();
  plugin.subtitles.onChange(async (state) => {
    await tracks.dispose();
    tracks = plugin.scope();
    state.tracks.forEach((track, i) =>
      tracks.commands.add({ id: `track.${i}`, path: ["字幕", "选择字幕", track.title], order: 300 + i,
        radio: true, checked: () => plugin.subtitles.state?.selected?.id === track.id,
        run: () => plugin.subtitles.select(track) }));
  });
  ```
- 命令禁用或忙碌时，播放器不会调用 `run`（绑定的按键被接住但不执行）。`run` 抛出的错误交给 `onError`。

### 4.1 绑定

```text
binding  = { 修饰键 "+" } 输入          修饰键按 Ctrl、Alt、Shift、Cmd 的顺序，各至多一次
输入     = A–Z | 0–9 | F1–F24 | Space | Enter | Tab | Escape | Backspace | Delete | Insert
         | Home | End | PageUp | PageDown | ArrowLeft | ArrowRight | ArrowUp | ArrowDown
         | Plus | 其它单个符号（> < / , . ; ' [ ] \ - = ` …）
         | Numpad0–Numpad9 | NumpadPlus | NumpadMinus | NumpadMultiply | NumpadDivide
         | NumpadEnter | NumpadDecimal
         | Mouse:LeftClick | Mouse:LeftDoubleClick[:LeftHalf | :RightHalf] | Mouse:MiddleClick | Mouse:X1 | Mouse:X2
         | Mouse:WheelUp | Mouse:WheelDown | Mouse:WheelLeft | Mouse:WheelRight
```

- 字母按键名匹配（与大小写锁无关），`Shift+E` 是 Shift 加 E。符号键按打出的字符写、不写 Shift：`">"`，不是 `"Shift+."`；`+` 写成 `Plus`；`"Shift+1"` 非法，写 `"!"`。
- 命名键按物理键识别，与修饰键和打出的文字无关（⌥⌦、⌥← 也是 `Alt+Delete`、`Alt+ArrowLeft`）。`Delete` 是向前删除键 ⌦（没有独立 ⌦ 键的 Mac 笔记本上为 fn+⌫），`Backspace` 是 ⌫。
- 小键盘是独立的输入名：`"5"` 只认主键盘的 5，`"Numpad5"` 只认小键盘的 5；`"Enter"` 不认小键盘回车（写 `"NumpadEnter"`）。菜单里显示为 `Num5`、`Num+`、`NumEnter`。
- Option（`Alt`）组合按物理键识别：`Alt+O` 按 ⌥O 触发，不看 ⌥O 打出的 `ø`；⌥E、⌥N、⌥U、⌥I、⌥\` 这类死键同样触发（死键只在视频区获得焦点时生效，输入框里照常组字）。键盘布局：Option 与 Ctrl 组合的字母和符号取当前布局不带修饰时的字符（德语 QWERTZ 上标着 Y 的键是 `Ctrl+Y`，与键帽一致）；`Alt+Shift+数字/符号` 时系统不报告该布局 Shift 后的字符，按美式键位换算（⌥⇧. 是 `Alt+>`，即使该布局 Shift+. 打出 `:`），美式键位上没有的符号键（如德语的 ß、´）与 Option+Shift 同按时不匹配任何绑定。在需要 Option 才能打出符号的布局上（德语 ⌥5 = `[`、⌥7 = `|`、⌥⇧7 = `\`、⌥L = `@`），Option 组合打出的非字母、非数字字符先按该字符匹配（不带 Alt 与 Shift，绑定写 `"["`、`"@"`），没有这样的绑定时再按物理键匹配（`Alt+5`、`Alt+L`）；两者都有绑定时字符优先。打出的是字母（⌥O 的 `ø`）时只按物理键（`Alt+O`）。美式布局上 ⌥[ 打出 `“`，通常无人绑定，仍是 `Alt+[`。小键盘名与布局无关。
- Ctrl 组合即使系统以 ASCII 控制码送达也按键名识别：⌃Y 是 `Ctrl+Y`（不是 Shift+Tab），⌃I、⌃H、⌃M、⌃[ 分别是 `Ctrl+I`、`Ctrl+H`、`Ctrl+M`、`Ctrl+[`。
- macOS 上 `Ctrl+X` 按 ⌃X 或 ⌘X 都触发，`Cmd+X` 只认 ⌘X。
- `Mouse:LeftClick`（视频区左键单击）：只有它有生效的绑定时，播放器才延迟判定左键按下：系统双击间隔内没有第二次按下、指针移动不超过 4 点，才算单击并触发；第二次按下算双击（`Mouse:LeftDoubleClick` 照常）。没有绑定时左键行为不变（立即聚焦视频区，没有延迟）。
- 鼠标绑定只在视频区生效，可带修饰键；`:LeftHalf` / `:RightHalf` 按双击位置区分左右半边，带区的绑定优先于不带区的。滚轮按累计位移计步。右键保留给菜单。
- 作用范围：默认全局（主窗口或浮动面板窗口获得焦点、且按键没被焦点控件吃掉时，例如输入框里的空格不会触发）；`focus: "panel"` 只在本插件面板（附着或浮动）获得焦点时生效，此时先于全局查找。工具窗口里不触发绑定。
- 菜单或系统对话框打开时绑定暂停。默认只在按下时触发；`repeat: true` 时按住会重复触发。
- 冲突（同一输入、同一范围被两个命令声明）不报错：用户覆盖层优先，其次宿主自己的命令，再次先加载的插件；被压下的绑定写一行宿主日志，赢家停用后自动生效。测试里用 `host.conflicts` 检查。
- 宿主自己的命令：「全屏」（`Enter`、`F11`、`Mouse:LeftDoubleClick`）、「退出全屏」（`Escape`，不在菜单）、「退出」（`Cmd+Q`），以及 14 个基础播放动作（§5.5）。
- 用户覆盖层保存在界面状态文件（[架构总览 §6](../architecture/README.md)），由 `plugin.bindings`（§4.2）读写。

### 4.2 绑定管理 `plugin.bindings`

需要 `bindings.manage`。供快捷键设置这类插件列出、改绑、恢复所有命令（含播放器自己的）的绑定。

```ts
interface BindingRef { plugin: string /* manifest id；播放器自己的命令为 "host" */; command: string }
interface BindingInfo extends BindingRef {
  title: string; path: string[] | null;  // 不进菜单的命令 path 为 null
  scope: "global" | "panel";
  defaults: string[];   // 命令自己声明的
  current: string[];    // 生效的：有覆盖时为覆盖值，否则为 defaults
  overridden: boolean;
  shadowed: { input: string; by: BindingRef }[];  // current 中被别的命令占用的
  labels: string[];     // current 各项的菜单文字（Ctrl+←、Num5）
}
bindings.list(): Promise<{ commands: BindingInfo[]; orphans: { ref: BindingRef; inputs: string[] }[] }>;
bindings.set(ref: BindingRef, inputs: string[]): Promise<void>;  // [] = 解绑
bindings.reset(ref?: BindingRef): Promise<void>;                 // 省略 = 清空全部覆盖
bindings.onChange(listener: () => void): Disposable;
```

- `list`：当前声明的全部命令（运行中的插件与播放器），以及 `orphans`：覆盖层里命令当前不存在的条目（插件停用或卸载）。这些条目保留，插件回来后照常生效。
- `set`：最多 16 个，语法同 §4.1，按规范写法保存（`Ctrl+Shift+A`），重复或不合法时 `invalid`；命令不存在且也不是 orphan 时 `missing`。覆盖优先于所有默认绑定（§4.1 冲突规则），跨启动保存。改「全屏」之类的宿主命令同样可以。
- 「忽略默认快捷键」：对要忽略的命令 `set(ref, [])`，自己记下写过哪些，恢复时对它们 `reset(ref)`（裁决 R-K2）。
- `onChange`：命令声明、注销或覆盖层变化后调用（合并为每 200 ms 至多一次），收到后再 `list()`。只发给声明了 `bindings.manage` 的插件。

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
  readonly can: { pause; seek; rate; restart; close; snapshot; next; previous: boolean };
  readonly audioTracks: AudioTrack[] | null;
  readonly audioOutput?: { requested: string | null; active: string | null }; // §5.2 setAudioDevice
  readonly loop?: { a: number; b: number; count: number | null } | null; // A-B 循环，§5.2 setLoop
}

interface AudioTrack {
  id: number; selected: boolean;
  external: boolean;     // 外部音轨不能导出
  exportable: boolean;
  codec: string | null; title: string | null; language: string | null;
}
```

`PlayerState` 是不可变快照，可以保存下来，用作之后操作的条件（§5.3）。`loop` 由 SDK 总是给出（没有循环为 `null`）；类型上写成可选，只是为了让测试里手写的状态对象不必带它。`can.next` / `can.previous` 来自播放序列提供者（§5.4），没有提供者时为 `false`。

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
| `addAudioTrack(path, { select?, ifCurrent? })` → `Promise<AudioTrack>` | 给当前媒体加一条外部音频文件（本地绝对路径），默认同时选中（`select: false` 只添加）。返回新音轨（`external: true`、`exportable: false`），它保留到换来源。路径不合法 `invalid`，没有就绪的媒体（含生成流）`unavailable`，载入失败 `backend` |
| `selectAudioTrack(id, opts?)` → `Promise<PlayerState>` | 选中 `state.audioTracks` 中的一条；id 不存在 `invalid` |
| `step(unit, direction, opts?)` | `unit`：`"frame"`（一帧，前进时暂停在下一帧）或 `"keyframe"`（严格越过当前关键帧到下一个/上一个）；`direction`：`"backward"` \| `"forward"`。可用条件与核心「上一帧/下一帧」相同（§5.5，`message` 为其原因），关键帧另需就绪且可跳转，否则 `unsupported`。**resolve 时新位置已生效**：宿主等到位置变化且没有在途的跳转（或 1.5 秒内不动 = 已到边界）才答复，SDK 随后刷新，之后读 `player.state.position` 就是新位置。同一时间只有一步在途，第二个调用以 `busy` 失败；连按的去重由调用方决定 |
| `chapters()` → `Promise<{ sourceId, chapters: { title: string \| null; start: number }[] }>` | 当前媒体的章节（mpv `chapter-list`，`start` 为秒），按需读取，不随状态推送；没有媒体、生成流或没有章节时为空数组。`sourceId` 是读取时的来源，与 `player.state.sourceId` 比较可知是否已过期。需要 `playback.read` |
| `setLoop(loop \| null, opts?)` | A-B 循环：播到 `b` 回到 `a`（秒，`0 ≤ a < b ≤ 时长`），回跳 `count` 次（正整数，≤ 1 000 000；省略 = 无限），`null` 取消。结果在 `state.loop`（`count` 为请求值，不是剩余次数）；最后一次回跳后 `state.loop` 变为 `null`。范围不对 `invalid`；不可跳转的来源、生成流 `unsupported`；没有就绪的媒体 `unavailable`；条件提交同其它控制。来源变化（新的 `sourceId`）时播放器清除循环 |
| `audioDevices()` → `Promise<{ id: string; name: string }[]>` | 现在接入的音频输出设备（不含「系统默认」项；`id` 如 `coreaudio/<设备 UID>`，`name` 为系统显示的名字）。新接入的设备不推送，需要时再读（如每 5 秒）。需要 `playback.read` |
| `setAudioDevice(id \| null)` | 在该设备接入期间用它播放，不在时用系统默认，重新接入后切回；`null` = 总是系统默认。`state.audioOutput.requested` 是请求的 id，`active` 是正在用的（`null` = 系统默认），播放器每秒检查一次设备。只在本次运行内记住，不跨启动保存（插件自己保存并在激活时设置）。插件生成的音视频流（`openStream`）同样跟随。不存在的 id 也接受（`active` 为 `null`）；格式不对 `invalid` |
| `setStepSeek({ seconds })` | 核心第 1 档相对跳转（`seek-backward` / `seek-forward`，← / →）的步长：0.1–3600，一位小数，默认 5；菜单标题随之变为「后退 N秒」。所有插件最后写入的生效，该插件停用后回到上一个仍有效的值（都没有则 5） |

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

### 5.4 播放序列与统计

```ts
interface Player {
  provideSequence(provider: {
    next(): unknown; previous(): unknown;
    canNext: Bindable<boolean>; canPrevious: Bindable<boolean>;
  }): Disposable;
  next(): Promise<void>;       // 交给提供者，不等它执行完
  previous(): Promise<void>;
  stats(): Promise<PlaybackStats>;
}
interface PlaybackStats {
  backend: string; session: number; framesShown: number; framesDropped: number | null;
  video: { codec; width; height; fps;                   // 字段都可为 null
           decoder; pixelFormat; colorspace; displayWidth; displayHeight; displayFps; bitrate } | null;
  audio: { codec; sampleRate; channels; decoder; format; channelLayout; bitrate } | null;
  avSyncMs: number | null;      // 最近一次音画差（毫秒，有正负）
  cacheSeconds: number | null;  // 解复用器已缓冲的秒数
}
```

新字段原样取自 mpv 0.41 属性，读不到为 `null`，不做换算：

| 字段 | mpv 属性 | 说明 |
| --- | --- | --- |
| `video.decoder` / `audio.decoder` | `current-tracks/video/decoder` / `current-tracks/audio/decoder` | 短解码器名，如 `h264`、`aac` |
| `video.pixelFormat` | `video-params/pixelformat` | 如 `yuv420p` |
| `video.colorspace` | `video-params/colormatrix` | 如 `bt.709` |
| `video.displayWidth` / `displayHeight` | `video-out-params/dw` / `dh` | 宽高比校正与滤镜之后的尺寸 |
| `video.displayFps` | `estimated-display-fps` | 只在显示同步模式下有值；本播放器的渲染方式下实测通常为 `null` |
| `video.bitrate` / `audio.bitrate` | `video-bitrate` / `audio-bitrate` | 比特/秒；视频按关键帧更新，刚开始播放时可能为 `null` |
| `audio.format` | `audio-params/format` | 采样格式，如 `floatp` |
| `audio.channelLayout` | `audio-params/hr-channels` | 如 `stereo`、`5.1(side)` |
| `avSyncMs` | `avsync` × 1000 | 无音频或无视频时为 `null` |
| `cacheSeconds` | `demuxer-cache-duration` | |

不提供硬件解码状态（本播放器固定软件解码）。

- 同时只能有一个提供者（例如播放列表）；另一个插件再登记以 `conflict` 失败（激活期间 = 激活失败）。`dispose` 或插件停用时撤销。
- `canNext` / `canPrevious` 每轮更新重新求值，变化才发送；任何插件从 `player.state.can.next/previous` 读到。
- `next()` / `previous()`：没有提供者或标志为假时以 `unavailable` 失败；否则宿主把请求交给提供者，在它的作用域里调用 `next` / `previous`。
- `stats()` 需要 `playback.read`；没有来源时 `video`、`audio` 为 `null`。

### 5.5 核心动作 `plugin.host.run`

需要 `commands`。

```ts
type HostAction =
  | "play-pause" | "stop" | "previous" | "next"
  | "frame-back" | "frame-forward"
  | "seek-backward" | "seek-forward"
  | "rate-down" | "rate-up" | "rate-normal"
  | "volume-up" | "volume-down" | "mute";
plugin.host.run(action: HostAction): Promise<void>;
```

基础播放动作由播放器自己注册为菜单命令（所有者 `host`），与「全屏」「退出」相同；任何插件停用或不存在时它们照常可用。`host.run` 等价于执行该命令：同样的启用判断、同样的屏幕提示。插件的按钮（如底栏的播放/暂停）应调用它，而不是自己重新实现。

| 动作 | 菜单路径 | order / group | 默认绑定 | 行为与提示 |
| --- | --- | --- | --- | --- |
| `stop` | 关闭 | 120 / open | F4 | 关闭来源；没有来源时禁用 |
| `play-pause` | 播放 ▶ 播放/暂停 | 201 | Space | 暂停/继续，播完后重播；「已暂停」/「播放」；勾选 = 已暂停 |
| `previous` / `next` | 播放 ▶ 上一个 / 下一个 | 202 / 203 | PageUp、Mouse:X1 / PageDown、Mouse:X2 | 交给播放序列提供者（§5.4）；不可用时原因「没有上一个」/「没有下一个」 |
| `frame-back` / `frame-forward` | 播放 ▶ 跳转 ▶ 上一帧 / 下一帧 | 204 / 205，frame | D / F（按住重复） | `player.step("frame", …)`；在途时丢弃重复触发；完成后提示「定位: 下一帧 00:01:23.456」/「定位: 上一帧 …」（完成后的位置，精确到毫秒），没动时「已是最后一帧」/「已是第一帧」。两个方向分别判断：没有视频轨（纯音频、只有封面）禁用，原因「没有视频」；下一帧需要就绪（直播也可以，播完后不可）；上一帧播完后也可用，需要可跳转（生成流、不可跳转的直播原因「当前来源不支持逐帧后退」） |
| `seek-backward` / `seek-forward` | 播放 ▶ 跳转 ▶ 后退/前进 N秒 | 210 / 211，step | ← / →（按住重复） | 从最新未完成的目标累加，夹在 0 与时长之间；「前进 5秒: 00:37 / 01:40」；N 见 `setStepSeek` |
| `rate-normal` | 播放 ▶ 速度 ▶ 正常（单选） | 220 | Z | 1 倍；「播放速度: 1x」 |
| `rate-down` / `rate-up` | 播放 ▶ 速度 ▶ 减速 0.1 / 加速 0.1 | 221 / 222 | X / C | ±0.1，取两位小数，夹在 0.25–4；「播放速度: 1.1x」 |
| `volume-up` / `volume-down` | 声音 ▶ 音量 + / 音量 − | 501 / 502 | ↑、滚轮上 / ↓、滚轮下（按住重复） | ±5，夹在 0–200，先设音量再取消静音；「音量: 85%」 |
| `mute` | 声音 ▶ 静音（勾选） | 503 | M | 「静音: 开」/「静音: 关」 |

- 禁用时 `run` 以 `unavailable` 失败（`message` 为原因），未知 id 为 `invalid`。动作执行完才 resolve（之后 `player.state` 已刷新）；同一动作在途时的重复调用被丢弃并直接 resolve（相对跳转与音量除外，它们排队累加）。
- 执行失败（如来源拒绝）显示在视频上。
- 用户覆盖层的键为 `host/<动作 id>`。
- 用户覆盖层的旧键 `vestra.playback/<旧 id>` 在播放器启动时一次性改写为 `host/<新 id>`（§17）。

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
  onClose(listener: () => unknown): Disposable; // 用户用标题栏关闭了窗口
}
```

- 用户点标题栏的关闭按钮时，播放器关闭该窗口（等同 `close()`），`opened` 变为 `false`，然后调用 `onClose` 的监听；之后 `show()` 用最新属性重新打开。插件自己调用 `close()` / `dispose()` 不触发 `onClose`。轮询之类的工作在 `onClose` 里停掉。测试里用 `host.window(id).userClose()` 模拟。
- `create` 不会打开窗口。第一次 `show()` 时，用当前绑定的属性作为初始值打开，不会先闪一下默认值。
- **回调必须在第一次 `show()` 之前用 `on` 绑定**，之后再绑定新的回调名会抛错。
- 窗口 id 默认取文件名（`ui/picture.slint` → `picture`）；同一插件用同一文件开多个窗口时，用 `options.id` 区分，id 重复会抛错。

### 6.3 面板 `plugin.panels`

需要 `ui.mount`。

```ts
panels.mount(slot: "bottom" | "sidebar", file: string,
             options?: { component?: string; global?: string; title?: string }): Panel
panels.bottom: PanelSlot          // 底栏位置
panels.sidebar: SidebarSlot       // 侧栏位置

interface Panel extends View, Disposable {
  readonly ready: Promise<void>;                 // 已挂载
  setInteraction(locks: Partial<{ focused; dragging; popupOpen: boolean }>): void;
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>;
}

interface PanelSlot {
  readonly state: SlotState | null;              // null：该位置没有面板
  onChange(listener: (state: SlotState | null) => void): Disposable;
  setVisible(visible: boolean): Promise<void>;   // 设置用户偏好
  toggle(): Promise<void>;                       // 按播放器当前值翻转偏好
}
interface SidebarSlot extends PanelSlot {
  setDetached(detached: boolean): Promise<void>; // 分离为浮动窗口 / 附着回主窗口
}
interface SlotState {
  preferred: boolean;   // 用户的选择（菜单勾选）
  effective: boolean;   // 现在是否显示（附着或在浮动窗口里）
  reason: "userHidden" | "space" | "fullscreen" | "idle" | null;
  detached: boolean;    // 侧栏为浮动窗口；底栏总为 false
}
```

- 面板文件导出**一个 component 和一个 global**。属性和回调声明在 global 上（桥接对象），component 负责布局。名字由 SDK 从文件推断；文件导出多个时用 `options` 指定。`title` 是侧栏分离后浮动窗口的标题。
- `panels.bottom` / `panels.sidebar` 不属于挂载者：任何插件都能读取和切换（例如底栏按钮开关播放列表）。位置为空时 `setVisible` 等以 `missing` 失败；对底栏 `setDetached` 是 `invalid`。没有 `events` 时 `state` 只在本插件自己改动后刷新。侧栏的显示、宽度、分离状态与浮动窗口位置由播放器跨启动记住。
- 侧栏占视频右侧全高，用户可拖动分隔条改宽度（侧栏至少 317、视频至少 318）；窗口窄到放不下时先压缩视频，再不够才隐藏（`reason: "space"`，偏好不变）。分离后浮动窗口默认 300×408、最小 300×250，显示同一个面板组件，插件的属性和回调不变；点浮动窗口的关闭按钮等于 `setVisible(false)`。
- `showMenu`：在面板当前所在的窗口弹出单层菜单，列出本插件的命令（`null` 为分隔线），样式、勾选、快捷键文字与主菜单相同；选择一项等于执行该命令。`at` 是窗口坐标，在 Slint 里取 `self.absolute-position.x / 1px + self.mouse-x / 1px`（y 同理）。命令不属于本插件以 `missing` 失败，面板未显示以 `unavailable` 失败，最多 32 项。
- `setInteraction`：用户正在操作面板（输入框聚焦、拖动、弹出菜单打开）时设为 `true`，防止底栏自动隐藏。合并写入，最后写入生效。
- 面板转发 global 上的所有回调，没有「先绑定再显示」的限制。
- 底栏面板的内容可以画到槽位上方的视频区（负 `y`），宿主不裁切；侧栏与浮动窗口在其上层。进度条悬停时间提示这类内容可以直接画在底栏组件里（宿主的组合测试保证底栏外层不带 `clip`）。

### 6.4 Slint 文件规则

- 工具窗口文件恰好导出一个组件（通常 `inherits Window`）；不带 `export` 的辅助组件不限。
- 插件 → 界面用 `in` / `in-out property`，界面 → 插件用 `callback`。回调参数和属性值必须能用 JSON 表示（基本类型、数组、结构体），回调返回 `void`。
- 会被插件改值的输入控件用双向绑定（`value <=> root.x`、`checked <=> root.x`）。单向的 `value: root.x` 在用户操作控件后会断开，之后插件改值，控件不会跟着动。属性须为 `in-out`。
- `in` 属性只能单向绑定时，用 `changed x => { slider.value = root.x; }` 在模型变化时回推。
- 主题与控件：`import { Theme } from "@vestra/theme.slint";` 提供颜色（`bg-bar`、`bg-panel`、`text`、`accent`、`playing`…）、字号（`font-xs` 9px、`font-body` 11px、`font-tab` 12px、`font-title` 13px、`font-mono` 等宽（Menlo，只含西文，中文不要用它））和控件尺寸（`button-height` 21px、`input-height`、`form-row-gap`、`dialog-padding`…），完整列表见 `src/host/ui/vestra/theme.slint`。`import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint";` 提供 PotPlayer 样式的按钮（文字按钮 21px、`small` 18px；`text` 为空时是图标按钮：`icon` 是 Slint `Path` 的路径数据，画在 `icon-width` × `icon-height`（默认 14×12）的 viewbox 里，`stroked: true` 画 1.5px 线而不填充，`checked` 表示开启态；图标按钮的宽高由调用方设置，悬停提示用 `ToolTipArea` 包住）、2px 线滑块（`panel: true` 为面板样式）和悬停提示。
- 所有插件 Slint 都以 `fluent-dark` 风格编译：std-widgets 的控件默认是深色。默认字体是系统 UI 字体（含中文回退），插件不需要自己设字体。
- 编辑器里要让 `@vestra/...` 导入可解析，给 Slint 语言服务设 `libraryPaths`：`{ "vestra": "<仓库>/src/host/ui/vestra" }`。

### 6.5 屏幕提示 `plugin.osd`

需要 `ui.osd`。

```ts
osd.show(text: string): Promise<void>   // 视频左上角一行，2 秒后淡出；新提示替换旧提示
osd.panel(): OsdPanel                   // 本插件的只读信息块，每个插件一个
interface OsdPanel extends Disposable {
  set(text: string | null): void;       // 多行文字；null 隐藏；最后写入生效
}
```

- 提示：1–200 个字符，换行替换为空格；粗体 16px 白字黑描边，左/上边距 10px。插件停用时已显示的提示照常到时消失。
- 信息块：位于提示一行之下，半透明黑底、11px 等宽字，不接收输入，最多 32 行、4096 字节；多个插件各一块时从上到下排列。`dispose` 或插件停用时移除。第二次调用 `panel()` 抛错。
- 无界面模式下以 `unavailable` 失败（`panel().set` 的失败交给 `onError`）。

### 6.6 主窗口 `plugin.window`

```ts
interface MainWindow {
  readonly state: WindowState;   // 同步只读，启动时已读好
  onChange(listener: (state: WindowState) => void): Disposable;
  setVideoArea(size: { width: number; height: number }): Promise<{ width; height; clamped: boolean }>;
  setAlwaysOnTop(onTop: boolean): Promise<void>;
  setCursorAutoHide(mode: "fullscreen" | "playing"): Promise<void>;
}
interface WindowState {
  readonly fullscreen: boolean;
  readonly videoArea: { readonly width: number; readonly height: number }; // 逻辑点
}
```

- `state`、`onChange` 不需要能力；三个 `set…` 需要 `window.control`。`videoArea` 是视频区（不含底栏、侧栏、分隔条），全屏时是整个屏幕；无界面模式为 0×0。有 `events` 时随窗口变化更新（从插件第一次读 `state` 或调用 `onChange` 开始跟随），没有 `events` 时只在启动和本插件的 `setVideoArea` 之后更新。
- `setVideoArea`：把窗口调成视频区为 `width`×`height`（1–16384 逻辑点；1 倍 = 一个视频像素一个点）。播放器加上底栏和附着的侧栏（侧栏宽 + 3），保持窗口左上角，再把窗口挪回屏幕可见区（不含菜单栏和 Dock）。放不下时按比例缩小；小于最小尺寸（视频区宽 318、高 120，窗口 320×182）时放大到最小值（会有黑边）；这两种情况 `clamped: true`。resolve 时布局已是新尺寸（最多等 1 秒），返回实际视频区，`state` 已刷新。全屏时与无界面模式 `unavailable`（`message`「全屏时不可用」）。
- `setAlwaysOnTop`：`true` 时播放器的普通窗口（主窗口、浮动侧栏、工具窗口，包括之后新开的）浮在其他应用的窗口之上（`NSFloatingWindowLevel`）；系统对话框不受影响。多个插件时最后一次调用生效，该插件停用后回到其余插件中最后的值（都没有则不置顶）。无界面模式 `unavailable`。
- `setCursorAutoHide`：`"fullscreen"`（默认）只在全屏时隐藏指针；`"playing"` 在窗口模式也隐藏。隐藏条件不变：播放中（就绪且未暂停）、指针在视频区、3 秒内没有指针移动或按键、没有菜单或系统对话框、面板没有在操作；窗口模式另需主窗口是活动窗口。多个插件时最后一次调用生效，该插件停用后回到其余插件中最后的值（都没有则 `"fullscreen"`）。
- 测试：`host.mainWindow`（`{ fullscreen, videoArea, onTop, cursorAutoHide }`）；`host.setMainWindow({ fullscreen?, videoArea? })` 模拟用户改变窗口并推送。

## 7. 对话框 `plugin.dialogs`

需要 `ui.dialog`。系统原生对话框；没有时长限制，用户选择或取消才结束。同一时间只有一个对话框（文件对话框与确认框共用），第二个以 `busy` 失败；无界面模式 `unavailable`。

```ts
dialogs.confirm(options: {
  message: string;        // 1–200 个字符，粗体
  detail?: string;        // ≤ 2000 个字符，正文
  confirm: string;        // 确认按钮（默认按钮，回车），1–40 个字符
  cancel?: string;        // 取消按钮（Esc），1–40 个字符，默认「取消」
  destructive?: boolean;  // 确认按钮标为破坏性操作（红字）
  checkbox?: string;      // 文字下方的复选框，1–40 个字符
}): Promise<{ confirmed: boolean; checked: boolean }>
```

- 系统提示框（`NSAlert`），以主窗口为父窗口的表单（sheet）；主窗口不可用时为应用模态窗口。`confirmed`：用户点了确认按钮；`checked`：复选框的状态（没有复选框为 `false`，取消时也如实返回）。长度按 Unicode 字符计（一个汉字一个字符），超出 `invalid`。
- 「不再询问」之类的记忆由插件自己保存（`storage.json`），播放器不记。
- 测试：`host.dialogAnswers.push({ confirmed, checked })`；`host.confirms` 记录每次的选项。

文件对话框：

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

| 节点 | 字段与范围 | FFmpeg | 播放中原地修改的字段 |
| --- | --- | --- | --- |
| `{ kind: "eq", brightness, contrast, saturation, gamma }` | brightness −1–1，contrast 0–2，saturation 0–2，gamma 0.5–2 | `eq` | 全部 |
| `{ kind: "gblur", sigma, steps: 3 }` | sigma 0–10 | `gblur` | sigma |
| `{ kind: "hue", degrees }` | −180–180 | `hue=h` | degrees |
| `{ kind: "sharpen", amount, size }` | amount 0–2；size `"small"`（3×3）\| `"large"`（7×7） | `unsharp`（亮度） | — |
| `{ kind: "denoise", lumaSpatial, chromaSpatial, lumaTemporal, chromaTemporal }` | 各 0–20（0 = 不处理） | `hqdn3d` | — |
| `{ kind: "deblock", strength }` | `"weak"` \| `"strong"` | `deblock` | — |
| `{ kind: "deband", threshold, range }` | threshold 0.003–0.5（四个平面相同），range 1–64 的整数 | `deband` | — |
| `{ kind: "transform", rotate, flipH, flipV }` | rotate 0 \| 90 \| 180 \| 270（顺时针），先旋转再翻转 | `transpose` / `hflip` / `vflip` | — |
| `{ kind: "deinterlace", when, output }` | when `"interlaced"`（只处理标记为隔行的帧）\| `"all"`；output `"frame"` \| `"field"`（帧率加倍） | `bwdif` | — |
| `{ kind: "reframe", mode, aspect, x, y }` | mode `"crop"` \| `"pad"`；aspect（显示宽高比）0.25–4；x、y −1–1（−1 左/上、0 居中、1 右/下） | `crop` / `pad`（黑边） | — |
| `{ kind: "volume", gainDb }` | −60–0 | `volume` | gainDb |
| `{ kind: "equalizer", frequency, q: 1, gainDb }` | frequency 20–20000，gainDb −6–6 | `equalizer` | gainDb |
| `{ kind: "treble", frequency: 3000, slope: 0.5, gainDb }` | gainDb 0–6 | `treble` | gainDb |
| `{ kind: "dynaudnorm", enabled, maxGain, peak }` | maxGain 1–4，peak 0.5–0.95 | `dynaudnorm` | 全部 |
| `{ kind: "channels", mode }` | `"stereo"`（不改）\| `"left"` \| `"right"` \| `"mono"` \| `"swap"`；多于两声道时先混成立体声 | `aformat` + `pan` | — |
| 音频 `delayMs` | −10000–10000，10 的倍数；全部插件的正、负延迟各自合计不超过 10000 | — | — |

- 「—」的字段改值时宿主重建本阶段的处理链（画面可能停顿一帧，音频处理状态复位）；拖动滑块的控件用表中可原地修改的字段。
- 画面节点作用于宿主的方向处理（容器旋转标记）之后：竖拍视频先被摆正，`transform` 再在摆正后的画面上旋转。
- 所有插件合计最多 64 个节点。video 插件同时打开全部效果时的最长链：eq、hue、sharpen、denoise、deblock、deband、deinterlace、transform、reframe、gblur 各一个，共 10 个节点（transform 最多展开为 3 个 FFmpeg 滤镜）。
- 「含效果」截图与导出（§10）使用同一条链，与播放画面一致。

## 9. 字幕 `plugin.subtitles`

读取需要 `subtitles.read`，修改需要 `subtitles.control`。

```ts
interface Subtitles {
  readonly state: SubtitlesState | null;  // 首次读取前为 null
  onChange(listener: (s: SubtitlesState) => void): Disposable;
  refresh(): Promise<SubtitlesState>;
  select(track: SubtitleTrack | null, opts?: { role?: "primary" | "secondary"; ifCurrent? }): Promise<SubtitlesState>; // null = 关闭
  load(bytes: Uint8Array, opts?: { format?: "srt" | "ass"; title?: string;
       select?: "primary" | "secondary" | "none"; ifCurrent?; signal? }): Promise<SubtitleTrack>;
  remove(track: SubtitleTrack, opts?): Promise<SubtitlesState>;       // 只能删本插件载入的
  setStyle(style: SubtitleStyle | null, opts?): Promise<SubtitlesState>; // null = 撤销本插件的样式层
}

interface SubtitlesState {
  readonly sourceId: number;
  readonly phase: string;
  readonly tracks: SubtitleTrack[];
  readonly selected: SubtitleTrack | null;
  readonly secondary: SubtitleTrack | null; // 次字幕，与 selected 同时显示
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
  codec: string | null;    // 后端的编解码器名，如 "subrip"、"ass"、"hdmv_pgs_subtitle"
  text: boolean;           // 文本字幕（见下表），否则为图形字幕
  streamIndex: number | null; // 内嵌轨在容器中的流号（ffmpeg -map 0:<n>）
}

type SubtitleStyle = {
  fontId: string;          // 取自 state.fonts
  fontSize: number;        // 16–72
  color: "white" | "yellow" | "cyan";
  outline: number;         // 0–4
  marginPercent: number;   // 2–15
  offsetMs: number;        // −600000–600000
  bold?: boolean;          // 默认 false
  shadow?: number;         // 阴影偏移 0–4（1080p 参考），50% 黑；默认 0
  box?: number | null;     // 背景框不透明度 0–100；null/省略 = 无框
  assOverride?: "keep" | "force"; // 默认 "keep"
  secondaryPosition?: "top" | "aboveMain"; // 次字幕位置，默认 "top"
};
```

- `select`、`remove`、`setStyle` 默认以提交前读到的当前选择为条件；`opts.ifCurrent` 传入 `SubtitlesState` 可绑定到更早的选择。
- `load` 上传字幕文件的字节、校验、提交，默认选中（`select: "none"` 只加入轨道列表，选择不变）。`signal` 在提交之前中止会丢弃上传并以 `AbortError` reject；提交发出后不再中止。
  - `format: "srt"`（默认）：UTF-8 SRT，≤ 2 MiB，≤ 10 000 条。
  - `format: "ass"`：ASS 或 SSA 脚本原样交给渲染器（保留 `\pos`、`\fad` 等标签与脚本样式）；校验 UTF-8、≤ 2 MiB、含 `[Script Info]` 与 `[Events]`、1–20 000 行 `Dialogue:`。ASS 外部轨暂不能进入「含字幕」截图（`media.capture` 以 `unsupported` 失败）。
  - `title`：1–200 个字符，作为轨道标题；省略时由播放器命名。
  - VTT、SAMI 等其它格式由插件自己转换成 SRT 再载入。
- `text` 判定：`codec` 为 `subrip`、`ass`、`ssa`、`webvtt`、`mov_text`、`text` 时为 `true`，其余（含 `null`）为 `false`。插件载入的 SRT 在后端是 `webvtt`。
- `streamIndex`：只有内嵌轨、且播放器能确认 FFmpeg 流号时才有值（本地文件经宿主的 lavf 读取路径打开时）；外部轨、网络来源与其它情况一律 `null`。可直接用于 `tools.run("ffmpeg", ["-i", 源路径, "-map", \`0:${streamIndex}\`, …])`。
- 样式按 1080p 参考高度缩放。`box` 有值时画背景框，阴影不生效；`shadow` 只在没有框时生效。
- 次字幕：`select(track, { role: "secondary" })` 设置（`null` 关闭），`load(bytes, { select: "secondary" })` 载入后作为次字幕。不能与主字幕是同一条（两个方向都 `invalid`），图形字幕不能作次字幕（`invalid`）；共用 `trackConfirmation`；换来源时清空；移除其外部轨时一并清空。「含字幕」截图只含主字幕。
- `secondaryPosition`：`"top"` 在画面顶部；`"aboveMain"` 在主字幕上方（按底边距加两行主字幕的高度估算，mpv `secondary-sub-pos`）。随生效的样式层（最后设置样式的插件）。
- `assOverride`：`"keep"`（默认）时 ASS 字幕保留脚本自己的字体、颜色、位置，只按窗口缩放，样式只作用于 SRT 等文本字幕；`"force"` 时样式强制覆盖 ASS 脚本样式。

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

`validate`、`rollback`、`setEnabled`、`uninstall` 需要 `plugins.manage`（插件工坊、插件管理）；`list`、`onChange` 不需要能力（关于窗口等也可以读）。

```ts
interface Packages {
  validate(directory: string): Promise<PackageCandidate>; // 校验目录中的包
  rollback(id: string): Promise<PackageChange>;           // 回退到上一版本
  list(): Promise<PluginInfo[]>;
  onChange(listener: (plugins: PluginInfo[]) => void): Disposable;
  setEnabled(id: string, enabled: boolean): Promise<void>;
  uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>;
}
interface PluginInfo {
  id: string; version: string | null;
  origin: "installed" | "development" | "trial";   // 安装库 / --plugin 开发加载 / 试载
  state: "starting" | "running" | "disabled" | "failed" | "stopped";
  error: string | null;     // failed 时的原因
  capabilities: string[];
  directory: string;        // 包目录；数据目录按 §3.4 规则推出
  previousVersion: string | null; // 可回退到的版本
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
- `list`：运行中的插件、安装库里的全部插件（含停用、启动失败、未运行的）与本次运行的开发加载，按 id 排序。`onChange` 在列表变化（启动、就绪、停用、失败、安装、卸载）后收到新列表，约每 200 ms 至多一次。
- `setEnabled(id, false)`：停止该插件（与播放器停用插件相同，§3.2）；已安装的插件写进安装库，之后启动播放器也不再运行，直到 `setEnabled(id, true)`；开发加载只在本次运行有效。`setEnabled(id, true)` 重新加载它。停用自己 `invalid`，试载 `unsupported`（用试载句柄停止），未知 id `missing`，正在安装 `busy`。安装或回退一个停用的插件会让它重新启用。
- `uninstall`：只对已安装插件（开发加载、试载 `unsupported`；未知 `missing`；自己 `invalid`）。先停止，再从安装库删除（当前与上一版本，一次原子提交），回收它的包文件；`removeData: true` 时在进程结束后删除它的数据目录。用户覆盖层里它的绑定保留（`bindings.list` 的 orphans）。返回的 `PackageChange` 通常已是 `committed`。
- `trial.stop()` 以 `code: "cleanup"` 失败表示播放器停止了试载但未确认清理；再次调用会重新请求停止。

## 13. 错误

失败一律是普通 `Error`，带稳定的 `code`（类型 `PluginError = Error & { code: string }`）。按 `code` 判断，不要按 `message` 文本判断。

| code | 含义 |
| --- | --- |
| `stale` | 条件不再成立（来源或字幕选择已被改变），**什么都没做** |
| `missing` | 面板位置为空；`showMenu` 的命令不属于本插件；`system.reveal` / `openPath` 的路径不存在；`clipboard.writeImage` 的文件不存在 |
| `capability` | manifest 未声明所需能力 |
| `invalid` | 参数不合法（如滤镜值超出范围） |
| `busy`、`conflict`、`unavailable`、`unsupported` | 播放器拒绝接纳，未执行（`conflict` 也用于已有别的播放序列提供者；`unavailable` 也用于 `next`/`previous` 不可用、面板未显示、无界面模式） |
| `backend` | 载入或解析失败，`message` 为后端原因 |
| `disabled` | 插件正在停用 |
| `replaced` | 已被接受，但完成前被另一个来源替换（`open`、`openStream`、`media.ready`） |
| `failed`、`ended` | `media.ready()`：载入失败 / 已播完。`failed` 也用于 `tools.run` 非零退出（`message` 为 stderr 最后一行）、`system.*` 打开失败、剪贴板被系统拒绝写入 |
| `aborted` | `signal` 中止（`error.name === "AbortError"`） |
| `timeout` | `openStream` 启动超时；`error.stage` 为 `"prefill"`（首批缓冲未在 10 秒内备好）或 `"activation"`（选中后 10 秒内未激活）。`tools.run` 超过 `timeoutMs`；`system.*` 超过 10 秒 |
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
| 操作 | `host.command(id).run()`（禁用或忙碌时同样拒绝）；`host.press(binding, { panel? })`（按绑定表执行，`panel: true` 模拟面板有焦点）；`host.window(id).emit(callback, ...args)`；`host.mounted().emit(...)`；`host.dialogAnswers.push([path] \| null)`；`host.step("next" \| "previous")`（模拟别的插件调用 `player.next()`） |
| 模拟来源 | `host.openElsewhere(path, phase?)`（另一个插件打开了媒体）；`host.setSource({ phase, position, duration, … })` |
| 断言 | `host.window(id).props`（kebab-case 名）、`.visible`；`host.command(id).state`、`.declaration`；`host.conflicts`（`[绑定, 赢家, ...被压下]`）；`host.layout.bottom` / `.sidebar`（可直接改后 `host.emit("window.state", {})` 模拟别处的变化）；`host.osd`（显示过的提示）、`host.osdPanel`；`host.sequence`（提供者的标志）；`host.menus`（`showMenu` 记录）；`host.sent(method)`；`host.source`；`host.processing`、`host.subtitles`、`host.snapshots`、`host.jobs`、`host.producers` |
| 模拟数据 | `host.stats`（`player.stats()` 的返回值，第 2 轮新增字段可省略，SDK 补 `null`）；`host.dataDirectory`（`plugin.dataDirectory`，`storage.json` 直接读写其中的真实文件）；`host.setSource({ audioTracks })`（`addAudioTrack` / `selectAudioTrack` 有默认处理，可用 `host.handle("audio.addTrack" \| "audio.selectTrack", …)` 注入失败）；`host.subtitles.tracks` 的条目可带 `codec`、`text`、`streamIndex` |
| 通用能力 | `host.clipboard`（`{ text, images }`，预置 `text` 测 `readText`）；`host.power`（合并后的 `"off"` / `"playing"`）；`host.tools.handle(tool, (args) => ({ stdout, stderr }))`（未注入的工具 `unavailable`；抛 `HostError` 模拟失败），`host.tools.calls`；`host.system.revealed`、`.opened`；`host.subtitleLoads`（每次 `load` 的 `format`、`title`、`select` 与字节）；`host.system.urls`（`system.openUrl`） |
| 第 3a 轮批次 2 | `host.bindings.overlay`（`"<plugin>/<command>"` → 输入，可预置；`bindings.set` 写入，`host.press` 按它匹配，可 `host.press("Mouse:LeftClick")`）；`host.packages`（`packages.list` 的返回值）、`host.setPackages(list)`（推送 `onChange`）、`host.packageCalls`；`host.audioDevices`（`setAudioDevice` 更新 `host.source.audioOutput`）；`host.subtitles.selection.secondary` |
| 第 3a 轮 | `host.mainWindow`、`host.setMainWindow(patch)`（§6.6）；`host.app`（§3.5）；`host.dialogAnswers.push({ confirmed, checked })`、`host.confirms`（§7）；`host.chapters`（`player.chapters()` 在媒体就绪时返回它）；`setLoop` 写入 `host.source.loop`，打开新来源时清除 |
| 核心动作 | `host.coreAction(id)`、`host.coreCommands`（播放器的 14 条核心命令，§5.5；`press` 先匹配它们）；`host.coreRuns`；`host.stepSeek`；`host.steps` |
| 注入故障 | `host.handle(method, fn)`：抛 `new HostError(code, message)` 模拟播放器拒绝，抛普通 `Error` 模拟答复丢失；`host.reset(method)` 恢复 |
| 同步 | `await host.settle(ms?)`：等到没有在途请求（可额外等待计时器） |
| 结束 | `await stop()` |

语义是简化的：没有解码，没有 Slint 渲染，导出任务每次查询前进一步。处理节点与音频延迟按播放器的同一套范围校验（越界的 `set` 以 `invalid` 进入 `failed`）。

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
- `check` 也检查 manifest 的能力名（与宿主同一张表，§2）。

## 16. 通用能力

很多插件都需要、行为应当一致的基础能力。剪贴板与阻止休眠由播放器提供；外部工具、设置文件、Finder 由 SDK 在插件进程里实现，不经过播放器。插件里不要再用 `pbcopy`、`caffeinate`、`open` 等命令自己实现。

### 16.1 剪贴板 `plugin.clipboard`

需要 `clipboard`。

```ts
clipboard.readText(): Promise<string | null>;      // 没有文本为 null
clipboard.writeText(text: string): Promise<void>;  // ≤ 1 MiB（UTF-8）
clipboard.writeImage(path: string): Promise<void>; // PNG/JPEG（按文件内容判断），≤ 16 MB，绝对路径
```

- 写入替换剪贴板的全部内容。文本以 `public.utf8-plain-text`、图片以 `public.png` / `public.jpeg` 写入系统剪贴板，可在文本编辑、预览等应用中粘贴。
- 失败：`invalid`（类型、大小、非绝对路径、不是 PNG/JPEG）、`missing`（图片文件不存在）、`unavailable`（无界面模式）、`failed`（系统拒绝写入）。

### 16.2 阻止休眠 `plugin.power`

需要 `playback.control`。

```ts
power.preventSleep(mode: "off" | "playing"): Promise<void>;
```

- 任一插件要求 `"playing"` 即生效；插件改回 `"off"` 或停用时撤销它自己的要求。
- 生效期间，来源阶段为 `ready` 且未暂停（含生成流）时，播放器持有系统断言：正在显示视频帧时阻止显示器睡眠（`caffeinate -d -i`），纯音频只阻止系统睡眠（`-i`）；暂停、播完、关闭来源时释放。断言绑定播放器进程，播放器退出时随之结束。
- 可用 `pmset -g assertions` 查看。
- 失败语义：`preventSleep` 只登记要求（参数不对时 `invalid`），断言在播放条件满足时由播放器持有。启动 `/usr/bin/caffeinate` 失败只写播放器日志（`host.error power …`），不通知插件，相同条件下不重试，条件变化（暂停/继续、有无视频）时再试。插件不能据此提示「无法阻止屏保」。

### 16.3 外部工具 `plugin.tools`

```ts
tools.run(tool: "ffmpeg" | "ffprobe", args: string[],
          options: { timeoutMs: number; maxOutput?: number; signal?: AbortSignal }):
  Promise<{ stdout: Uint8Array; stderr: string }>;
tools.available(tool: "ffmpeg" | "ffprobe"): Promise<boolean>;
```

- 可执行文件按环境变量 `VESTRA_FFMPEG` / `VESTRA_FFPROBE`（绝对路径），再按 `PATH` 查找；找到的结果在本插件内缓存，找不到时下次重新查找。
- 不经 shell：参数原样传给程序，路径里的空格、引号不需要转义。
- 超过 `timeoutMs`、`signal` 中止、所在作用域释放（含插件停用）都会结束子进程：先 SIGTERM，300 ms 后仍在则 SIGKILL。
- `stdout` 最多 `maxOutput` 字节（默认 16 MiB），超出时结束进程并以 `invalid` 失败；`stderr` 保留最后 64 KiB。
- 失败：`unavailable`（找不到）、`timeout`、`failed`（非零退出，`message` 为 stderr 最后一行）、`invalid`、`aborted`。
- 调用方按工作量给超时；没有固定上限。

### 16.4 设置文件 `plugin.storage`

```ts
storage.json<T>(name: string, options: {
  defaults: T;
  validate?: (value: unknown) => value is T;
  debounceMs?: number;            // 默认 300
}): JsonStore<T>;

interface JsonStore<T> extends Disposable {
  readonly value: T;
  save(next: T): void;            // 立即更新 value，debounceMs 后写入
  flush(): Promise<void>;         // 立即写入待写的值
}
```

- 文件为 `dataDirectory/<name>.json`；`name` 只能是 1–64 个 `a-z`、`0-9`、`-`，否则 `invalid`。
- 创建时同步读取。文件不存在用 `defaults`；解析失败或 `validate` 返回 `false` 时，把原文件改名为 `<name>.json.bad-<毫秒时间戳>` 留给用户，并用 `defaults`。
- 写入：临时文件 + 改名，按调用顺序一次一个，最后一次 `save` 的值最后落盘。文件权限 0600（只有当前用户可读写），设置里常有私人路径与播放记录。
- `dispose()` 与插件停用时先写完待写的值。只有 `save` 会写入：原地修改 `value` 的属性不会保存。

### 16.5 Finder 与浏览器 `plugin.system`

```ts
system.reveal(paths: string | string[]): Promise<void>; // 在 Finder 中选中（一次 /usr/bin/open -R a b c）
system.openPath(path: string): Promise<void>; // 用默认应用打开文件或文件夹（/usr/bin/open）
system.openUrl(url: string): Promise<void>;   // 用默认浏览器打开 http(s) 地址（/usr/bin/open）
```

- `reveal`：最多 256 个路径，一次调用全部选中（同一文件夹的多个文件在同一窗口高亮）；不存在的路径跳过，全部不存在时 `missing`。路径先转为绝对路径。
- `openPath` 路径不存在 `missing`。
- `openUrl` 只接受 `http:` / `https:` 地址（按 WHATWG URL 解析并规范化，≤ 8192 字符），其它协议或格式不对为 `invalid`；不检查网址能否访问。
- 三者 `open` 超过 10 秒 `timeout`，失败 `failed`。

## 17. 变更说明

### 第 2 轮批次 A（2026-10）

兼容扩展：`PlaybackStats` 新字段；`player.addAudioTrack`、`selectAudioTrack`；`subtitles.load` 的 `format`、`title`、`select`；`SubtitleTrack.codec`、`text`、`streamIndex`；`SubtitleStyle.bold`、`shadow`、`box`、`assOverride`；8 种视频节点与 `channels` 音频节点；`clipboard`、`power`、`tools`、`storage`、`system`；绑定的小键盘名。

范围放宽：`SubtitleStyle.offsetMs` −600 000–600 000；音频 `delayMs` −10 000–10 000（正、负合计各 10 000）。

默认行为变化（不改签名，但效果可见）：

- **ASS 字幕默认保留脚本样式**：宿主不再在启动时设置 `sub-ass-override=force`，默认是 `keep`（mpv `scale`）。内嵌或外部 ASS 字幕按脚本自己的字体、颜色、位置显示；设置了字幕样式的插件若需要覆盖 ASS，传 `assOverride: "force"`。SRT 等文本字幕不受影响。
- **小键盘与主键盘分开**：小键盘的数字、加减号、回车不再匹配 `"5"`、`"Plus"`、`"Enter"` 等主键盘绑定。
- **Option 与 Ctrl 组合按键名识别**：`Alt+字母` 在 macOS 上可用；⌃Y、⌃I、⌃H、⌃M、⌃[ 不再被识别成 Shift+Tab、Tab、Backspace、Enter、Escape。
- **来源阶段变化立即推送**：`source.state` 在阶段变化（如 `ready → ended`、新来源就绪）时立即发出，不再等 100 ms 周期；周期推送不变。`media.onEnd` 之后打开下一项会更快。

### 第 2 轮批次 B：基础播放动作移交核心（2026-10）

新增：`plugin.host.run`（§5.5）、`player.step`、`player.setStepSeek`（§5.2）；`source.state` 增加内部字段 `hasVideo`（SDK 不对外暴露）。

**破坏性变更，集中切换**：核心动作与 playback 原有命令占用同样的菜单路径与按键。迁移期内播放器只在 `VESTRA_CORE_ACTIONS=1` 时注册核心动作；P3（与 playback 迁移同一次合并）删除了该开关，核心动作总是注册。仍注册旧路径或旧按键的插件，菜单注册以 `conflict` 失败，按键被核心动作压下。

| 旧 id（`vestra.playback/…`） | 新 id（`host/…`） |
| --- | --- |
| `toggle-pause` | `play-pause` |
| `close-source` | `stop` |
| `previous` / `next` | `previous` / `next` |
| `backward-5` / `forward-5` | `seek-backward` / `seek-forward` |
| `rate-down` / `rate-up` | `rate-down` / `rate-up` |
| `rate-10` | `rate-normal` |
| `volume-up` / `volume-down` | `volume-up` / `volume-down` |
| `mute` | `mute` |

用户覆盖层中旧键的绑定在播放器启动时一次性改写为新键并保存；新键已存在时保留新键、丢弃旧键。

迁移写法：

```ts
// 之前：playback 自己注册命令，底栏按钮调用控制器
plugin.commands.add({ id: "toggle-pause", path: ["播放", "播放/暂停"], order: 201, bindings: ["Space"], run: () => controller.togglePause() });
bar.on("playPause", () => controller.togglePause());
// 之后：命令由核心注册，底栏按钮执行核心动作
bar.on("playPause", () => plugin.host.run("play-pause"));
await plugin.player.setStepSeek({ seconds: settings.value.step }); // 第 1 档步长
```

测试：`TestHost` 像播放器一样注册这 14 条核心命令，放在 `host.coreCommands`（不进 `host.commands`，插件自己的命令断言不受影响）；`host.coreAction(id)` 取得命令，`.run()` 执行，`.state` 为启用/勾选状态；`host.press` 先匹配核心绑定（核心赢得冲突）；`host.run` 按同样规则执行并把提示写入 `host.osd`，`host.coreRuns` 记录执行过的动作，`host.stepSeek`、`host.steps` 记录 `setStepSeek` 与 `step`。插件仍声明旧命令时，`host.conflicts` 会列出与核心动作的冲突。

### 第 3a 轮批次 1：窗口、应用信息、确认框、章节与循环（2026-10）

兼容扩展（不用它们的插件不必改）：

- 新模块 `plugin.window`（§6.6）：`state`、`onChange`（不需要能力），`setVideoArea`、`setAlwaysOnTop`、`setCursorAutoHide`（新能力 `window.control`，宿主与构建工具同一张能力表）。
- `plugin.app`（§3.5）：播放器版本、构建哈希、mpv / FFmpeg / Slint 版本，同步只读。
- `plugin.dialogs.confirm`（§7，`ui.dialog`）：系统确认框，可带复选框。插件不再需要自制确认窗口。
- `player.chapters()`、`player.setLoop()`、`PlayerState.loop`（§5.1、§5.2）。

行为说明：

- 所有插件在激活前多读一次 `window.state`（与读 `player.state` 并行），所以 `plugin.window.state` 在 `activate` 里就是真实值。直接对协议计数的测试会看到这个请求。
- 来源变化（新的 `sourceId`）时播放器清除 A-B 循环（mpv `ab-loop-a/b/count`），之前不清除。
- `window.state` 事件的数据多了 `videoArea`（内部字段，`panels.*.state` 不变）。

### 第 3a 轮批次 2：绑定管理、插件管理、音频输出、次字幕（2026-10）

兼容扩展：

- `plugin.bindings`（§4.2，新能力 `bindings.manage`）；新输入 `Mouse:LeftClick`（§4.1）。
- `packages.list`、`onChange`（不需要能力）、`setEnabled`、`uninstall`（§12）；`PluginInfo`。
- `player.audioDevices`、`setAudioDevice`、`PlayerState.audioOutput`（§5.1、§5.2）。
- 次字幕：`subtitles.select(…, { role: "secondary" })`、`load({ select: "secondary" })`、`SubtitlesState.secondary`、`SubtitleStyle.secondaryPosition`（§9）。

行为说明：

- 安装库记录停用的插件，启动时跳过它们；安装或回退会清除停用标记。
- `Mouse:LeftClick` 有绑定时，视频区的左键单击要等过系统双击间隔才触发（约 0.25–0.5 秒），双击照常；没有绑定时左键行为与之前完全相同。
- 插件生成的流在 `setAudioDevice` 选择的设备上播放并跟随它；所选设备拔出时改用系统默认，不再因设备消失而结束（之前跟随系统默认、所选设备的概念不存在）。
- 新来源打开时播放器清除 mpv 的 `secondary-sid`（之前不设置，mpv 默认不选次字幕）。
