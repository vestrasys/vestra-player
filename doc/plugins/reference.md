# 插件 API 参考

本文是插件 API 的权威规格：每个模块、方法的签名、参数、返回值、错误和使用约束。入门请先读 [入门教程](tutorial.md)，按功能找写法看 [插件示例](examples.md)，设计取舍见 [SDK 设计说明](../architecture/sdk-design.md)。

类型定义的源头是 SDK 仓库 [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk) 的 `src/plugin/types.ts`（npm 包 `@vestra-player/sdk`；本仓库安装在 `src/node_modules/@vestra-player/sdk/dist/plugin/types.d.ts`）。本文与它不一致时以代码为准，并应修正本文。

- 协议版本：`vestra/1`
- 运行环境：每个插件一个 Node 进程：安装包里是播放器内置的 Node 26（`vestra/1` 期间主版本只升不降），开发构建用 `VESTRA_NODE` 或 `PATH` 上的 `node`（也要 26）。可以使用 Node 标准库、文件系统、网络和 npm 依赖。播放器用 `--no-experimental-webstorage` 启动 Node：没有 `localStorage` / `sessionStorage`（和 Node 24 以前一样），持久数据用 `plugin.storage`（§16.4）。插件是受信任的，manifest 能力是契约声明，不是安全沙箱。
- 系统：macOS 与 Windows（Windows 目前只做了编译检查，未在真机运行）。API 与系统无关；某个系统做不到的功能以 `unsupported` 失败，各节写明平台差异，汇总见 §16.7。插件不要直接调用系统命令（`open`、`sips`、`plutil`、`security` 等），用本文的能力或跨平台的纯 Node 实现。

## 1. 概览

### 1.1 入口

```ts
import type { Plugin } from "@vestra-player/sdk";

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
  "apiVersion": "vestra/1",
  "entry": "main.ts",
  "capabilities": ["commands", "ui.window"]
}
```

| 字段 | 规则 |
| --- | --- |
| `id` | 1–128 个 ASCII 字母、数字与 `.-_`。`vestra` 与 `vestra.` 开头的 id（不分大小写）只留给播放器内置插件（官方插件）：安装这样的包以 `reserved` 失败（§12）。除此之外播放器不强制前缀。第三方插件（包括托管在官方插件仓库里的）用 `<GitHub 用户名或组织>.<插件名>`，发布到官方插件仓库时必须这样取（`vestra-plugin check` 对无前缀的 id 只提示） |
| `version` | 三段非负整数 `x.y.z`；构建工具写入 `dist/manifest.json` |
| `apiVersion` | 固定 `vestra/1` |
| `entry` | 包内相对路径；开发时可为 `.ts`，构建后为 `.js` |
| `capabilities` | 只能是下表中的值，不重复 |
| `minPlayerVersion` | 可选，`x.y.z`：能加载它的最低播放器版本（播放器版本即 `plugin.app.version`，只比较三段数字）。更旧的播放器以 `versionMismatch`（「requires Vestra Player ≥ x.y.z」）拒绝；播放器先读这一项再做严格解析，所以写了新字段的包在旧播放器上报的是版本不够而不是未知字段。用到 SDK 1.1 新增的接口时写 `"0.2.0"`；用到 SDK 1.3 新增的（菜单 id、`Theme.danger`、视频空链）写 `"0.3.0"`；用到 SDK 1.5 新增的（`@vestra/icons.slint`、配色 `vestra-light`）写 `"0.4.0"` |

可选的显示字段（播放器 0.2.0；商城与插件管理显示）。`name`、`punchline`、`description`、`tags` 写英文原文，播放器 0.4.3 起用插件词典翻译后通过 `PluginInfo.display` 给出（§6.10、§12）：

| 字段 | 规则 |
| --- | --- |
| `name` | 1–64 个字符，单行，首尾无空白 |
| `author` | 1–64 个字符，同上 |
| `punchline` | 一句话简介，1–120 个字符，单行 |
| `description` | 纯文本，1–2000 个字符，可换行 |
| `tags` | 至多 8 个，每个 1–24 个字符、单行，不区分大小写不重复 |
| `icon` | 包内 PNG/JPEG 路径（按内容判断），≤ 512 KB，边长 ≤ 1024（建议 256×256）|
| `readme` | 包内 `.md` 路径，UTF-8，≤ 256 KB；省略时包里有 `README.md` 就用它 |
| `changelog` | 同上；省略时用 `CHANGELOG.md`（如有） |
| `sourceUrl` | `https:` 地址，≤ 2048 字节 |
| `license` | 1–64 个字符，建议 SPDX 表达式（如 `MIT`） |

显式写出的 `icon`、`readme`、`changelog` 找不到文件时拒绝。`vestra-plugin build` 会把它们（以及默认的 `README.md`、`CHANGELOG.md`）复制进 `dist/`：README 会显示在商城详情页，请写成面向用户的说明。

未知字段、未知能力、越出包目录的路径（含符号链接）都会被拒绝。规则表宿主（`host/src/manifest.rs`）与 SDK CLI 共用，一致性由 `tests/sdk/host-agreement.test.ts` 检查。

调用未声明能力的功能，以 `code: "capability"` 失败。

| capability | 开放的 API |
| --- | --- |
| `commands` | `commands.add`、`host.run` |
| `ui.window` | `windows.create` |
| `ui.mount` | `panels.mount`（`"bottom"`、`"sidebar"`、`"backdrop"`）、`panels.bottom` / `panels.sidebar`（任何插件读写两个面板位置）、`panels.backdrop` |
| `ui.dialog` | `dialogs.*`（含 `dialogs.confirm`） |
| `playback.read` | `player.state` 启动时读取、`player.refresh`、`player.stats`、`player.chapters`、`player.audioDevices`、`timeline.marks`；滤镜读取处理状态 |
| `events` | `player.state` / `onChange` 实时更新、面板可见性、字幕状态刷新 |
| `playback.control` | `player.open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput`、`provideSequence`、`next`、`previous`、`addAudioTrack`、`selectAudioTrack`、`step`、`setStepSeek`、`setLoop`、`setAudioDevice`；`power.preventSleep` |
| `producer.av` | `player.openStream`（另需 `playback.control`：选中生成流和打开文件一样是一次来源提交） |
| `processing.video` | `video.filter`、`video.view`、`video.render`（另需 `playback.read`；`video.filter` 还要 `events`，§8） |
| `processing.audio` | `audio.filter`（另需 `playback.read`、`events`，§8） |
| `subtitles.read` | `subtitles.state`、`onChange`、`refresh` |
| `subtitles.control` | `subtitles.select`、`load`、`remove`、`setStyle` |
| `media.read` / `media.control` | `media.capture`、`Snapshot.export`、`ExportJob`（`processed` 截图另需 `playback.read`、`subtitles.read`） |
| `source.resolve` | `resolvers.add`（文件解析器另需 `producer.av`，§11） |
| `plugins.manage` | `packages.validate`、`validateArchive`、`rollback`、`setEnabled`、`uninstall`（`list`、`onChange` 不需要能力） |
| `ui.osd` | `osd.show`、`osd.panel` |
| `clipboard` | `clipboard.readText`、`writeText`、`writeImage` |
| `window.control` | `window.setVideoArea`、`setAlwaysOnTop`、`setCursorAutoHide`、`setPictureInPicture` |
| `bindings.manage` | `bindings.*`（§4.2） |
| `timeline.marks` | `timeline.layer()`（§5.6；读取 `timeline.marks` 只需 `playback.read`） |
| `ui.drop` | `drops.provide`、`Panel.onDrop`（§6.7） |
| `ui.theme` | `appearance.set`、`appearance.setLanguage`（§6.8；读取 `appearance.theme`、`appearance.language` / `onChange` 不需要能力） |
| `ui.settings` | `settings.page`、`settings.show`（§6.9） |
| `secrets` | `secrets.get`、`set`、`delete`（§16.6） |

`tools`、`storage`（§16）在插件进程内执行，`system`（§16.5）由播放器执行，都不需要能力。读取 `window.state` / `onChange`（§6.6）、`plugin.app` 与 `app.system()`（§3.5）、`plugin.i18n.locale`（§6.10）、`ui.image`、`ui.markdown`（§6.1）不需要能力。

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
           host, clipboard, power, tools, storage, system, window,
           bindings, ui, timeline, drops, appearance, settings;
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
| `JsonStore` | 写完待写的值 |
| 进行中的 `tools.run` | 结束子进程（以 `aborted` 失败） |
| 监听（`on`、`bind`、`onChange`…） | 退订 |
| `Media` | **不**关闭媒体；需要时显式 `media.close()` |

### 3.4 数据目录 `plugin.dataDirectory`

本插件自己的可写目录（设置、记住的文件夹等），不需要能力。

- 位置（播放器 0.2.0 起）：每个**安装实例**一个目录，`<数据根>/<安装实例 GUID>/`；开发加载（`--plugin`）是 `<数据根>/development/<manifest id>/`。数据根在 macOS 是 `~/Library/Application Support/Vestra/plugin-data/`，Windows 是 `%APPDATA%\Vestra\plugin-data\`（环境变量 `VESTRA_PLUGIN_DATA` 可改）。播放器在启动插件前创建。路径的格式不属于 API。
- 安装实例：每次全新安装生成一个 GUID。同一来源再安装同 id 是升级，沿用实例与数据；从另一来源安装同 id 并确认替换时，新插件拿新的空目录，旧实例的数据保留，以后从旧来源再装回来时接上（见 §12）。插件停用、重新加载、升级、回退时都保留数据；卸载时只有 `uninstall(id, { removeData: true })` 才删除，不删时同来源重装可接上。
- 别的插件（如插件管理）要某个插件的数据目录，读 `packages.list()` 里的 `dataDirectory`。**不要**再用 `dirname(plugin.dataDirectory)` 加 id 推算（0.2.0 之前的规则，已失效）。
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
    readonly slint: string;          // 链接的 Slint 版本，如 "1.18.1"
  };
}
interface App extends AppInfo {      // plugin.app 的类型
  system(): Promise<SystemInfo>;
}
interface SystemInfo {
  readonly os: "macos" | "windows" | "linux" | "other";
  readonly name: string;             // "macOS"、"Windows 11"
  readonly version: string | null;   // macOS "26.2"（同 sw_vers -productVersion）；Windows "23H2"（DisplayVersion）
  readonly build: string | null;     // macOS "25C56"；Windows "22631.4460"
  readonly model: string | null;     // macOS hw.model，如 "Mac15,6"；Windows 固件的产品名
  readonly arch: string;             // 播放器构建的 CPU 架构："aarch64"、"x86_64"
  readonly displays: readonly {      // 主显示器在前；无界面模式为空
    readonly name: string | null;    // 系统给的名字
    readonly width: number;          // 当前模式的像素（逻辑尺寸 × scale）
    readonly height: number;
    readonly scale: number;          // 每逻辑点的像素数（Retina 为 2）
    readonly refreshRate: number | null; // Hz
    readonly primary: boolean;
  }[];
}
```

- 同步只读、不需要能力，子作用域相同。播放器启动插件进程时随握手交给它（不经请求），整个进程内不变。
- `system()`：每次调用向播放器读取（显示器可能插拔），不需要能力。系统版本与机型在播放器进程内只读一次。CPU 型号、核数和内存用 Node 的 `os`（`cpus()[0].model`、`cpus().length`、`totalmem()`、`freemem()`），播放器不重复提供。显示器：macOS 来自 `NSScreen`（名字是系统设置里显示的本地化名称，如「内建视网膜显示器」，不是 `system_profiler` 的 `Color LCD`；像素为逻辑尺寸 × `scale`；刷新率为该屏的最高帧率），Windows 来自 winit 的显示器列表（名字是设备名 `\\.\DISPLAY1`）。其它系统 `version`、`build`、`model` 为 `null`。
- `system` 是方法，不在 `Object.keys(plugin.app)` 和 `JSON.stringify(plugin.app)` 里。
- `libav` 只有 libmpv 报告的 `ffmpeg-version`（键 `ffmpeg`）：libmpv 0.41 不报告 libavcodec 等各库的版本，需要时用 `tools.run("ffmpeg", ["-version"])` 读取。
- 测试：`host.app` 在 `startPlugin` 之前设置；`host.systemInfo` 是 `system()` 的返回值（默认是一台 macOS 26.2 的 Mac）。

## 4. 命令 `plugin.commands`

需要 `commands`。

```ts
commands.add(options: CommandOptions): Command

interface CommandOptions {
  id: string;                         // 插件内唯一
  path?: (string | Text)[];           // 菜单路径（英文原文），1–8 段，最后一项是菜单项；1 段 = 顶层直接命令；省略 = 不进主菜单；第一段可写内置菜单 id，如 "@video"；带数据的段用 Text（§6.11）
  title?: string | Text;              // 菜单文字（英文原文或 Text），默认取 path 最后一项；省略 path 时必填
  order?: number;                     // 同级排序，小的在前；整数，±1 000 000 内，默认 0
  group?: string;                     // 分组名，相邻两项分组不同时画分隔线；默认 "domain"
  radio?: boolean;                    // checked 画成单选圆点而不是 ✓
  bindings?: string[];                // 默认按键/鼠标绑定，见 4.1
  focus?: "panel";                    // 绑定只在本插件面板获得焦点时生效
  repeat?: boolean;                   // 按住不放时随系统自动重复触发；默认 false
  enabled?: Bindable<boolean>;        // 默认 true
  checked?: Bindable<boolean | null>; // null：无勾选标记（默认）
  busy?: Bindable<boolean>;           // 默认 false
  reason?: Bindable<TextValue | null>; // 禁用或忙碌时显示的原因（英文原文，或 Text，§6.11）
  run(): unknown;
}
type Bindable<T> = T | (() => T);

interface Command extends Disposable {
  readonly id: string;
  set(state: Partial<{ enabled; checked; busy; reason }>): void; // 值或函数，最后写入生效
  readonly ready: Promise<void>;  // 播放器已注册该命令
}
```

- **路径是英文原文，菜单按它合并**（播放器 0.4.3）：不同插件路径前几段相同就进同一个子菜单。播放器显示时用**声明它的插件**的词典（§6.10）翻译标题和各段；播放器自己的菜单（下表的 7 个顶层菜单，以及「Playback ▸ Speed」「Playback ▸ Seek」）始终显示播放器的词。多个插件共用的子菜单要写同一个英文 key，key 登记在 [菜单表](../product/menus.md)。`reason` 也按插件词典翻译。
- **内置菜单**：第一段写 id（`@video`）或 key（`Video`）都进播放器的菜单：

  | id | key | 0.4.2 及以前的标题（仍可用） |
  | --- | --- | --- |
  | `@open` | Open | 打开 |
  | `@playback` | Playback | 播放 |
  | `@subtitles` | Subtitles | 字幕 |
  | `@video` | Video | 视频 |
  | `@audio` | Audio | 声音 |
  | `@screen` | Screen | 屏幕 |
  | `@settings` | Settings | 设置 |

  旧中文标题是别名：`["视频", "Blur"]` 与 `["@video", "Blur"]`、`["Video", "Blur"]` 是同一个菜单项（重复注册一样以 `conflict` 失败）；「播放 ▸ 速度」「播放 ▸ 跳转」「设置 ▸ 外观」也分别是 `Playback ▸ Speed`、`Playback ▸ Seek`、`Settings ▸ Appearance` 的别名（只改有下一级的段）；旧标题的段照原文显示。只有第一段按 id 解释，后面的段照常是原文；未知的 id（`@view`）或 id 后面没有菜单项（`["@video"]`）以 `invalid` 失败。0.4.2 及以前的播放器标题是中文，`["Video", "Blur"]` 在那里是一个单独的顶层菜单，给这些播放器的插件写 `@video`；0.3.0 之前的播放器把 `@video` 当作普通标题，插件要设 `"minPlayerVersion": "0.3.0"`。SDK 常量 `MENU_IDS`（从 `@vestra-player/sdk/testing` 导入）列出全部 id，与播放器一致。绑定表（§4.2）里的 `title`、`path` 是显示出来的文字；用户改的绑定按插件 id 与命令 id 保存，与路径无关。
- 菜单排序：同级按 `order` 升序；子菜单（路径中间的一段）排在它下面最靠前的命令的位置，并取该命令的 `group`。`order` 相同时按插件 id、再按同一插件的声明顺序，**不按标题排序**。`group` 只决定分隔线，不参与排序：一个分组要连在一起，它的 `order` 也要连续。
- **内置菜单的位置固定**（播放器 0.3.0）：上表 7 个内置顶层菜单不按里面的命令排位置，而按播放器固定的顶层 `order`：打开 110、播放 200、字幕 300、视频 400、声音 500、屏幕 710、设置 830。不论写 id（`["@video", "Blur…"]`）还是直接写标题（`["视频", "Blur…"]`），插件的菜单项都不会把内置菜单挪走。在内置菜单的**直接下一级**，`order` 照常排序，只有 0（即省略 `order`）表示「排在最后」：排在所有非 0 项之后，多个 0 再按插件 id、声明顺序。内置项都用 100 以上的 `order`，所以省略 `order` 的插件项排在内置项之后；要排进内置项之间，按 [菜单表](../product/menus.md) 取相邻的 `order`；负数排在最前。再往下一级（插件自己的子菜单里）0 就是普通的 0。`group` 默认 `"domain"`，与相邻内置项的分组不同时会画分隔线；同一插件的几项要连成一组，给它们同一个 `group`。0.3.0 之前的播放器把内置菜单也排在其中最小的 `order` 处，直接写标题、省略 `order` 的项会把整个菜单挪到顶部；要在这些播放器上也不挪动，写该菜单范围内的 `order`（如视频 400–499）。
- 推荐顶层骨架（[doc/product/menus.md](../product/menus.md) 维护顶层编号）：`open` 100–199（打开、关闭）、`domain` 200–699（播放、字幕、视频、声音、模拟器等子菜单）、`screen` 700–799（宿主「全屏」700）、`windows` 800–899（播放列表、播放信息等窗口入口）、`exit` 900–999（宿主「退出」990）。
- 同一路径不能注册两次，也不能既是菜单项又是子菜单（`conflict`）。路径、`order`、`group`、绑定格式不合法时 `commands.add` 立即抛出 `invalid`。
- 菜单项右侧的快捷键文字由绑定表生成（第一个生效的键盘绑定）。菜单宽度按文字估算：中日韩等宽字符算一个字宽，其它字符（含带变音符号的拉丁字母）算半个。
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
- 命令禁用或忙碌时，播放器不会调用 `run`。忙碌时绑定的按键被接住但不执行；禁用时按键让出（§4.1）。`run` 抛出的错误交给 `onError`。
- **数量上限**：每个插件实例同时最多 160 条命令，播放器全部（所有插件加上播放器自己的命令）最多 1024 条；超出时该命令的 `ready` 以 `limit` 失败（在激活期间注册的，激活随之失败），已注册的不受影响，`dispose` 后名额立即释放。上限只防失控（如按轨道数无限添加），不是产品限制：内置插件里最多的 video 在空闲时注册 71 条，全部插件约 290 条。按媒体内容动态添加的菜单项（轨道、章节、预设列表）请自行设一个合理的显示上限。常量 `COMMAND_LIMITS`（`{ perPlugin, total }`，从 `@vestra-player/sdk/testing` 导入）与播放器一致，`TestHost` 执行同样的上限。

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
- macOS 上 `Ctrl+X` 按 ⌃X 或 ⌘X 都触发，`Cmd+X` 只认 ⌘X。Windows 上 `Ctrl` 就是 Ctrl，`Cmd` 是 Windows 徽标键（大多被系统保留），所以两个系统通用的绑定写 `Ctrl+…`。宿主的「退出」（`Cmd+Q`）在 Windows 上不触发，用窗口的关闭按钮或 Alt+F4。
- `+` 无论怎样打出都是 `Plus`：美式 ⇧=、德语等布局的独立 `+` 键都是 `Plus`，带 Option 时是 `Alt+Plus`。
- macOS 系统快捷键先于播放器：⌃F1–⌃F8（键盘导航，如 ⌃F6「将焦点移到浮动窗口」）、⌃↑/⌃↓/⌃←/⌃→（调度中心与切换桌面）、⌘Tab、⌘Space 等默认被系统截走，绑定收不到，默认绑定请避开。
- `Mouse:LeftClick`（视频区左键单击）：按下与松开相距不超过 4 点才算单击。多数情况下**松开即触发**：同样修饰键的 `Mouse:LeftDoubleClick`（任一半边）没有生效的绑定，或者单击的命令可勾选（状态有 `checked`，如「播放/暂停」、开关、单选项）。可勾选的命令被双击执行两次，状态回到原样，同时双击照常触发：默认绑定下双击仍是全屏，播放状态不变，与常见播放器相同。只有单击的命令不可勾选、又有双击绑定时，才等过系统双击间隔（Windows 默认 0.5 秒）：间隔内没有第二次按下才算单击，第二次按下算双击、单击不触发。没有 `Mouse:LeftClick` 绑定时左键行为不变（立即聚焦视频区）。
- 鼠标绑定只在视频区生效，可带修饰键；`:LeftHalf` / `:RightHalf` 按双击位置区分左右半边，带区的绑定优先于不带区的。滚轮按累计位移计步。右键保留给菜单：在视频区按下并在视频区松开时，在松开处弹出（交互对齐 G-15）。窗口模式下在视频区按住左键（不带修饰键）移动超过 4px 时拖动窗口（G-05），这次按下不再算 `Mouse:LeftClick`，也不参与双击；全屏与菜单打开时不拖动。
- 作用范围：默认全局（主窗口或浮动面板窗口获得焦点、且按键没被焦点控件吃掉时，例如输入框里的空格不会触发）；`focus: "panel"` 只在本插件面板（附着或浮动）获得焦点时生效，此时先于全局查找。工具窗口里不触发绑定。
- 窗口标准快捷键不是绑定：工具窗口、浮动面板、选项窗口按系统的标准关闭键关闭（macOS ⌘W；Windows Ctrl+W、Ctrl+F4；Linux Ctrl+W），这些键在窗口收到时就被播放器处理，不进入绑定表，也不交给该窗口里的插件 Slint。主窗口不响应（关闭主窗口等于退出）：插件绑定 `Ctrl+W` 在主窗口照常触发，在这些窗口里不触发（macOS 上 `Ctrl+W` 也认 ⌘W）。默认绑定都不用这些键。
- 菜单或系统对话框打开时绑定暂停。默认只在按下时触发；`repeat: true` 时按住会重复触发。
- 冲突（同一输入、同一范围被两个命令声明）不报错：用户覆盖层优先，其次宿主自己的命令，再次先加载的插件；被压下的绑定写一行宿主日志，赢家停用后自动生效。测试里用 `host.conflicts` 检查。
- 命令 `enabled: false` 时它的绑定让出按键：同一输入依次交给被它压下的命令、再到全局范围；没有可用的命令时，若禁用的命令带 `reason`，宿主把原因显示在 OSD 上并吃掉按键，不带 `reason` 则按键照常交给焦点控件（例如窗口模式下 `Escape` 不再被「仅退出全屏」吞掉）。
- 宿主自己的命令：「全屏」（`fullscreen`，英文 Toggle Fullscreen；`Enter`、`Alt+Enter`、`F11`、`Mouse:LeftDoubleClick`；画中画期间视频区双击改为把画面放回主窗口，§6.6）、「仅退出全屏」（`exit-fullscreen`，英文 Leave Fullscreen；`Escape`，不在菜单，只在全屏时可用）、「退出」（`Cmd+Q`），以及 14 个基础播放动作（§5.5）。
- 用户覆盖层保存在界面状态文件（[架构总览 §6](../architecture/README.md)），由 `plugin.bindings`（§4.2）读写。

### 4.2 绑定管理 `plugin.bindings`

需要 `bindings.manage`。供快捷键设置这类插件列出、改绑、恢复所有命令（含播放器自己的）的绑定。

```ts
interface BindingRef { plugin: string /* manifest id；播放器自己的命令为 "host" */; command: string }
interface BindingInfo extends BindingRef {
  title: string; path: string[] | null;  // 显示文字；不进菜单的命令 path 为 null
  key?: string[] | null;                 // 菜单路径的英文 key，各语言相同（["Playback", "Seek", "Back 5 s"]）；播放器 0.4.3、SDK 1.5.11
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
- `key`（P-2）：按菜单位置找命令时用它，不要用随语言变化的 `path`；内置菜单是 key（`Video`，声明为 `@video` 也一样），Text 段是英文渲染（`Track 2 (opus)`）。
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
  readonly source: string | null;    // 路径或 URL；生成流见下
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

`source` 的取值：本地路径或 URL；没有来源时为 `null`。生成流（`kind === "stream"`）有两种：`openStream`（§5.2）的来源是不透明的 `producer://<n>`，不要解析它；文件解析器（§11）生成的流保留打开时的路径。判断是不是生成流看 `kind`，不要看 `source`。

`PlayerState` 是不可变快照，可以保存下来，用作之后操作的条件（§5.3）。`loop` 由 SDK 总是给出（没有循环为 `null`）；类型上写成可选，只是为了让测试里手写的状态对象不必带它。`can.next` / `can.previous` 来自播放序列提供者（§5.4），没有提供者时为 `false`。

### 5.2 控制

需要 `playback.control`（`openStream` 另需 `producer.av`）。

| 方法 | 说明 |
| --- | --- |
| `open(source, { paused?, signal?, ifCurrent? })` → `Promise<Media>` | 打开路径或 URL，替换当前来源。默认**无条件**。播放器接受加载后 resolve（不等就绪，就绪用 `media.ready()`）。匹配解析器的网址和文件先交给解析器（§11） |
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
| `setAudioDevice(id \| null)` | 在该设备接入期间用它播放，不在时用系统默认，重新接入后切回；`null` = 总是系统默认。`state.audioOutput.requested` 是请求的 id，`active` 是正在用的（`null` = 系统默认），播放器每秒检查一次设备。只在本次运行内记住，不跨启动保存（插件自己保存并在激活时设置）；设置它的插件停用时回到系统默认（与 `setAlwaysOnTop` 相同）。插件生成的音视频流（`openStream`）同样跟随。不存在的 id 也接受（`active` 为 `null`）；格式不对 `invalid` |
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

限制（`frame()` 的结果不合规时 SDK 结束这个流：启动期间 `openStream` 以 `invalid` 失败，播放中 `onStop` 收到 `code: "invalid"` 的错误；SDK 1.3 之前 `code` 是 `"error"`）：

- 画面：正好 `width × height × 4` 字节的 RGBA，每帧最多 4 MiB（4 194 304 字节）。1280×720（3.5 MiB）可以，1920×1080（7.9 MiB）超出；大画面请降低分辨率，播放器会缩放显示。
- 声音：48 kHz 立体声交错的 `Float32Array`（长度为偶数），每个采样都必须是有限值且在 −1–1 之间（`NaN`、`Infinity`、溢出的混音都会结束流）；自己混音时先限幅。长度不限，SDK 按每包至多 4800 帧（100 毫秒）分包发送。
- SDK 在 `frame()` 返回后把画面复制一次进发送缓冲区（SDK 1.3 起；之前两次），所以 `frame()` 可以复用同一个缓冲区，返回后改写它不会影响已经交出的帧。
- `onStop` 只报告插件之外的停止：被别的来源替换、用户关闭、出错；插件自己调用 `stop()` / `dispose()`（以及停用）时不调用，`stop()` 的 promise 结束就是停止完成。
- 生成流不经过播放器的滤镜（§8）：`video.filter` / `audio.filter` 在生成流播放期间以 `unsupported` 失败。

停止（`stop()`、`dispose()`、插件停用）时 SDK 先关闭来源（仍是当前来源时），再断开画面连接，最后释放生成端（SDK 1.1.3 起；之前先断开连接，播放器会把它报成「producer transport closed」加载失败，插件不必再自己先 `close`，原有的这种写法照常可用）。播放器一侧：连接在两帧之间断开后，来源 0.5 秒内被关闭或替换就不算失败；仍选中则报 `producer transport closed`（例如插件进程崩溃）；帧中途断开、协议错误立即报告。

画面和声音经本机连接送给播放器，由 SDK 建立，插件不用管：macOS、Linux 是 `/tmp` 下私有目录里的 Unix 套接字；Windows 是回环地址（`127.0.0.1`）上的 TCP 端口，SDK 连上后先发播放器给的 32 字节令牌，没有令牌的连接被播放器丢弃（本机任何进程都能连回环端口）。协议上 `producer.prepare` 的答复相应是 `socket: "<路径>"` 或 `socket: "tcp://127.0.0.1:<端口>"` 加 `token`（十六进制）。

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
           decoder; pixelFormat; colorspace; displayWidth; displayHeight; displayFps; bitrate;
           hwdec; transfer } | null;
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
| `video.hwdec` | `hwdec-current` | 正在用的硬件解码器，如 `videotoolbox`、软件渲染下的 `d3d11va-copy`；`"no"` 表示软件解码（mpv 解不了的格式、没有可用的硬件解码器） |
| `video.transfer` | `video-params/gamma` | 传输特性原样给出，如 `bt.1886`、`srgb`、`pq`（HDR10）、`hlg`；判断是不是 HDR 由插件做 |

**渲染方式**（`backend`）：

| `backend` | 何时 | 画面 |
| --- | --- | --- |
| `libmpv · GPU 渲染（OpenGL）` | 默认（macOS） | libmpv 用 OpenGL 在自己的线程里渲染，按视频区的物理像素（窗口尺寸 × 屏幕缩放）渲染，缩放由 mpv 完成；硬件解码 `hwdec=auto-safe`（解不了的格式 mpv 自动改用软件解码，`video.hwdec` 如实报告）。界面固定用 Slint 的 OpenGL 渲染器（femtovg） |
| `libmpv · 软件渲染` | 后备：无界面模式（`--headless`）；环境变量 `VESTRA_SOFTWARE_RENDER=1`；Windows、Linux（暂未实现 GPU 渲染）；OpenGL 初始化失败（mpv 的 OpenGL 上下文建不起来，或窗口不能显示 OpenGL 纹理）时自动退回，并在标准错误输出写一行 `render: …` | CPU 渲染。硬件解码用回拷模式 `hwdec=auto-copy-safe`（GPU 解码后把帧拷回内存，如 Windows 上的 `d3d11va-copy`、`nvdec-copy`；都不可用时 mpv 改用软件解码）。按视频区的物理像素渲染，再按来源分辨率封顶（不在 CPU 上渲染比来源更多的像素，余下的放大由界面完成），总像素不超过 3840×2160 |

- 两种方式下 `framesShown` 都是播放器交给视频区的帧数（含生成流的帧），播放、截图、导出、滤镜与显示层（§8、§8.1、§10）行为相同。渲染选项（§8.2）只在 GPU 渲染下有效。
- 运行中退回软件渲染（窗口不能显示纹理）只在第一个窗口出现时发生；如果那时已经在播放，当前来源可能要重新打开。
- 诊断：`VESTRA_HWDEC` 覆盖 `hwdec`（如 `no`、`d3d11va`）；软件渲染下取它的回拷模式（`d3d11va` → `d3d11va-copy`，`no` 不变）；`VESTRA_FRAME_LOG=1` 每秒在标准错误输出一行渲染与显示的帧数和渲染尺寸（两种方式都有）。

- 同时只能有一个提供者（例如播放列表）；另一个插件再登记以 `conflict` 失败（「上一个/下一个已由 vestra.playlist 提供」），**不算激活失败**：插件照常运行，只是不提供上一个/下一个；屏幕提示与插件管理的「提示」同面板位置被占（§6.3）。之后它的标志变化时会再登记一次，原提供者撤销后就能接上。启动时内置插件先登记（同 §6.3）。`dispose` 或插件停用时撤销。
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
| `play-pause` | 播放 ▸ 播放/暂停 | 201 | Space | 暂停/继续，播完后重播；「已暂停」/「播放」；勾选 = 已暂停 |
| `previous` / `next` | 播放 ▸ 上一个 / 下一个 | 202 / 203 | PageUp、Mouse:X1 / PageDown、Mouse:X2 | 交给播放序列提供者（§5.4）；不可用时原因「没有上一个」/「没有下一个」 |
| `frame-back` / `frame-forward` | 播放 ▸ 跳转 ▸ 上一帧 / 下一帧 | 204 / 205，frame | D / F（按住重复） | `player.step("frame", …)`；在途时丢弃重复触发；完成后提示「定位: 下一帧 00:01:23.456」/「定位: 上一帧 …」（完成后的位置，精确到毫秒），没动时「已是最后一帧」/「已是第一帧」。两个方向分别判断：没有视频轨（纯音频、只有封面）禁用，原因「没有视频」；下一帧需要就绪（直播也可以，播完后不可）；上一帧播完后也可用，需要可跳转（生成流、不可跳转的直播原因「当前来源不支持逐帧后退」） |
| `seek-backward` / `seek-forward` | 播放 ▸ 跳转 ▸ 后退/前进 N秒 | 210 / 211，step | ← / →（按住重复） | 从最新未完成的目标累加，夹在 0 与时长之间；「前进 5秒: 00:37 / 01:40」；N 见 `setStepSeek` |
| `rate-normal` | 播放 ▸ 速度 ▸ 正常（单选） | 220 | Z | 1 倍；「播放速度: 1x」 |
| `rate-down` / `rate-up` | 播放 ▸ 速度 ▸ 减速 0.1 / 加速 0.1 | 221 / 222 | X / C | ±0.1，取两位小数，夹在 0.25–4；「播放速度: 1.1x」 |
| `volume-up` / `volume-down` | 声音 ▸ 音量 + / 音量 − | 501 / 502 | ↑、滚轮上 / ↓、滚轮下（按住重复） | ±5，先设音量再取消静音；「音量: 85%」。`volume-up` 上限 100（交互对齐 G-13a）：已经 ≥ 100（别的插件用 `setOutput` 设到 100 以上）时不再增加、照常提示；`volume-down` 不低于 0。`setOutput` 的范围仍是 0–200 |
| `mute` | 声音 ▸ 静音（勾选） | 503 | M | 「静音: 开」/「静音: 关」 |

- 禁用时 `run` 以 `unavailable` 失败（`message` 为原因），未知 id 为 `invalid`。动作执行完才 resolve（之后 `player.state` 已刷新）；同一动作在途时的重复调用被丢弃并直接 resolve（相对跳转与音量除外，它们排队累加）。
- 执行失败（如来源拒绝）显示在视频上。
- 用户覆盖层的键为 `host/<动作 id>`。
- 用户覆盖层的旧键 `vestra.playback/<旧 id>` 在播放器启动时一次性改写为 `host/<新 id>`（§17）。

### 5.6 时间轴标记 `plugin.timeline`

进度条上的标记（章节、书签等）由各插件发布、任何插件读取。每个插件一层，播放器合并。

```ts
type MarkKind = "chapter" | "bookmark" | "marker";
interface TimelineMark { time: number; kind: MarkKind; title: string | null; plugin: string /* 发布者 manifest id */ }
timeline.marks: readonly TimelineMark[];      // 所有层合并，按 time 升序；需要 playback.read
timeline.onChange(listener: (marks: readonly TimelineMark[]) => void): Disposable; // 需要 events
timeline.layer(): TimelineLayer;              // 本插件的层（timeline.marks 能力）；第二次调用抛错
interface TimelineLayer extends Disposable {
  set(marks: { time: number; kind: MarkKind; title?: string | Text }[], opts?: { ifCurrent?: PlayerState }): void;
}
```

- `marks` 同步只读：有 `playback.read` 的插件在激活前读一次；有 `events` 时从第一次读 `marks` 或调用 `onChange` 开始跟随（播放器在合并结果变化时推送，至多每 100 ms 一次）；没有 `events` 时只有启动时的值。同一时间的标记按插件、再按各层内的顺序排列。
- `set`：替换本插件的整层，最后写入生效，`[]` 清空。单层至多 1000 个，`title` 至多 100 个字符，`time` 在 `[0, 时长]`（时长未知时只要求 ≥ 0），`kind` 只能是三者之一；不合法时以 `invalid` 交给 `onError`（`set` 本身不抛错、不返回 promise）。`ifCurrent` 只比较来源（`sourceId`）：写入到达时来源已经换了，丢弃本次写入，不报错；同一来源里的暂停、跳转不影响。
- `title` 是数据（章节、书签自己的名字）；带固定文字的标题用 Text（`text("Chapter {}", [n])`，§6.11），播放器按本插件词典翻译后保存，别的插件读到的是显示文字，100 个字符按显示文字计。
- 来源变化（新 `sourceId`）时播放器清空**所有**层；`dispose` 或插件停用时清除本插件的层。`kind` 播放器不解释，供读取方配色（建议：章节 `Theme.text-dim`、书签 `Theme.accent`）。
- 测试：`host.timeline.layers`（插件 id → 标记）、`host.timeline.marks`（合并结果）；`host.setTimelineLayer(pluginId, marks)` 模拟别的插件发布；打开新来源时清空。

## 6. 窗口与面板

### 6.1 共同接口 `View`

```ts
interface View {
  on(callback: string, handler: (...args) => unknown): Disposable;
  set(props: Props): void;
  bind(props: () => Props): Disposable;
}
type Props = Record<string, PropValue>;
type PropValue = Json | ImageRef;   // Json：null、布尔、数字、字符串、数组、对象
plugin.ui.image(path: string): ImageRef; // 不需要能力
```

**运行时图片**：Slint `image` 属性的值用 `plugin.ui.image(path)` 得到的 `ImageRef`，只能作为属性的**整个值**（不能放进结构体、列表）。工具窗口、面板（含背景层、浮动侧栏）都支持。

- 路径：绝对路径，位于本插件的 `dataDirectory` 或包目录下；规范化后（含符号链接）越界为 `invalid`，不存在为 `missing`。
- 文件：PNG 或 JPEG（按文件内容判断，不看扩展名），至多 16 MB、4096×4096。`ui.image` 同步返回，不检查文件；播放器在**每次使用**（属性写入、窗口打开、面板重建）时检查，失败作为属性写入错误交给 `onError`（§6.1 的重试规则照常），属性保持原值。
- 缓存：播放器按（插件、规范路径、文件大小、修改时间）缓存解码结果，每个插件至多 64 MB，超出时先淘汰最久未用、且没有任何属性正在显示的图片；被淘汰的引用仍然可用，再用时重新解码。播放器不监视文件：内容变了就重新调用 `ui.image`（每次调用都是一个新值，所以同一路径也会重新发送）。
- 释放：`ImageRef` 属于创建它的作用域；作用域释放后再写入它以 `invalid` 失败（SDK 在发送前拒绝），已经显示的图片保持到被覆盖或视图关闭。
- 线上形式（`{ "$vestraImage": 路径, "ref": n }`）是内部细节，不要自己构造：播放器对每次使用都按「该插件 + 路径规则」重新检查，手写的值不会得到额外的权限。
- 测试：视图属性里是线上形式，`host.imagePath(host.window(id).props.cover)` 读出路径；`host.images` 记录写入过的图片。

**Markdown 文档**（播放器 0.2.0，SDK 1.1）：控件库 `@vestra/markdown.slint` 的 `MarkdownView` 显示 Markdown，属性值用 `await plugin.ui.markdown(text, options?)` 得到的 `MarkdownRef`。

```slint
import { MarkdownView, MarkdownBlock } from "@vestra/markdown.slint";
export component Readme inherits Window {
    in property <[MarkdownBlock]> doc;
    callback open-link(string);
    MarkdownView { blocks: root.doc; link-clicked(url) => { root.open-link(url); } }
}
```

```ts
plugin.ui.markdown(text: string, options?: { base?: string; remoteImages?: boolean }): Promise<MarkdownRef>
const doc = await plugin.ui.markdown(readme, { base: readmeUrl });
window.set({ doc });
window.on("openLink", (url: string) => plugin.system.openUrl(url));
```

- 播放器解析（pulldown-cmark）并绘制：标题 1–6、段落（粗体、斜体、删除线、行内代码、链接）、无序/有序/任务列表（嵌套 ≤ 4 层）、代码块（等宽、不高亮）、引用（≤ 4 层）、图片、GFM 表格（单元格为纯文本，≤ 16 列 × 500 行）、分隔线。**内嵌 HTML 不显示**；脚注、数学不启用。`MarkdownRef` 放在 Slint 类型为 `[MarkdownBlock]` 的位置：属性的整个值，或（播放器 0.4.0 之后，SDK 1.5.4）属性值任意深度的一个字段，例如消息列表每行的 `body`；类型不符时整次写入失败。它属于创建它的作用域：释放后写入为 `invalid`，不论在值里的哪一层。`ImageRef` 仍只能作为整个值，嵌在数组或对象里为 `invalid`。
- 组件：`MarkdownView` 是带滚动的整篇文档；`MarkdownBlocks`（播放器 0.4.0 之后，SDK 1.5.4）不滚动、高度随内容，放进自己的布局或 `ListView` 的行。两者都有 `in [MarkdownBlock] blocks`、`link-clicked(string)`，以及可选的 `in bool code-copy`（默认 `false`）与 `copy-code(string)`：打开后代码块右上角有「复制」按钮，点击发出 `copy-code(代码文本)`，由插件写剪贴板（`plugin.clipboard.writeText`）。不打开时外观与以前相同。

```slint
import { ListView } from "std-widgets.slint";
import { MarkdownBlocks, MarkdownBlock } from "@vestra/markdown.slint";
export struct Message { id: string, role: string, body: [MarkdownBlock] }
export component Chat inherits Window {
    in property <[Message]> messages;
    callback copy-code(string);
    ListView {
        for message in root.messages: VerticalLayout {
            padding: 8px;
            MarkdownBlocks { blocks: message.body; code-copy: true; copy-code(code) => { root.copy-code(code); } }
        }
    }
}
```

- 更新：播放器按（插件、视图、属性）记住每个文档的指纹，未变的文档（包括为同一文本重新调用 `ui.markdown` 得到的新引用）不重新解析；含文档的值在形状不变时原地更新：变化的行改写、末尾增删的行原地加减、变化的文档只换变化的块。所以流式回复时每次把整个消息列表重发一遍即可：列表、滚动位置和其它行都保留。形状变化（字段增减、类型变化、原来是空列表）时整体替换，并由 Slint 检查类型。
- 显示细节：中文回退到的系统字体（苹方、Microsoft YaHei 等）没有斜体，Slint 的 FemtoVG 与软件渲染器也不合成斜体，所以斜体另加琥珀色、删除线另加灰色，保证在任何字体下都看得出；行内代码整段用 Slint 的等宽胶囊样式（字号略小、半透明底），代码块整块用 `Theme.font-mono`，其中的中文按系统字体回退显示；任务列表的方框用图形绘制，不依赖字体字形；表格每行列宽相同，表头与正文都按分隔行的对齐方式（`:--`、`:-:`、`--:`）对齐。
- 链接：点击时 `MarkdownView` 发出 `link-clicked(url)`，相对地址已按 `base` 解析（`#锚点` 原样）；打开与否由插件决定。
- `base`：`http(s)` 地址（如 README 自己的地址）或绝对目录；相对链接与图片按它解析。没有 `base` 时相对图片显示替代文字。
- 图片：PNG/JPEG。本地图片按 `ui.image` 的规则（本插件包目录或数据目录内，≤ 16 MB、4096²，与 `ui.image` 共用每插件 64 MB 的解码缓存）。`https:` 图片由 SDK 在本插件进程里下载到 `<dataDirectory>/.vestra-cache/markdown/`：单张 ≤ 4 MB、10 秒、重定向 ≤ 5 次（只跟随到 `https:`）、同时 4 个、每文档 ≤ 64 张；缓存 32 MB，按最近使用淘汰，同一地址 24 小时内不重新下载。下载失败、`http:`、SVG/GIF 等显示替代文字，`markdown()` 本身不因此失败。`remoteImages: false` 不下载。本地图片写成相对 `base`（绝对目录）的路径，或用 `/` 分隔：Markdown 文本里的 `\` 后跟标点是转义，Windows 绝对路径 `C:\…\.cache\a.png` 会丢掉 `.` 前的 `\`，找不到文件而显示替代文字。
- 上限：每个文档文本 ≤ 256 KB（否则 `invalid`）、超过 4000 块截断并在末尾提示、图片 ≤ 64 张；一个属性值里所有文本（各文档的文本也算）合计 ≤ 1 MiB，每个文档算值里的一个节点（共 16384，见上面的属性值上限）。
- 示例插件：`src/examples/markdown/`。测试：`host.markdown(host.window(id).props.doc)` 读回 `{ text, base, images }`（嵌在行里的同样读：`host.markdown(rows[0].body)`）；远程图片由 `host.remoteImages`（地址 → 本地文件）提供，测试不联网。

| 方法 | 说明 |
| --- | --- |
| `on(name, fn)` | 处理 Slint 回调，`fn` 收到回调参数。名字可写 camelCase |
| `bind(fn)` | 保持属性等于 `fn()` 的结果；每轮更新重新求值，**值变化时才发送**，所以用户在界面上改过的 in-out 属性不会被无关的更新覆盖 |
| `set(props)` | 立即写入；即使值与上次相同也发送一次，可用来把被用户改动的控件「拉回」模型值 |

写入失败时：绑定值在下次变化前不重发；`set` 的值退避重试（0.25 秒起翻倍，最长 30 秒）；连续失败只报告一次错误（交给 `onError`，消息以属性名开头，如 `rows: UI array exceeds 4096 rows`）。

**属性值上限**（播放器 0.4.3，SDK 1.5.10；括号里是更早的播放器）。每个属性值（`set`、`bind` 的一次写入，或窗口、面板、设置页的一个初始属性）分别计算：

| 上限 | 值 | 说明 |
| --- | --- | --- |
| 嵌套深度 | 8 | 属性值本身是第 0 层 |
| 值的总数 | 16384（4096） | 数组、对象、字符串、数字、布尔、`null` 各算一个，属性值本身也算；一个 Markdown 文档算一个。每行 8 个字段的列表每行 9 个，约 1800 行（以前约 450 行） |
| 一个数组的行数 | 4096（1024） | |
| 一个字符串或字段名 | 64 KiB | UTF-8 字节 |
| 全部文本 | 1 MiB | 所有字符串和字段名，加上其中 Markdown 文档的文本 |

此外整条请求不能超过播放器的 1 MiB 消息上限。常量 `UI_VALUE_LIMITS`（从 `@vestra-player/sdk/testing` 导入）与播放器一致，播放器的测试检查两边相同。

- 超出时整次写入以 `limit` 失败，属性保持原值，按上面的规则交给 `onError`。初始属性超出时：工具窗口的 `show()` 被拒绝（一向如此）；面板、设置页照常挂上，只是不带这个属性（保持默认值），播放器把失败作为 `ui.error` 交给 `onError`。不让挂载失败，是因为激活期间的挂载失败会让整个插件激活失败。以前这种属性被悄悄丢掉，只写宿主日志。设置窗口未打开时的 `settings.set` 当场检查，超出即失败，与打开时相同。
- 超过 1 MiB 的请求由插件运行器在发送前以 `limit` 拒绝；以前播放器会因此停止插件。
- 播放器之后才发现的失败（重建面板或选项窗口时图片文件已被删除、回调参数无法转换、浮动侧栏准备失败）作为 `ui.error` 推给插件，SDK 交给根作用域的 `onError`，`code` 为播放器给的（`missing`、`ui`、`ui-prepare` 等）。
- 这些上限限制的是一次写入在界面线程上的工作量（转换加比较，每个值约 0.35 µs，满额一次约 6 ms，Apple M 系列），不是建议的列表大小。可能无限增长的列表（播放列表、历史）只发视图附近的行和总行数，随滚动移动窗口（内置播放列表的做法：200 行窗口）。
- `TestHost` 对 `ui.open`、`ui.set`、`ui.mount`、`ui.setMount`、`settings.register`、`settings.set` 执行同样的检查，`code` 与消息和播放器相同。

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

- 用户点标题栏的关闭按钮，或按系统的标准关闭键（macOS ⌘W；Windows Ctrl+W、Ctrl+F4；Linux Ctrl+W；输入框获得焦点时也有效）时，播放器关闭该窗口（等同 `close()`），`opened` 变为 `false`，然后调用 `onClose` 的监听；之后 `show()` 用最新属性重新打开。插件自己调用 `close()` / `dispose()` 不触发 `onClose`。轮询之类的工作在 `onClose` 里停掉。测试里用 `host.window(id).userClose()` 模拟。
- `create` 不会打开窗口。第一次 `show()` 时，用当前绑定的属性作为初始值打开，不会先闪一下默认值。
- **回调必须在第一次 `show()` 之前用 `on` 绑定**，之后再绑定新的回调名会抛错。
- 窗口 id 默认取文件名（`ui/picture.slint` → `picture`）；同一插件用同一文件开多个窗口时，用 `options.id` 区分，id 重复时 `windows.create` 立即抛出 `conflict`（SDK 1.3 之前是不带 `code` 的普通 `Error`）。

### 6.3 面板 `plugin.panels`

需要 `ui.mount`。

```ts
panels.mount(slot: "bottom" | "sidebar" | "backdrop", file: string,
             options?: { component?: string; global?: string; title?: string }): Panel
panels.bottom: PanelSlot          // 底栏位置
panels.sidebar: SidebarSlot       // 侧栏位置
panels.backdrop: {                // 背景层
  readonly state: { effective: boolean } | null;  // null：没有背景层
  onChange(listener: (state: { effective: boolean } | null) => void): Disposable;
}

interface Panel extends View, Disposable {
  readonly ready: Promise<void>;                 // 已挂载
  readonly state: PanelState | null;             // 本面板的状态（SDK 1.5.2）；null：未挂载
  onChange(listener: (state: PanelState | null) => void): Disposable;
  show(): Promise<void>;                         // 切到本面板的页签并显示侧栏
  toggle(): Promise<void>;                       // 本页签正显示 → 隐藏侧栏；否则 show()
  setInteraction(locks: Partial<{ focused; dragging; popupOpen: boolean }>): void;
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>;
  onDrop(listener: (e: { paths: string[]; at: { x; y }; modifiers: DropModifiers }) => unknown): Disposable; // §6.7
}

interface PanelSlot {
  readonly state: SlotState | null;              // null：该位置没有面板
  readonly panels: readonly SlotPanel[] | null;  // 页签，按顺序（SDK 1.5.2）；旧播放器为 null
  onChange(listener: (state: SlotState | null) => void): Disposable; // 只有页签变化时也调用
  setVisible(visible: boolean): Promise<void>;   // 设置用户偏好
  toggle(): Promise<void>;                       // 按播放器当前值翻转偏好
  activate(plugin: string): Promise<void>;       // 切换当前页签，不改显示（SDK 1.5.2）
}
interface SlotPanel { plugin: string; title: string; active: boolean } // plugin：清单 id
interface PanelState {
  active: boolean;      // 本面板是当前页签（底栏、背景层、唯一的侧栏面板总为 true）
  preferred: boolean;   // 位置的偏好
  effective: boolean;   // 位置看得见且本面板是当前页签
  reason: SlotState["reason"] | "otherTab";  // 不是当前页签时为 "otherTab"
  detached: boolean;
}
interface SidebarSlot extends PanelSlot {
  setDetached(detached: boolean): Promise<void>; // 分离为浮动窗口 / 附着回主窗口
}
interface SlotState {
  preferred: boolean;   // 用户的选择（菜单勾选）
  effective: boolean;   // 现在是否看得见（附着、全屏弹出层弹出，或在浮动窗口里）
  reason: "userHidden" | "space" | "fullscreen" | "idle" | null; // "fullscreen"：全屏覆盖层当前收起
  detached: boolean;    // 侧栏为浮动窗口；底栏总为 false
}
```

- 面板文件导出**一个 component 和一个 global**。属性和回调声明在 global 上（桥接对象），component 负责布局。名字由 SDK 从文件推断；文件导出多个时用 `options` 指定。`title` 是侧栏分离后浮动窗口的标题，也是侧栏页签的文字（不给时用插件的 `name`，再没有时为「Vestra Player」）。
- `panels.bottom` / `panels.sidebar` 不属于挂载者：任何插件都能读取和切换（例如底栏按钮开关播放列表）。位置为空时 `setVisible` 等以 `missing` 失败；对底栏 `setDetached` 是 `invalid`。没有 `events` 时 `state` 只在本插件自己改动后刷新。侧栏的显示、宽度、分离状态与浮动窗口位置由播放器跨启动记住。
- 侧栏占视频右侧全高，用户可拖动分隔条改宽度（侧栏至少 317、视频至少 318）；窗口窄到放不下时先压缩视频，再不够才隐藏（`reason: "space"`，偏好不变）。分离后浮动窗口默认 300×408、最小 300×250，显示同一个面板组件，插件的属性和回调不变；点浮动窗口的关闭按钮或按标准关闭键（同 §6.2）等于 `setVisible(false)`。
- **全屏时的附着侧栏**（交互对齐 G-01）：不挤压视频，而是以覆盖层盖在视频右侧（全高，宽为用户拖出的宽度、至少留 318 给视频）。进入全屏时**收起**（`effective: false`，`reason: "fullscreen"`）；偏好为显示时，指针移到屏幕右边缘 4px 以内**弹出**（向左滑出 150ms，`effective: true`，`reason: null`），弹出期间底栏收窄到弹出层左边。指针离开弹出层（回到它左边的视频或底栏上）500ms 后收起；没有锁时 3 秒无活动也收起。全屏中 `setVisible(true)`（F6、底栏按钮）立即弹出，保持到指针进入后再离开或 3 秒无活动；`setVisible(false)` 立即收起。偏好为隐藏时右边缘不弹出。弹出不抢键盘焦点；收起时焦点交回视频区。浮动侧栏不受影响。是否处于覆盖形态由插件按 `window.state.fullscreen && !state.detached` 自己判断；`reason: "fullscreen"` 表示「覆盖层收起，右边缘可以唤出」，不再表示「全屏时不可用」，所以按偏好画开关按钮的插件把它当作「开」。
- `showMenu`：在面板当前所在的窗口弹出单层菜单，列出本插件的命令（`null` 为分隔线），样式、勾选、快捷键文字与主菜单相同；选择一项等于执行该命令。`at` 是窗口坐标，在 Slint 里取 `self.absolute-position.x / 1px + self.mouse-x / 1px`（y 同理）。命令不属于本插件以 `missing` 失败，面板未显示以 `unavailable` 失败，最多 32 项。菜单比窗口高时在窗口内滚动（与主菜单相同）。建议在右键**松开**时调用（与主菜单一致）。
- `setInteraction`：用户正在操作面板（输入框聚焦、拖动、弹出菜单打开）时设为 `true`，防止底栏自动隐藏、全屏侧栏弹出层收起。合并写入，最后写入生效。
- 指针停在底栏上或全屏侧栏弹出层上时，播放器自己把该面板视为锁住（交互对齐 G-04：不因空闲隐藏，指针也不隐藏），在其上移动也算活动；这个锁由播放器维护，插件的 `setInteraction` 写入不会清掉它。底栏内容画到槽位上方视频区的部分（负 `y`）不算在内。
- 面板转发 global 上的所有回调，没有「先绑定再显示」的限制。
- 底栏面板的内容可以画到槽位上方的视频区（负 `y`），宿主不裁切；侧栏与浮动窗口在其上层。进度条悬停时间提示这类内容可以直接画在底栏组件里（宿主的组合测试保证底栏外层不带 `clip`）。
- 每个插件至多挂载一个面板（任一位置）。
- **侧栏页签**（[panel-slots.md](../architecture/panel-slots.md) 第 1 步）：侧栏可以放多个插件的面板，每个是一个页签，最多 8 个。第二个侧栏面板的 `ready` 照常 resolve。只有一个侧栏面板时界面与以前完全一样；两个及以上时宿主在当前面板上方画 31px 页签条（样式同 ui-spec §5.2 的播放列表页签，105px 宽、放不下时等分变窄；当前页签文字下有渐变短线，其余页签文字为 `text-dim`、悬停有底色），右端是作用于整个侧栏的分离/附着和关闭按钮（「关闭侧栏」）。点页签只改宿主属性（`sidebar-active`），不重编主窗口；键盘焦点交给侧栏。
  - 显示、隐藏、分离是**侧栏级**的：`panels.sidebar.setVisible` / `toggle` / `setDetached` 作用于所有页签，`state` 描述整个侧栏（`SlotState` 形状不变）。`panels.sidebar.panels` 列出页签（`{ plugin, title, active }`），`activate(plugin)` 切换当前页签。
  - 本面板：`panel.state.active === false`、`effective: false`、`reason: "otherTab"` 表示登记了但当前不显示；变化通过 `panel.onChange` 通知。`panel.show()` 切到本页签并显示侧栏；`panel.toggle()` 在侧栏显示且本页签是当前页签时隐藏侧栏，否则 `show()`（后台页签被切到前面，而不是关掉侧栏）。旧播放器（没有页签）上它们等于 `setVisible(true)` / `toggle()`。
  - 后台页签：`showMenu` 以 `unavailable` 失败；`setInteraction` 照常记录，但只有当前页签的锁对全屏弹出层起作用；只有当前页签收到 `focus: "panel"` 的按键；拖放交给当前页签，`at` 从页签条下方算起。
  - `tabbed`：面板的桥接 global 声明了 `in property <bool> tabbed` 时，宿主在画页签条期间把它设为 `true`（插件不要自己写），自带页签条的面板（播放列表）据此隐藏自己的页签条。
  - 浮动窗口里是整个侧栏，带同样的页签条，标题随当前页签；全屏弹出层同样带页签条、显示当前页签。
  - 页签顺序：内置插件的在前（播放列表固定第一），其余按第一次出现的顺序；顺序、当前页签、显示和分离状态、浮动窗口位置跨启动保存（`ui-state.json` 的 `sidebarSlot`，旧版本按面板保存的 `panels` 自动迁移，播放列表的状态成为侧栏的状态）。首版不能关闭单个页签、不能拖动排序。
- **底栏和背景层同一时间只有一个面板**。位置已有别的插件的面板（已显示，或正在挂载）时，`ready` 以 `conflict` 失败，消息说明位置和占用者（「底栏已被 vestra.playback 使用」）；侧栏已有 8 个页签时同样以 `conflict` 失败（「侧栏最多 8 个页签」）。这类拒绝**不算激活失败**：插件照常运行，命令、快捷键都在，只是没有面板。播放器同时显示一条屏幕提示、写宿主日志，插件管理里该插件显示「提示：面板未显示：底栏已被 … 使用」（`PluginInfo.error` 与 `PluginInfo.warnings`，状态仍为 `running`）。占用者卸载后位置空出，插件重新加载即可挂上。`ui-prepare` 只表示文件编译或桥接失败。
- 启动时**内置插件先挂载**：内置插件激活完成之前，其它插件（已安装的第三方插件、`--plugin` 开发加载）的挂载请求先等着，所以内置面板总是保有自己的位置（播放列表的侧栏页签排第一），与各插件进程谁先回应无关。

**背景层**（`"backdrop"`）：没有视频帧时画在视频区（铺满，替代黑色的视频图像），用于专辑封面、空闲徽标。

- 同一时间只有一个背景层，第二个插件挂载以 `conflict` 失败（规则同上，不算激活失败）；插件停用时卸载。组件与 global 推断规则同上，`title` 不用。
- 什么时候显示（`state.effective`）由播放器决定：来源阶段 `empty`，或 `ready` / `ended` 且 mpv 没有输出画面（即 `stats().video === null`）时为 `true`；`loading` 时为 `true`（交互对齐 G-16a：加载中视频区不再停在上一个来源的最后一帧，背景层插件可以画加载界面）；`error` 保持上一状态；有画面时为 `false`。画什么由插件按 `player.state.phase` 与 `stats().video` 自己决定（没有 `mode`）。
- 带封面的音频文件：挂着背景层时，播放器让 mpv 不把封面（内嵌图片、同目录的 cover.jpg 等）选作视频轨（`audio-display=no`、`cover-art-auto=no`），所以 `stats().video` 为 `null`、`effective` 为 `true`，封面由背景层插件自己画（例如读文件的内嵌图片）；从挂载后打开的下一个文件起生效。背景层卸载（`dispose` 或插件停用）后恢复 mpv 默认（`embedded-first`、`exact`），mpv 重新显示封面。
- 层次：在视频区的输入层之下、屏幕提示与信息块之下；不接收指针和键盘，双击全屏、右键菜单、滚轮照常由视频区处理。`showMenu` 以 `unavailable` 失败，`setInteraction` 无效果。
- 关闭来源（`empty`）后视频区不再保留最后一帧：没有背景层时是 `Theme.bg-video` 纯色。
- 没有 `events` 时 `state` 只在本插件自己挂载后刷新。测试：`host.backdrop`（`{ owner, effective }`，挂载时 `effective: true`）；`host.setBackdropEffective(bool)` 模拟画面出现或消失并推送。

### 6.4 Slint 文件规则

- 工具窗口文件恰好导出一个组件（通常 `inherits Window`）；不带 `export` 的辅助组件不限。
- 插件 → 界面用 `in` / `in-out property`，界面 → 插件用 `callback`。回调参数和属性值必须能用 JSON 表示（基本类型、数组、结构体），回调返回 `void`。
- 会被插件改值的输入控件用双向绑定（`value <=> root.x`、`checked <=> root.x`）。单向的 `value: root.x` 在用户操作控件后会断开，之后插件改值，控件不会跟着动。属性须为 `in-out`。
- `in` 属性只能单向绑定时，用 `changed x => { slider.value = root.x; }` 在模型变化时回推。
- 主题与控件：`import { Theme } from "@vestra/theme.slint";` 提供颜色（由播放器按当前配色下发，§6.8；插件只读，不能给 `Theme` 赋值，硬编码颜色不会随配色变化）（`bg-bar`、`bg-panel`、`text`、`accent`、`playing`、`danger`（错误与危险操作：失败状态、删除按钮；播放器 0.3.0，SDK 1.3）…）、字号（`font-xs` 9px、`font-body` 11px、`font-tab` 12px、`font-title` 13px、`font-mono` 等宽字体名（由播放器按系统设置：macOS Menlo、Windows Consolas、Linux DejaVu Sans Mono；其中的中文按系统字体回退显示））和控件尺寸（`button-height` 21px、`input-height`、`form-row-gap`、`dialog-padding`…），完整列表见 `src/host/ui/vestra/theme.slint`。`import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint";` 提供紧凑样式的按钮（文字按钮 21px、`small` 18px；文字按钮的首选宽度随文字变宽（至少 66px，`small` 41px，最小宽度不变），译文较长时不溢出，设了固定 `width` 的不受影响（播放器 0.5.0）；`text` 为空时是图标按钮：`icon` 是 Slint `Path` 的路径数据，画在 `icon-width` × `icon-height`（默认 14×12）的 viewbox 里，`stroked: true` 画 1.5px 线而不填充，`checked` 表示开启态；图标按钮的宽高由调用方设置，悬停提示用 `ToolTipArea` 包住：悬停 600ms 显示，指针离开或按下时关闭，下次悬停再显示；只读属性 `showing` 表示提示正在显示）、2px 线滑块（`panel: true` 为面板样式）和悬停提示，以及输入链接组件 `UrlInputRow`（UI-DLG-02：多行输入 + 「打开」「取消」；`in-out property <string> text`、`placeholder`、`confirm-text`（默认「打开」）、`cancel-text`、`enabled`，`callback accepted(string)`（回车或点确定，Shift+回车换行）、`cancelled()`（Esc 或点取消），`focus-input()`、`select-all()` 两个公共函数；不校验地址，校验由插件做；用法见示例 §29）。
- 图标：`import { Icon } from "@vestra/icons.slint";`（播放器 0.4.0，SDK 1.5）：`Icon { name: "play"; color: Theme.text; }` 画播放器自己的 V-Line 图标（圆头 1.5px 线条，不填充），默认大小是图标的绘制框，设 `width`/`height` 可缩放；`Icons.commands(name)`、`Icons.box-width(name)`、`Icons.box-height(name)`、`Icons.line(name)` 给出路径数据，供自己的 `Path` 用。名称与绘制框见[界面规格](../product/ui-spec.md) §2.5。
- 所有插件 Slint 都以 `fluent-dark` 风格编译：std-widgets 的控件在深色配色下是深色；选浅色配色 `vestra-light` 时，播放器在运行时把 std-widgets 的配色方案（`Palette.color-scheme`）切到浅色，切回时复原（插件不要自己改 `Palette.color-scheme`）。每个窗口都用系统默认字体，字体里没有的字（例如西文字体里的中文）由 Slint 按字符回退到系统里有这个字的字体，插件不需要自己设字体；插件写的 `default-font-family` / `font-family` 照常生效，缺字同样回退。播放器不内置界面字体。
- 编辑器里要让 `@vestra/...` 导入可解析，给 Slint 语言服务设 `libraryPaths`：`{ "vestra": "<仓库>/src/host/ui/vestra" }`。
- 文字（播放器 0.4.3）：用户看得到的字面量都写成 `@tr("English")`。可带参数（`@tr("Back {} s ({})", root.step, root.key)`，`{}` 依次、`{0}` 按序号）、复数（`@tr("{n} file" | "{n} files" % root.count)`，`{n}` 是数，只分 1 与其它）和上下文（`@tr("Menu" => "Open")`）。播放器加载文件时把字面量换成插件词典（§6.10）里的译文，Slint 再代入参数；没有译文时显示英文。不要在 Slint 里拼接文字（`"[" + a + "]"` 写成 `@tr("[{}]", a)`），也不要从 TS 传整句文字进来（传数据，句子在 Slint 里）。`@vestra/` 控件库自带的文字（`UrlInputRow` 的默认按钮、代码块的复制按钮）由播放器翻译。

### 6.5 屏幕提示 `plugin.osd`

需要 `ui.osd`。

```ts
osd.show(text: TextValue, args?: TemplateArgs): Promise<void> // 视频左上角一行，2 秒后淡出；新提示替换旧提示
osd.panel(): OsdPanel                   // 本插件的只读信息块，每个插件一个
interface OsdPanel extends Disposable {
  set(text: TextValue | null, args?: TemplateArgs | null, options?: { maxColumns?: number }): void; // 多行文字；null 隐藏；最后写入生效
}
type TemplateArgs = readonly TextArg[] | Readonly<Record<string, TextArg>>;  // TextArg = string | number | Text | TextList（§6.11）
```

- 模板（播放器 0.4.3）：`text` 是英文模板，播放器先查插件词典（§6.10）得到译文，再用 `args` 填占位符（`{}` 依次，`{0}` 按序号，`{name}` 取对象字段；`{{`、`}}` 是花括号；整数不带小数；没有对应参数的占位符原样保留）。不给 `args` 时不格式化，文字原样显示（旧插件不变）。`args` 至多 16 个字符串或有限数字，否则 `invalid`。不要在 TS 里拼句子（`` `Speed ${rate}×` ``）：译者只能翻译整句模板；参数本身是固定集合里的词时，每个取值写一整句模板。0.4.3 之前的播放器忽略 `args`，显示模板原文。
- 文字值（SDK 1.5.11，§6.11）：参数可以是 Text / TextList，同样翻译后代入（「下载失败：{}」+ `text("Connection timed out")`）；`text` 本身也可以是 Text 或 TextList（这时不给 `args`，参数在它里面）。
- `maxColumns`（8–1000，SDK 1.5.11）：播放器在翻译、格式化**之后**把每行截到这么多列，截断处加 `…`；中日韩字符与全角字符算 2 列，组合用符号 0 列（`unicode-width`）。插件按视频区宽度算列数，不用量译文。

- 提示：显示出来的文字 1–200 个字符，换行替换为空格；白字放在半透明深色圆角底板上，底边一条渐变线，左/上边距 10px（播放器 0.4.0 之前是粗体白字黑描边）。字号随视频区高度变化（交互对齐 G-10）：视频区高度的 3%，取整后夹在 12–48px（默认窗口约 18px，1080 高的全屏 32px），底板内边距与圆角随字号缩放。插件停用时已显示的提示照常到时消失。
- 信息块：位于提示底板下方 4px，与提示同样的半透明深色圆角底板，字号随提示同比例缩放（提示 16px 时 11px，夹在 9–28px），不接收输入，最多 32 行、4096 字节；多个插件各一块时从上到下排列。`dispose` 或插件停用时移除。第二次调用 `panel()` 抛错。
- 无界面模式下以 `unavailable` 失败（`panel().set` 的失败交给 `onError`）。

### 6.6 主窗口 `plugin.window`

```ts
interface MainWindow {
  readonly state: WindowState;   // 同步只读，启动时已读好
  onChange(listener: (state: WindowState) => void): Disposable;
  setVideoArea(size: { width: number; height: number }): Promise<{ width; height; clamped: boolean }>;
  setAlwaysOnTop(onTop: boolean): Promise<void>;
  setCursorAutoHide(mode: "fullscreen" | "playing"): Promise<void>;
  setPictureInPicture(on: boolean): Promise<void>;
}
interface WindowState {
  readonly fullscreen: boolean;
  readonly videoArea: { readonly width: number; readonly height: number }; // 逻辑点
  readonly pictureInPicture: { readonly supported: boolean; readonly active: boolean };
}
```

- `state`、`onChange` 不需要能力；四个 `set…` 需要 `window.control`。`videoArea` 是视频区（不含底栏、侧栏、分隔条），全屏时是整个屏幕；无界面模式为 0×0。有 `events` 时随窗口变化更新（从插件第一次读 `state` 或调用 `onChange` 开始跟随），没有 `events` 时只在启动和本插件的 `setVideoArea` 之后更新。
- `setVideoArea`：把窗口调成视频区为 `width`×`height`（1–16384 逻辑点；1 倍 = 一个视频像素一个点）。播放器加上底栏和附着的侧栏（侧栏宽 + 3），保持窗口左上角，再把窗口挪回屏幕可见区（macOS 不含菜单栏和 Dock；Windows 按整个显示器计算，可能与任务栏重叠）。放不下时按比例缩小；小于最小尺寸（视频区宽 318、高 120，窗口 320×182）时放大到最小值（会有黑边）；这两种情况 `clamped: true`。resolve 时布局已是新尺寸（最多等 1 秒），返回实际视频区，`state` 已刷新。全屏时与无界面模式 `unavailable`（`message`「全屏时不可用」）。
- `setAlwaysOnTop`：`true` 时播放器的普通窗口（主窗口、浮动侧栏、工具窗口，包括之后新开的）浮在其他应用的窗口之上（macOS `NSFloatingWindowLevel`，Windows 为「总在最前」）；系统对话框不受影响。多个插件时最后一次调用生效，该插件停用后回到其余插件中最后的值（都没有则不置顶）。无界面模式 `unavailable`。
- `setCursorAutoHide`：`"fullscreen"`（默认）只在全屏时隐藏指针；`"playing"` 在窗口模式也隐藏。隐藏条件不变：播放中（就绪且未暂停）、指针在视频区、3 秒内没有指针移动或按键、没有菜单或系统对话框、面板没有在操作；窗口模式另需主窗口是活动窗口。多个插件时最后一次调用生效，该插件停用后回到其余插件中最后的值（都没有则 `"fullscreen"`）。
- `setPictureInPicture`：`true` 把视频放进系统画中画小窗播放，`false` 放回主窗口。小窗打开（或关闭）之后才 resolve，这时 `state.pictureInPicture.active` 已更新。小窗里是播放器渲染的画面（含画面效果和 mpv 画的字幕），播放器自己画的屏幕提示、信息块、底栏不在小窗里；画中画期间主窗口视频区显示灰字「正在以画中画播放」，在视频区双击等于小窗的「返回」（关闭小窗、不暂停、不切换全屏，交互对齐 IA-CORE-12）。GPU 渲染（§5.4）时小窗直接显示 GPU 渲染出的帧（不拷贝），软件渲染时显示软件渲染的帧，分辨率都是主窗口视频区的渲染尺寸（小窗拉得比视频区大时会变糊）。规则：
  - 全屏时调用 `true`：播放器先退出全屏，再打开小窗。已经打开时再调 `true` 直接 resolve（之后算这个插件打开的）；没打开时调 `false` 直接 resolve。
  - 小窗的播放 / 暂停按钮和跳转按钮直接控制播放（跳转距离由系统给出）；小窗显示进度条，可以跳转的来源才有，直播或不能跳转的来源只有播放 / 暂停。小窗的「返回」按钮关闭小窗、不暂停；小窗的关闭按钮关闭小窗并暂停（和 Safari、QuickTime 一致）。这两种关闭插件都从 `onChange` 看到 `active` 变为 `false`。
  - 小窗跟着来源走：换来源时保持；新来源没有视频，或来源被关闭时自动关闭。打开它的插件停用时也关闭。主窗口重建（挂载、卸载界面）不影响小窗。
  - 错误：系统不支持（`supported` 为 `false`）`unsupported`；没有视频或无界面模式 `unavailable`；上一次调用还没完成 `busy`；系统拒绝或 5 秒内没打开 `failed`（`message` 是原因）。
  - `supported`：macOS 14 及以后为 `true`；Windows、Linux 和无界面模式为 `false`（以后会由播放器自己做一个无边框置顶小窗，接口不变）。不支持时插件可以用 `setVideoArea` + `setAlwaysOnTop` 做「迷你尺寸」作为退路。
- 测试：`host.mainWindow`（`{ fullscreen, videoArea, onTop, cursorAutoHide, pictureInPicture }`）；`host.setMainWindow({ fullscreen?, videoArea?, pictureInPicture? })` 模拟用户改变窗口（包括在小窗里关闭画中画）并推送。`TestHost` 的 `setPictureInPicture` 立即切换 `active`（全屏时同时退出全屏）；把 `host.mainWindow.pictureInPicture.supported` 设为 `false` 模拟 Windows、Linux。

### 6.7 拖放 `plugin.drops`、`Panel.onDrop`

需要 `ui.drop`。

```ts
interface DropModifiers { alt: boolean; shift: boolean; cmd: boolean } // cmd：macOS ⌘，Windows Ctrl
drops.provide(handler: {
  extensions: string[];                                          // 小写、不带点；"folder" 匹配文件夹
  drop(e: { paths: string[]; modifiers: DropModifiers }): unknown;
}): Disposable;
Panel.onDrop(listener: (e: { paths: string[]; at: { x: number; y: number }; modifiers: DropModifiers }) => unknown): Disposable;
```

- **视频区**上松手：播放器按扩展名（不区分大小写）把路径分组，保留文件管理器（Finder、资源管理器）给出的顺序，每个提供者一次调用。文件夹按保留名 `"folder"` 匹配（不展开，展开由提供者做）。同一扩展名有多个提供者时**先登记的生效**，后登记的写一行播放器日志；没有提供者的文件，播放器什么也不做（不提示）。
- **面板**（附着的底栏、侧栏，以及浮动侧栏窗口）上松手：该面板有 `onDrop` 监听时，整组路径交给它（不按扩展名分）；没有监听时按视频区规则分发。`at` 是**面板坐标**：相对面板组件左上角的逻辑点（面板根组件自己的坐标系；窗口坐标要加上根组件的 `absolute-position`）。滚动列表请用自己回传的滚动位置换算到行。
- 工具窗口、选项窗口上松手：忽略。全屏时底栏浮在视频上，松手一律按视频区处理。
- 不提供拖动过程中的位置（没有 `onDragOver`），拖放期间播放不暂停、不变化。
- `extensions` 1–64 个，每个 1–16 个小写字母或数字；不合法时 `provide` 立即抛出 `invalid`。`dispose` 或插件停用时撤销。
- 测试：`host.drop(paths, { target: "video" | "panel", at?, modifiers? })` 按同一规则分发（`"folder"` 认存在的目录）；`host.dropProviders` 列出登记，`host.panelDrops` 为面板是否在监听。

### 6.8 外观 `plugin.appearance`

```ts
appearance.theme: ThemeId;                                      // 同步只读，不需要能力
appearance.onChange(listener: (theme: ThemeId) => void): Disposable; // 需要 events
appearance.set(theme: ThemeId): Promise<void>;                  // 需要 ui.theme；SDK 对其它值抛 invalid
type ThemeId = "vestra" | "vestra-light";
```

- 配色固定由播放器提供：`vestra`（深色，默认）和 `vestra-light`（浅色，播放器 0.4.0、SDK 1.5）。两套都取自标志的色谱，颜色值见[界面规格](../product/ui-spec.md) §2.1。以后还可能增加配色，插件不要假定只有这两种，用 `Theme` token 而不是比较配色 id。圆角、字号、控件尺寸不随配色变化，没有单独的圆角选项。
- 视频区在两套配色里都是深色（`Theme.bg-video`）：画在视频上的文字和图标用浅色，不要用 `Theme.text`（浅色配色里是深色字）。
- 播放器不认识的配色 id（例如早期版本有过、现已去掉的配色；0.4.0 之前的播放器收到 `vestra-light`）一律按 `vestra` 处理：界面状态文件里保存的旧值读成 `vestra`；用旧 SDK 构建的插件发来的旧 id 也按 `vestra` 执行，不报错。
- 播放器把配色下发到每个视图（主窗口与其中的底栏、侧栏、背景层，浮动侧栏，工具窗口，选项窗口），新开的窗口一开始就是当前配色；`Theme` 的颜色随之变化，插件 Slint 不用改。切换不打断正在拖动的滑块与打开的菜单。单个视图下发失败只写日志，下一个刷新周期重试。
- 选择由播放器保存（界面状态文件的 `appearance.theme`），调用 `set` 的插件停用后配色保持。`theme` 在激活前随窗口状态读好；有 `events` 时从第一次读 `theme` 或调用 `onChange` 开始跟随别的插件的切换。
- 屏幕提示「配色：…」由调用方插件自己显示。
- 界面语言（播放器 0.4.3、SDK 1.5.9）：

  ```ts
  appearance.language: LanguagePreference;                  // 保存的选择，同步只读，不需要能力
  appearance.setLanguage(language: LanguagePreference): Promise<void>; // 需要 ui.theme；其它值 invalid
  type LanguagePreference = "system" | Locale;
  ```

  `setLanguage` 只保存选择（界面状态文件的 `appearance.language`；`"system"` 即跟随系统），**下次启动生效**，本次运行的语言（`plugin.i18n.locale`，§6.10）不变；提示用户「重启后生效」由调用方插件做。保存后播放器推送 `appearance.changed`，有 `events` 的插件的 `appearance.language` 随之更新。0.4.3 之前的播放器读到 `"system"`，`setLanguage` 以 `unsupported` 失败。
- 测试：`host.appearance`（当前值，可在 `startPlugin` 前设置）、`host.setAppearance(theme)` 模拟别的插件切换；`host.language`、`host.setLanguage(language)`。

### 6.9 设置页 `plugin.settings`

需要 `ui.settings`。

```ts
type SettingsCategory = "general" | "playback" | "subtitles" | "video" | "audio" | "input" | "appearance" | "plugins" | "other";
settings.page(file: string, options: { id: string; category: SettingsCategory; title: string; order?: number }):
  View & Disposable & { readonly ready: Promise<void> };
settings.show(pageId?: string): Promise<void>;
```

- 页面文件与面板相同：导出一个 component 和一个 global，名字由 SDK 推断（没有 `component` / `global` 选项）；属性与回调在 global 上，回调没有「先绑定再显示」的限制。页面宽 440 逻辑点，高度由组件决定，超高时选项窗口垂直滚动。
- `id`：本插件内唯一，1–64 个字母、数字、`.`、`-`、`_`；重复或不合法 `invalid`。`title` 1–24 个字符；每个插件至多 8 页（超出 `invalid`）；`order` 整数（±1 000 000），同一分类内小的在前，再按插件 id、页面 id。不合法的 `options` 在 `page()` 时立即抛出；文件编译失败时 `ready` 以 `ui-prepare` 失败（激活期间 = 激活失败）。
- 选项窗口（标题 Options，640×480、最小 600×420，可调整大小）由播放器创建：左侧分类树（160 宽，九个分类按上面的固定顺序，标题为 General / Playback / Subtitles / Video / Audio / Keys and Mouse / Appearance / Plugins / Other，按界面语言显示，简体中文是 常规 / 播放 / 字幕 / 视频 / 声音 / 快捷键与鼠标 / 外观 / 插件 / 其它；空分类不显示），右侧当前页面，底部 Close（关闭）。页面 `title` 写英文原文，按插件词典翻译后显示（§6.10）。窗口内不触发快捷键，标准关闭键（同 §6.2）等于「关闭」；打开不影响播放。
- `show()`：打开窗口或把它带到前面。`pageId` 只能是本插件的页面（别的值 `invalid`）；省略时显示当前页，窗口第一次打开时显示上次的页面（播放器记在界面状态文件 `settings.lastPage`），不存在则第一页。没有界面时 `unavailable`。
- 页面所属插件停用（或 `dispose`）时页面移除：正在显示的话切到同一分类的第一页，分类空了切到「常规」第一页（再没有则第一页），全空显示 Nothing to set here（没有可设置的项目）；插件重新启用后随它的注册恢复。页面集合变化时窗口重建，各页面的属性由播放器按最后写入的值恢复（`bind` 照常）。
- 页面属性可以是运行时图片（§6.1）。
- 测试：`host.settings.pages`（id → `{ category, title, order, file, component, global, props, emit(callback, ...args) }`；`title` 是按 `host.locale` 显示的文字）、`host.settingsPage(id)`；`host.settings.shows`（每次 `show` 的页面 id，省略为 `null`）、`host.settings.current`。

### 6.10 界面语言 `plugin.i18n`

播放器 0.4.3、SDK 1.5.9。不需要能力。

```ts
plugin.i18n.locale: Locale;   // 本次运行的界面语言，同步只读
type Locale = "en" | "zh-Hans" | "ja" | "ko" | "de" | "fr" | "es";
```

- 界面语言在播放器启动时决定，整个运行期间不变：保存的选择（§6.8 `appearance.language`）或系统的首选语言里第一个支持的，都没有用英文。繁体中文（`zh-Hant`，台湾、香港、澳门）用英文。0.4.3 之前的播放器报告 `"en"`。
- **插件不写翻译代码**：没有 `t()`、没有语言变化事件（`plugin.i18n.text` 返回的是数据，不是译文，§6.11）。用户看得到的文字都写英文原文，放在三处，由播放器用插件自己的词典翻译：声明（命令 `title`、`path`、`reason`，设置页与面板标题，manifest 的 `name`、`punchline`、`description`、`tags`）；Slint 的 `@tr`（§6.4）；交给播放器显示的句子（`osd.show`、`OsdPanel.set`、`dialogs.confirm` 的模板与 `args`，文件对话框的 `title`）。`locale` 只用于不是文字的东西（日期、数字的写法）。
- 词典：插件目录的 `locales/messages.pot`（`vestra-plugin i18n extract` 生成）与每种语言一个 `locales/<lang>.po`（gettext 格式；英文是原文，没有 `en.po`）。`vestra-plugin build` 把 `.po` 编译成 `dist/locales/<lang>.json` 放进包里（`.po` 不进包）；开发加载（`--plugin`）直接读 `.po`。格式与加载规则见[多语言设计](../architecture/i18n.md) §3。
- 不翻译的文字（各语言的本名、产品名）前加 `i18n-ignore` 注释：`extract` 不收录，`check` 不警告。给译者的说明写在文字前以 `TRANSLATORS:` 开头的注释里。
- 测试：`startPlugin(activate, { directory, locale: "zh-Hans" })` 或 `host.locale = "zh-Hans"`（在 `startPlugin` 之前）加载该语言的词典；`host.command(id).display`（显示出来的菜单）、`host.osd`、`host.osdPanel`、`host.confirmTexts`、`host.translate(text, args)` 是按该语言显示的文字，声明原文在 `.declaration` 与 `host.sent(…)`。没设 `locale` 时插件文字按英文，但播放器自己的文字（核心动作的提示、拒绝消息）保持 SDK 1.5.8 的中文，旧测试不受影响。

### 6.11 文字值 `plugin.i18n.text`、`list`、`error`、`resolve`

播放器 0.4.3、SDK 1.5.11。不需要能力。用于固定文字与数据混排、TS 又不能拼句子的地方（第 2 轮请求 R2-audio-1 等，设计见 [i18n 轮记录](../process/rounds/2026-10-i18n.md)「第 2 轮答复」）。

```ts
plugin.i18n.text(source: string, args?: TemplateArgs): Text      // { $vestraText: source, args }：纯数据，不是译文
plugin.i18n.list(items: readonly TextArg[], style?: ListStyle): TextList  // { $vestraList: items, style }
plugin.i18n.error(source: string, args?: TemplateArgs): TextError // Error，text 字段是 Text，message 是英文渲染
plugin.i18n.resolve(value: TextValue): string                    // 只给离开播放器的文字（剪贴板、文件）
type ListStyle = "list" | "clauses" | "semicolons" | "sentences" | "lines";
type TextArg = string | number | Text | TextList;
type TextValue = string | Text | TextList;
```

- **播放器翻译**：显示时用本插件词典查 `source`，再用渲染过的参数格式化（规则同 §6.5；Text 总是格式化）。参数里的字符串、数字是数据；参数里的 Text / TextList 同样翻译，可以嵌套。TextList 用界面语言的分隔符连接各项：

  | style | en（ko、de、es 同） | zh-Hans | ja | fr |
  | --- | --- | --- | --- | --- |
  | `list`（列举，默认） | `, ` | `、` | `、` | `, ` |
  | `clauses`（同一句的分句） | `, ` | `，` | `、` | `, ` |
  | `semicolons` | `; ` | `；` | `；` | ` ; `（前面是窄不换行空格） |
  | `sentences`（最后一句不加句号） | `. ` | `。` | `。` | `. ` |
  | `lines` | 换行 | 换行 | 换行 | 换行 |

- **接受的位置**（任何「播放器显示插件文字」的地方）：命令 `title`、`path` 各段（不能是 TextList；菜单按**英文渲染**合并，`Track 2 (opus)`）、`reason`；`osd.show`、`OsdPanel.set` 的文字与参数；确认框各字段；文件对话框 `title`、`saveFile` 的 `fileName`；窗口、面板、设置页属性值里 Slint 类型是 `string` 的位置（任意深度：`[string]` 模型、结构体字段）；`TimelineLayer.set` 的标题；`subtitles.load({ title })`；解析器 `resolve` 抛出的 `TextError`（`player.state.error` 与打开失败的消息显示译文；流在启动时失败同样）。
- **属性值**：播放器在任何检查之前把属性值里的 Text / TextList 换成显示的字符串（`ui.open`、`ui.set`、`ui.mount`、`ui.setMount`、`settings.register`、`settings.set`），所以 §6.1 的上限按换好的字符串计；Slint 结构体字段不能以 `$` 开头，`$vestraText` 不会与真实数据冲突。
- **上限**（`TEXT_LIMITS`）：嵌套 8 层，一个值里文字值与参数共 1024 个，每个 Text 至多 16 个参数，每个 TextList 至多 256 项，`source` 与字符串参数各至多 4096 字节（模板参数原来是 1024 字节）。不合法时在 `text()`/`list()` 处同步抛 `invalid`，播放器也以 `invalid` 拒绝。
- **`resolve`**：同步返回本次语言的字符串，用插件进程自己读的同一份词典（`locales/<lang>.json`，开发时 `.po`）。**只用于离开播放器的文字**（info 的「复制信息」、hotkeys 导出的 CSV）；界面上一律直接传文字值，否则 `check` 看不出句子是否完整。
- **提取**：`vestra-plugin i18n extract` 收集每个 `text(…)`、`error(…)`、`resolve("…")` 调用的 `source`（字面量、条件两支、`as const` 表；`plugin.i18n.text` 或解构出来的 `text` 都认）和 `{ $vestraText: "…" }` 字面量；只有占位符和标点的 `source`（`"{} ({})"`、`"{0}: {1}"`）也照常收录，各语言的标点不同（中文 `"{}（{}）"`）。字段类型是 Text 时不再报「不是字面量」。
- **旧播放器**：插件代码运行在播放器自带的 SDK 上，0.4.3 之前的播放器没有 `plugin.i18n.text`；用文字值的插件写 `"minPlayerVersion": "0.4.3"`。
- 测试：TestHost 与播放器同样显示：`FakeCommand.display`（及不可枚举的 `display.reason`）、`host.osd`、`host.osdPanel`、`host.confirmTexts`、`host.fileDialogs`、窗口与面板的 `props`（文字值已换成字符串；原样在 `host.sent(…)`）、时间轴与字幕标题、解析器的 `TextError`、`BindingInfo.key`。

## 7. 对话框 `plugin.dialogs`

需要 `ui.dialog`。系统原生对话框；没有时长限制，用户选择或取消才结束。同一时间只有一个对话框（文件对话框与确认框共用），第二个以 `busy` 失败；无界面模式 `unavailable`。

```ts
dialogs.confirm(options: {
  message: TextValue;     // 1–200 个字符，粗体
  detail?: TextValue;     // ≤ 2000 个字符，正文
  confirm: TextValue;     // 确认按钮（默认按钮，回车），1–40 个字符
  cancel?: TextValue;     // 取消按钮（Esc），1–40 个字符，默认「取消」
  destructive?: boolean;  // 确认按钮标为破坏性操作（红字）
  checkbox?: TextValue;   // 文字下方的复选框，1–40 个字符
  args?: TemplateArgs;    // 字符串 message、detail 的模板参数（播放器 0.4.3，§6.5）；Text 字段自带参数
}): Promise<{ confirmed: boolean; checked: boolean }>
```

- 文字（播放器 0.4.3）：全部字段写英文原文，播放器按插件词典翻译（§6.10）；`message`、`detail` 是模板，用 `args` 填占位符，按钮与复选框只翻译。长度限制按显示出来的文字计。默认的取消按钮是播放器的 Cancel（简体中文「取消」）。文件对话框的 `title` 同样翻译。任何文字字段都可以是 Text / TextList（SDK 1.5.11，§6.11），例如权限列表 `text("It will be able to:\n{}", [list(perms, "lines")])`。`saveFile` 的 `fileName`：字符串是数据，不翻译；Text 按插件词典翻译（`text("Playlist.m3u8")`），译文里的 `/`、`\` 换成 `-`。

- macOS：系统提示框（`NSAlert`），以当前活动的播放器窗口（发起确认的工具窗口、浮动侧栏，通常就是用户刚点按钮的窗口）为父窗口的表单（sheet），没有活动窗口时挂主窗口；都不可用时为应用模态窗口。`confirmed`：用户点了确认按钮；`checked`：复选框的状态（没有复选框为 `false`，取消时也如实返回）。长度按 Unicode 字符计（一个汉字一个字符），超出 `invalid`。
- Windows：系统任务对话框（`TaskDialogIndirect`，播放器的可执行文件内嵌 Common Controls v6 的应用清单），挂在当前活动的播放器窗口（没有时挂主窗口）。正文是 `message`，有 `detail` 时空一行接在后面，标题栏是「Vestra Player」；按钮文字是 `confirm`、`cancel`（两者相同时取消按钮末尾多一个看不见的空格，以区分答复）；`destructive` 显示警告图标；按 Esc 或关闭对话框等于取消。有 `checkbox` 时，用户确认后再弹一个「是 / 否」对话框，正文为复选框文字，选「是」即 `checked: true`；取消时不再询问，`checked` 为 `false`。
- 其它系统 `unsupported`。
- 「不再询问」之类的记忆由插件自己保存（`storage.json`），播放器不记。
- 测试：`host.dialogAnswers.push({ confirmed, checked })`；`host.confirms` 记录每次的选项（原文），`host.confirmTexts` 是显示出来的文字。

文件对话框：

| 方法 | 返回 |
| --- | --- |
| `openFile({ title? })` | 路径；取消为 `null` |
| `openFiles({ title? })` | 路径数组；取消为 `[]` |
| `openDirectory({ title? })` | 路径；取消为 `null` |
| `saveFile({ title?, fileName? })` | 路径；取消为 `null` |

## 8. 视频与音频滤镜

需要 `processing.video` / `processing.audio`，以及 `playback.read` 和 `events`。没有 `events` 时 SDK 收不到来源变化：媒体就绪前设的值一直停在 `waiting`，换片源后失败状态也不会自动清除。

```ts
plugin.video.filter(id: string): VideoFilter  // Filter<VideoNode[]>
plugin.audio.filter(id: string): AudioFilter  // Filter<{ nodes: AudioNode[]; delayMs?: number }>

interface Filter<T> extends Disposable {
  set(value: T): void;          // 期望的处理链；最后写入生效；空链 = 撤销本插件的这条链
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
- **空链**（视频 `set([])`，音频 `nodes: []` 且 `delayMs` 为 0）删除本插件的这条链，其它插件不受影响；从未应用过时什么都不发送。播放器 0.3.0 之前视频空链以 `invalid`（「resource must contain a node」）失败，效果留在画面上；音频一直可以。
- **生成流**（`openStream`，§5.2）的画面和声音不经过播放器的解码管线，处理链对它不起作用：生成流播放期间 `set` 非空链以 `{ state: "failed", error.code: "unsupported" }` 结束（不轮询；播放器 0.3.0 之前一直停在 `waiting` 并每 250 毫秒重试），空链照常删除。值保留，来源换成文件后 SDK 自动重新应用。
- **硬件解码**：滤镜是 FFmpeg 的软件滤镜，需要内存里的帧。GPU 渲染默认的零拷贝硬件解码（`hwdec=auto-safe`，macOS 上是 VideoToolbox）把帧留在显存里，mpv 配置不了滤镜图，只写一行日志就把它丢掉（播放器 0.3.0 之前的缺陷：状态是 `applied`，画面却没变）。播放器 0.3.0 起，只要有任何插件的视频链，就改用同一解码器的回拷模式（`auto-copy-safe`，仍是硬件解码，`stats().video.hwdec` 报 `videotoolbox-copy`），链全部删除后恢复；切换时解码器重新初始化，画面可能停顿一下。万一这次切换让 mpv 改用了软件解码（`stats().video.hwdec` 变成 `"no"`），播放器会再请求一次硬件解码；再失败，这个文件就用软件解码播完，下一个文件照常先试硬件解码。软件渲染本来就用回拷模式，有没有链都不切换。

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
| `{ kind: "frameBlend", fps }` | 24–120 的整数（输出帧率）；全部插件合计最多 1 个 | `framerate=fps=N`（相邻帧混合插帧） | — |
| `{ kind: "volume", gainDb }` | −60–0 | `volume` | gainDb |
| `{ kind: "equalizer", frequency, q: 1, gainDb }` | frequency 20–20000，gainDb −6–6 | `equalizer` | gainDb |
| `{ kind: "treble", frequency: 3000, slope: 0.5, gainDb }` | gainDb 0–6 | `treble` | gainDb |
| `{ kind: "dynaudnorm", enabled, maxGain, peak }` | maxGain 1–4，peak 0.5–0.95 | `dynaudnorm` | 全部 |
| `{ kind: "channels", mode }` | `"stereo"`（不改）\| `"left"` \| `"right"` \| `"mono"` \| `"swap"`；多于两声道时先混成立体声 | `aformat` + `pan` | — |
| 音频 `delayMs` | −10000–10000，10 的倍数；全部插件的正、负延迟各自合计不超过 10000 | — | — |

- 「—」的字段改值时宿主重建本阶段的处理链（画面可能停顿一帧，音频处理状态复位）；拖动滑块的控件用表中可原地修改的字段。
- 画面节点作用于宿主的方向处理（容器旋转标记）之后：竖拍视频先被摆正，`transform` 再在摆正后的画面上旋转。
- 所有插件合计最多 64 个节点。video 插件同时打开全部效果时的最长链：eq、hue、sharpen、denoise、deblock、deband、deinterlace、transform、reframe、gblur 各一个，共 10 个节点（transform 最多展开为 3 个 FFmpeg 滤镜）。
- `frameBlend` 不论写在哪个插件、链里哪个位置，播放器都把它放在整条视频图的**最后**（所有插件的链之后），所以降噪、反交错这类时间域滤镜仍在原帧率上工作。帧率只能改一次：第二个 `frameBlend`（同一插件的另一个滤镜，或别的插件）以 `invalid` 失败。打开后 `stats().video.fps`（`estimated-vf-fps`）读到的是输出帧率（如 48）。CPU 成本高（1080p 24 → 60 fps 约多一个核），插件应按 `stats()` 的原帧率决定是否加，并在丢帧时提示。
- 「含效果」截图与导出（§10）使用同一条链，与播放画面一致；含 `frameBlend` 时导出视频的帧率随之改变。

### 8.1 显示层 `plugin.video.view()`

需要 `processing.video`（状态跟随来源，所以同时要 `playback.read`）。显示层决定视频在视频区里怎么摆：宽高比、填充方式、缩放与平移。它是播放器的显示属性，**不改变画面本身**：截图与导出（§10）不受影响，滤镜（§8）也看不到它。

```ts
plugin.video.view(): VideoView;
interface VideoView extends Disposable {
  set(value: Partial<ViewSettings>): void;   // 合并写入，最后写入生效；失败交 onError 与 status
  readonly status: FilterStatus;             // 与滤镜相同的四态
  onStatus(listener: (s: FilterStatus) => void): Disposable;
}
interface ViewSettings {
  aspect: number | null;              // 显示宽高比 0.25–4；null = 按来源
  fit: "keep" | "stretch" | "fill";   // 保持比例（黑边）/ 拉伸填满 / 填满并裁切
  zoom: number;                       // 0.25–4，1 = 适应视频区
  panX: number; panY: number;         // −1–1，单位为显示出的视频宽 / 高
}
```

| 字段 | mpv 属性 |
| --- | --- |
| `aspect` | `video-aspect-override`（`null` 写 `-1`） |
| `fit` | `keep`：`keepaspect=yes`、`panscan=0`；`stretch`：`keepaspect=no`、`panscan=0`；`fill`：`keepaspect=yes`、`panscan=1` |
| `zoom` | `video-zoom = log2(zoom)` |
| `panX` / `panY` | `video-pan-x` / `video-pan-y` |

- **单一拥有者**：第一个调用 `view()` 的插件取得显示层（调用时就向播放器登记）。别的插件的 `view()` 照常返回句柄，但 `status` 为 `failed`（`conflict`）并交 `onError`；之后每次 `set` 都会再试一次，拥有者 `dispose` 或停用后就能取得。同一插件第二次调用 `view()` 抛错（先 `dispose` 旧的）。
- 默认值 `{ aspect: null, fit: "keep", zoom: 1, panX: 0, panY: 0 }`。拥有者 `dispose` 或停用时，播放器把五项写回默认值。
- 范围外的值、未知字段：`set` 立即以 `invalid` 进入 `failed` 并交 `onError`，请求不发出，已应用的值不变。
- 没有就绪的媒体时为 `waiting`：值照样写入播放器（这些属性跨来源保持），媒体就绪后变为 `applied`。播放器拒绝写入时为 `failed`（`backend`），并恢复写入前的值。
- **换来源时保留当前值**（与 mpv 相同）。需要「换片源时复位」的插件自己在 `player.state.sourceId` 变化时 `set({ zoom: 1, panX: 0, panY: 0 })`。
- 渲染尺寸按视频区确定（§5.4），所以「不保持宽高比」「填满」在任何窗口比例下都正确；`zoom > 1` 时渲染尺寸随之放宽（放大后不是放大的低分辨率缓冲）。

### 8.2 渲染选项 `plugin.video.render()`

需要 `processing.video`（同时要 `playback.read`）。HDR 色调映射、用户着色器和运动补偿，是 GPU 渲染器（§5.4）在显示时做的事：**不改变画面本身**，截图与导出（§10）得到的是没有这些效果的帧，滤镜（§8）也看不到它们。

```ts
plugin.video.render(): RenderOptions;
interface RenderOptions extends Disposable {
  set(value: Partial<RenderSettings>): void;  // 合并写入，最后写入生效；失败交 onError 与 status
  readonly status: FilterStatus;              // 与滤镜相同的四态
  onStatus(listener: (s: FilterStatus) => void): Disposable;
}
interface RenderSettings {
  toneMapping: "auto" | "off" | "bt.2390" | "hable" | "mobius" | "reinhard" | "clip";
  targetPeak: number;     // SDR 显示器亮度（尼特）100–400
  peakDetect: boolean;    // 逐场景估算峰值亮度
  shaders: string[];      // mpv 用户着色器（.glsl），绝对路径，文件要存在，最多 16 个，按顺序执行
  interpolation: boolean; // 运动补偿
}
```

| 字段 | mpv 选项 |
| --- | --- |
| `toneMapping` | `tone-mapping`：`auto` 只映射 HDR 来源；算法名原样传；`off` 写 `tone-mapping=clip` 与 `gamut-mapping-mode=clip`（不做曲线和色域压缩，超出目标亮度的部分直接裁掉），同时不做峰值检测 |
| `targetPeak` | `target-peak` |
| `peakDetect` | `hdr-compute-peak`（`yes` / `no`） |
| `shaders` | `glsl-shaders`（逐个加入，路径里可以有 `:`） |
| `interpolation` | `interpolation=yes`、`video-sync=display-resample`、`display-fps-override` = 主显示器的刷新率（读不到时 60）；关时 `interpolation=no`、`video-sync=audio` |

- **单一拥有者**，规则同显示层（§8.1）：第一个调用 `render()` 的插件取得渲染选项；别的插件的句柄为 `failed`（`conflict`），之后每次 `set` 再试。同一插件第二次调用抛错。`render()` 和 `view()` 是两个独立的句柄，可以由不同插件拥有。
- 默认值 `{ toneMapping: "auto", targetPeak: 203, peakDetect: true, shaders: [], interpolation: false }`。拥有者 `dispose` 或停用时，播放器把这些选项写回 mpv 自己的默认值（`target-peak=auto` 等，效果与默认值相同）。
- 范围外的值、未知字段、相对路径：`set` 立即以 `invalid` 失败，请求不发出。着色器文件不存在：播放器以 `invalid` 拒绝。播放器拒绝写入时为 `failed`（`backend`），并恢复写入前的值。
- **软件渲染**（§5.4 后备）：`set` 以 `unsupported` 失败，句柄不占有渲染选项。之后每次 `set` 都会再试（结果相同，播放器不会从软件渲染切回 GPU）。运行中退回软件渲染时，拥有者的 `status` 变为 `failed`（`unsupported`）。
- **着色器编译失败**：mpv 在第一次用到着色器时编译，所以 `set` 先报 `applied`，随后播放器把 `status` 推为 `failed`（`code: "failed"`，`message` 是 `shader compile failed: ` 加编译器的第一行错误，例如 `ERROR: 0:39: Use of undeclared identifier 'HOOKED_tex'`）。mpv 不报是哪个文件；列表里其它着色器照常执行。每次 `set` 最多推一次。
- 没有就绪的媒体时为 `waiting`：值照样写入（跨来源保持），媒体就绪后变为 `applied`。
- 平台与限制：只有 macOS 有 GPU 渲染，Windows、Linux 总是 `unsupported`。macOS 的 OpenGL 是 4.1，没有计算着色器，mpv 会关掉动态峰值检测，`peakDetect` 照样接受但不起作用。输出总是 SDR（8 位），显示器支持 EDR 时也不输出 HDR；`toneMapping: "auto"` 在 HDR 来源上总是做映射。运动补偿按主显示器的刷新率重采样，窗口在别的刷新率的显示器上时可能不够平滑。

## 9. 字幕 `plugin.subtitles`

读取需要 `subtitles.read`，修改需要 `subtitles.control`。

```ts
interface Subtitles {
  readonly state: SubtitlesState | null;  // 首次读取前为 null
  onChange(listener: (s: SubtitlesState) => void): Disposable;
  refresh(): Promise<SubtitlesState>;
  select(track: SubtitleTrack | null, opts?: { role?: "primary" | "secondary"; ifCurrent? }): Promise<SubtitlesState>; // null = 关闭
  load(bytes: Uint8Array, opts?: { format?: "srt" | "ass"; title?: string | Text /* §6.11 */;
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
- `secondaryPosition`：`"top"` 在画面顶部；`"aboveMain"` 在主字幕上方（按底边距加两行主字幕的高度估算，mpv `secondary-sub-pos`）。随生效的样式层（最后设置样式的插件）。主字幕是图形字幕（`styleSupported: false`）时 `setStyle` 原本以 `unsupported` 失败；选了次字幕时改为接受（样式作用于文本次字幕，对图形主字幕无效）。
- 次字幕的字体、字号、颜色、描边与主字幕相同（同一个样式层）；mpv 0.41 没有单独的次字幕字号，不提供「次字幕为主字幕 85%」。
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
    | { format: "png" | "bmp"; destination: string; sampleTime?: number }
    | { format: "jpeg"; destination: string; sampleTime?: number; quality?: number } // 1–100，默认 90
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
- 单帧格式：`png`（RGB 24 位）；`jpeg`（RGB，`quality` 为 1–100 的整数，默认 90，其它值 `invalid`）；`bmp`（24 位 RGB，与 PNG 逐像素相同）。JPEG 与 BMP 由播放器从校验过的 PNG 转换，转换完成后才发布到 `destination`（同样不覆盖已有文件），中间文件由播放器删除。扩展名不决定格式，插件按 `format` 自己命名。
- Windows：与 macOS 相同。离线工作进程照样把持有的源文件接到 FFmpeg 的标准输入，FFmpeg 用 `fd:` 协议读它（macOS、Linux 是 `/dev/fd/0`），所以 Windows 需要 FFmpeg 6.1 或更新（更早的版本没有 `fd` 协议，任务以 `failed` 结束，`message` 是 FFmpeg 的错误）。`ffprobe` 在 FFmpeg 同一目录下找 `ffprobe.exe`。
- 同一快照可多次导出；用完 `dispose()`。

## 11. 来源解析 `plugin.resolvers`

需要 `source.resolve`。用户（或任何插件）`player.open` 一个来源时，如果有插件登记了匹配的解析器，播放器先交给它解析。解析器有两种：

- **网页解析器**（`schemes` + `hosts`）：把网页地址解析成可播放的流地址。
- **文件解析器**（`extensions`，播放器 0.4.1，SDK 1.5.3）：按扩展名接管本地文件，返回由本插件生成画面和声音的流（模拟器、合成器等）。另需 `producer.av`。

```ts
resolvers.add(options:
  | {
      id: string;
      schemes: ("http" | "https")[];
      hosts: string[];    // 精确主机名
      resolve(url: string, signal: AbortSignal): Promise<ResolvedMedia>;
    }
  | {
      id: string;
      extensions: string[]; // 小写、不带点
      resolve(path: string, signal: AbortSignal): Promise<ResolvedStream>;
    }
): Resolver          // { ready: Promise<void> } & Disposable

type ResolvedMedia = {
  kind: "media";
  originalUrl: string;
  title?: string;
  primary: { url: string; headers?: Record<string, string> };
  audio?: { url: string; headers?: Record<string, string> }; // 音视频分离时
};

type ResolvedStream = {
  kind: "stream";
  width: number; height: number;   // 帧的限制同 openStream（§5.2）
  frame(): StreamFrame;
  started?(stream: Stream): void;  // 流开始播放后交出 Stream
};
```

**共同规则**

- 一个 scheme + 主机组合、一个扩展名，都只能有一个解析器。别的插件已登记时，`ready` 以 `conflict` 失败（消息如「文件类型 .nes 已由 vestra.nes 解析」），但**不让激活失败**：插件照常运行，播放器显示屏幕提示，插件管理里显示提示（`PluginInfo.error`，以及 `PluginInfo.warnings` 中的 `{ code: "resolver-taken", owner, target }`，`target` 如「文件类型 .nes」或 `https://主机`）。同一插件自己重复登记仍是 `conflict` 且让激活失败。启动时内置插件先登记，其它插件的登记请求等内置插件激活完成后再处理。
- 解析沿用原来那次 `open` 的仲裁：`ifCurrent`、操作 id 不变；打开方的 `open` 与 `media.ready()` 和打开普通文件一样结束。
- `resolve` 抛错时这次打开失败：`phase` 为 `"error"`，`state.error` 是错误的 `message` 原文；抛的是 `plugin.i18n.error(…)`（SDK 1.5.11，§6.11）时是按插件词典翻译的文字（≤ 2048 字符；播放器 0.4.1 之前带 `resolver failed: ` 前缀，SDK 1.5.3 之前发送的是 `String(error)`）。
- 打开被替换、被关闭（包括 `player.close()`、F4）、30 秒内没有答复，或解析器被注销、插件停用时，`signal` 中止，迟到的答复作废。
- 插件停用时它登记的解析器随之注销：之后这些地址或文件按普通来源打开。

**网页解析器**

- 两路流的 `headers` 必须相同，不同会明确失败。

**文件解析器**

- 扩展名 1–16 个，每个 1–16 个小写字母或数字，不能是 `folder`。只匹配本地绝对路径（扩展名不分大小写，`/roms/Game.NES` 也算）；URL 永远不匹配。
- 选中后 `state.kind` 为 `"stream"`，`state.source` 是打开的路径；`player.open(path, { paused: true })` 让流以暂停开始。生成流的限制同 `openStream`：不能跳转、变速、重播（`can` 中为 `false`），不经过滤镜（§8）。
- `resolve` 返回后，SDK 准备并预填充生成端，再作为答复交给播放器；播放器选中后 SDK 确认启动控制，然后**同步**调用 `started(stream)`，早于任何 `onStop`。`Stream` 与 `openStream` 返回的相同，属于登记解析器的插件：`stop()` 关闭来源（仍是当前来源时），插件停用时一并停止。
- `resolve` 返回之后、流开始播放之前被取消（打开被替换或关闭、插件停用）：SDK 释放生成端，不调用 `started`。
- 打开方不需要 `producer.av`，只需要 `playback.control`；文件选择、拖放、播放列表等入口统一用 `player.open(path)`。
- 用到文件解析器的插件设 `"minPlayerVersion": "0.4.1"`：旧播放器不认识 `extensions`，登记以 `invalid` 失败。

**有解析器的文件类型**（播放器 0.4.2，SDK 1.5.8）

```ts
resolvers.fileTypes: readonly string[]                         // 只读快照
resolvers.onFileTypesChange(listener: (types: string[]) => void): Disposable
```

- 所有插件登记的文件解析器的扩展名：小写、不带点、排序、去重，不给登记者。播放列表等按扩展名过滤文件的插件，用它在自带的媒体类型之外接受这些文件（如 NES 启用时的 `.nes`）。
- 读取不需要能力：激活前随 `window.state` 读到一次，插件第一次读时再读一次。变化通知需要 `events`：解析器登记、注销或其插件停用时，宿主推送 `resolvers.fileTypes` 事件，快照随之更新；没有 `events` 时监听不会触发，快照停在第一次读取时。旧播放器上是 `[]`。
- `TestHost`：`host.fileTypes()` 是本插件的文件解析器加上 `host.otherResolvers` 的扩展名；改了 `otherResolvers` 之后调用 `host.updateFileTypes()` 推送变化。

## 12. 插件包管理 `plugin.packages`

`validate`、`validateArchive`、`rollback`、`setEnabled`、`uninstall` 需要 `plugins.manage`（插件工坊、插件管理）；`list`、`onChange` 不需要能力（关于窗口等也可以读）。

```ts
interface Packages {
  validate(directory: string): Promise<PackageCandidate>; // 校验目录中的包
  validateArchive(file: string): Promise<PackageCandidate>; // 校验 .vxp 包文件（0.2.0）
  rollback(id: string): Promise<PackageChange>;           // 回退到上一版本
  list(): Promise<PluginInfo[]>;
  onChange(listener: (plugins: PluginInfo[]) => void): Disposable;
  setEnabled(id: string, enabled: boolean): Promise<void>;
  uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>;
}
interface PluginInfo {
  id: string; version: string | null;
  origin: "installed" | "development";   // 安装库 / --plugin 开发加载（试载已删除，§17）
  state: "starting" | "running" | "disabled" | "failed" | "stopped";
  error: string | null;     // failed 时的原因；running 时为提示（面板位置、上一个/下一个或解析器被占，§6.3、§11）
  warnings: { code: string; slot?: string; owner?: string; target?: string }[];
    // SDK 1.5.2：被别的插件占用而没拿到的东西；code 为 "slot-occupied"（带 slot）、"sequence-taken"、
    // "resolver-taken"（0.4.1，带 target），以后可能增加；owner 是占用者的 id
  capabilities: string[];
  directory: string;        // 包目录（0.4.3 之前的播放器：显示字段读其中的 manifest.json）
  display?: { name: string | null; punchline: string | null; description: string | null; tags: string[] };
    // SDK 1.5.9、播放器 0.4.3：manifest 显示字段按界面语言、用该包自己的词典翻译后的文字
  previousVersion: string | null; // 可回退到的版本
  builtin: boolean;         // 0.2.0：随播放器安装，只能停用
  source: PackageSource | null;   // 0.2.0：已安装插件的来源；开发加载为 null
  dataDirectory: string | null;   // 0.2.0：它的数据目录（§3.4）
}
type PackageSource =
  | { kind: "repository"; url: string }   // index.json 的完整地址
  | { kind: "link"; url: string }         // 插件包链接
  | { kind: "file" }                      // 本地 .vxp 或目录
  | { kind: "workshop" }                  // 插件工坊生成
  | { kind: "bundled" };                  // 随播放器安装（只由播放器设置）
interface PackageCandidate {
  readonly directory: string; readonly digest: string;
  readonly manifest: PackageManifest; readonly checks: string[];
  install(options?: { source?: PackageSource; replace?: boolean }): Promise<PackageChange>;
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

- 安装和回退从不重复提交：请求 id 与预期修订号由 SDK 处理，答复丢失时只按同一请求查询。播放器保留最近至少 512 条操作回执（满了淘汰最早结束的，进行中的不淘汰）；更早的请求 id 查询为 `missing`，再次提交按新请求处理。
- `list`：运行中的插件、安装库里的全部插件（含停用、启动失败、未运行的）与本次运行的开发加载，按 id 排序。`onChange` 在列表变化（启动、就绪、停用、失败、安装、卸载）后收到新列表，约每 200 ms 至多一次。
- `setEnabled(id, false)`：停止该插件（与播放器停用插件相同，§3.2）；已安装的插件写进安装库，之后启动播放器也不再运行，直到 `setEnabled(id, true)`；开发加载只在本次运行有效。`setEnabled(id, true)` 重新加载它。停用自己 `invalid`，未知 id `missing`，正在安装 `busy`。安装或回退一个停用的插件会让它重新启用。
- `uninstall`：只对已安装插件（内置插件、开发加载 `unsupported`；未知 `missing`；自己 `invalid`）。先停止，再从安装库删除（当前与上一版本，一次原子提交），回收它的包文件；`removeData: true` 时在进程结束后删除它的数据目录，否则安装实例转为休眠，同来源重新安装时接上数据。系统密钥库里该实例的条目不会被删除（无法枚举），但新实例读不到它们。用户覆盖层里它的绑定保留（`bindings.list` 的 orphans）。返回的 `PackageChange` 通常已是 `committed`。
- 候选包（播放器 0.4.2）：播放器为所有插件共保留最近 32 个 `validate`/`validateArchive` 的候选包，再校验时丢掉最早的；被丢掉的候选包 `install` 以 `stale` 失败，重新 `validate` 即可。0.4.2 之前候选包从不释放，一次运行里第 33 次校验起以 `busy` 失败。
- `validateArchive(file)`：`.vxp` 是 zip，根目录放 `manifest.json`（`vestra-plugin pack` 的产物）。播放器在解压过程中强制上限（包 ≤ 64 MB、≤ 1024 个文件、单文件 ≤ 32 MB、总计 ≤ 64 MB），只收 stored/deflate、不收加密，拒绝绝对路径、`..`、`\`、符号链接、忽略大小写重名；不合格为 `invalid`。之后的检查与 `validate` 相同。播放器不知道包的 sha256，从网络下载的包请先自己核对。
- `install({ source, replace })`（0.2.0）：来源决定升级还是替换。来源标识：`repository` 按 `url`；`link` 与 `file` 同为「手动安装」；`workshop` 单独一类。同 id 同来源 = 升级（沿用安装实例与数据，可回退）；同 id 不同来源：不带 `replace` 以 `conflict` 失败、什么都不做——插件问过用户后带 `replace: true` 重试；替换后新插件拿新的空数据目录，旧实例转为休眠（数据保留），以后从旧来源再安装时接上。同来源安装**更低**的版本同样按升级处理：播放器不比较版本号，沿用实例与数据，原来的当前版本成为 `previousVersion`（可以再回退过去）；商城的「安装其它版本」依赖这一点。不带参数等于 `{ source: { kind: "file" } }`。`source` 的地址须为 `http(s)`、≤ 2048 字节，`bundled` 为 `invalid`。同一时间每个 id 只运行一个。
- **内置插件**（`builtin: true`，随播放器安装、来源 `bundled`）：可以停用、启用、回退；`uninstall` 与任何 `install`（带不带 `replace`）都以 `unsupported` 失败（内置插件的 id 都是保留 id，同 id 的包从 0.4.1 起在 `validate` 时就以 `reserved` 失败，见下条）。内置插件只随播放器更新：播放器启动时，安装包里的版本比上次播种的版本新才升级（保留用户的停用状态与数据；用户回退后不会被同一安装包再次覆盖）。
- **保留 id**（播放器 0.4.1）：`vestra` 与 `vestra.` 开头的 id（不分大小写）只留给内置插件。`validate`、`validateArchive`、`install` 遇到这样的包以 `reserved` 失败，`message` 为「「<id>」：vestra. 开头的 id 只留给播放器内置插件，不能安装」（0.4.2 之前结尾是「不能安装或试载」）。商城安装、侧载（拖入 `.vxp`、从文件安装、从链接安装）、插件工坊与 AI 的安装都走这几个方法，所以都一样被拒绝。`rollback` 一个保留 id 只对内置实例有效（即回退内置插件，与以前相同）。保留 id 只有一种装进来的途径：安装包的内置插件集合（`builtin.json`，播种时来源 `bundled`）。
- 这条规则之前装进安装库的非内置保留 id 插件：启动时不运行，在 `list` 里是 `failed`，`error` 为上面那句话；用户可以卸载它。
- 开发加载（`--plugin`）与已安装插件同 id 时，本次运行只运行开发加载的那个。开发加载保留 id 仍然可以（内置插件的作者用 `--plugin` 开发），播放器在 stderr 打印一条警告；开发加载从不写进安装库，数据在 `plugin-data/development/<id>/`。

## 13. 错误

失败一律是普通 `Error`，带稳定的 `code`（类型 `PluginError = Error & { code: string }`）。按 `code` 判断，不要按 `message` 文本判断。

| code | 含义 |
| --- | --- |
| `stale` | 条件不再成立（来源或字幕选择已被改变），**什么都没做**。`staleDigest`：`PackageCandidate.install` 时候选包的文件在 `validate` 之后变了，重新 `validate` |
| `missing` | 面板位置为空；`showMenu` 的命令不属于本插件；`system.reveal` / `openPath` 的路径不存在；`clipboard.writeImage` 的文件不存在 |
| `unsupported` | 也用于当前系统做不到的功能（各节的平台差异、§16.7），如 Linux 上的剪贴板、确认框、`system.*`、`secrets`；以及当前来源做不到的，如生成流上的滤镜（§8） |
| `capability` | manifest 未声明所需能力 |
| `invalid` | 参数不合法（如滤镜值超出范围） |
| `busy`、`conflict`、`unavailable`、`unsupported` | 播放器拒绝接纳，未执行（`conflict` 也用于已有别的播放序列提供者、菜单路径已被占用、面板位置已被别的插件占用（这两种不算激活失败），以及同一插件里重复的窗口 id 或命令 id；`unavailable` 也用于 `next`/`previous` 不可用、面板未显示、无界面模式） |
| `backend` | 载入或解析失败，`message` 为后端原因 |
| `disabled` | 插件正在停用 |
| `replaced` | 已被接受，但完成前被另一个来源替换（`open`、`openStream`、`media.ready`） |
| `failed`、`ended` | `media.ready()`：载入失败 / 已播完。`failed` 也用于 `tools.run` 非零退出（`message` 为 stderr 最后一行）、`system.*` 打开失败、剪贴板被系统拒绝写入、`secrets` 读写失败 |
| `aborted` | `signal` 中止（`error.name === "AbortError"`） |
| `timeout` | `openStream` 启动超时；`error.stage` 为 `"prefill"`（首批缓冲未在 10 秒内备好）或 `"activation"`（选中后 10 秒内未激活）。`tools.run` 超过 `timeoutMs`；`system.*` 超过 10 秒 |
| `stream`、`device` | `openStream`：生成端报错 / 音频设备未就绪 |
| `backend-uncertain` | 滤镜阶段被播放器后端阻塞，`filter.retry()` 重新检查 |
| `limit` | 超出播放器的数量上限（如命令数，§4；密钥名字数，§16.6）；属性值超出上限（§6.1，播放器 0.4.3 起，以前为 `host`）；请求超过 1 MiB 消息上限 |
| `versionMismatch` | 包的 `minPlayerVersion` 高于本播放器（`validate`、`validateArchive`；§2） |
| `reserved` | 包的 id 是保留给内置插件的 `vestra` / `vestra.*`（不分大小写），不能安装（`validate`、`validateArchive`、`install`；§2、§12） |
| `noSource` | `media.capture`：没有可截图的来源 |
| `released`、`disposed` | 已 `release()` 的 `Snapshot` 再 `export`；已释放的 `ExportJob` 句柄还在等结果 |
| `unknown`、`rejected`、`expired` | 传输异常后 SDK 查询仍无法确认结果（正常情况下不会出现） |

处理函数里抛出的错误交给所在作用域的 `onError`。忘记 `await` 的 promise 被拒绝时只写日志，不会让插件进程崩溃。

请求没有客户端超时：管道有序，播放器对每个请求都答复或停止插件；超过 10 秒的请求只写一条日志并继续等待。

## 14. 测试工具

```ts
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";

const host = new TestHost();
const { stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
```

`TestHost` 是内存中的假播放器，说真实协议，SDK 的全部逻辑照常运行。`startPlugin` 读取插件目录的 `manifest.json`。

| 类别 | 成员 |
| --- | --- |
| 操作 | `host.command(id).run()`（禁用或忙碌时同样拒绝）；`host.press(binding, { panel? })`（按绑定表执行，`panel: true` 模拟面板有焦点；SDK 1.5.12 起与播放器一样让出禁用命令的按键（§4.1）：交给被它压下的命令、再到全局范围，执行第一个可用的命令，resolve 为 `"ran"`；只剩禁用的命令时，第一个带 `reason` 的把原因写进 `host.osd` 并吃掉按键，resolve 为 `"reason"`，都不带则 resolve 为 `"passed"`（交给焦点控件，什么都不执行）。没有绑定或要执行的命令忙碌时拒绝。1.5.11 及以前禁用命令的按键同样拒绝）；`host.window(id).emit(callback, ...args)`；`host.mounted().emit(...)`；`host.dialogAnswers.push([path] \| null)`；`host.step("next" \| "previous")`（模拟别的插件调用 `player.next()`） |
| 模拟来源 | `host.openElsewhere(path, phase?)`（另一个插件打开了媒体）；`host.setSource({ phase, position, duration, … })` |
| 界面语言 | `startPlugin(…, { locale })` / `host.locale`：加载插件该语言的词典；`host.command(id).display`、`host.translate(text, args)`、`host.confirmTexts`、`host.fileDialogs`、`host.loadCatalog(directory)`；文字值（§6.11）在 `props`、`osd`、时间轴、字幕标题里都按播放器的方式显示；`host.language`、`host.setLanguage(…)`（§6.10、§6.8）。核心命令的 `declaration.path` 是英文 key |
| 断言 | `host.window(id).props`（kebab-case 名）、`.visible`；`host.command(id).state`、`.declaration`；`host.conflicts`（`[绑定, 赢家, ...被压下]`）；`host.layout.bottom` / `.sidebar`（可直接改后 `host.emit("window.state", {})` 模拟别处的变化；`.panels` 为页签、`.owner` 为当前页签，被测插件是 owner 1）；SDK 1.5.2 起 `host.mountOther(slot, plugin, { title })` / `host.unmountOther(plugin)` 模拟别的插件的面板（侧栏为另一个页签；底栏、背景层被占时被测插件的挂载以 `conflict` 失败，`startPlugin` 不因此失败），`host.selectTab(plugin)` 模拟点页签；`host.osd`（显示过的提示）、`host.osdPanel`；`host.sequence`（提供者的标志）；`host.menus`（`showMenu` 记录）；`host.sent(method)`；`host.source`；`host.processing`、`host.subtitles`、`host.snapshots`、`host.jobs`、`host.producers` |
| 模拟数据 | `host.stats`（`player.stats()` 的返回值，第 2 轮新增字段可省略，SDK 补 `null`）；`host.dataDirectory`（`plugin.dataDirectory`，`storage.json` 直接读写其中的真实文件）；`host.setSource({ audioTracks })`（`addAudioTrack` / `selectAudioTrack` 有默认处理，可用 `host.handle("audio.addTrack" \| "audio.selectTrack", …)` 注入失败）；`host.subtitles.tracks` 的条目可带 `codec`、`text`、`streamIndex` |
| 通用能力 | `host.clipboard`（`{ text, images }`，预置 `text` 测 `readText`）；`host.power`（合并后的 `"off"` / `"playing"`）；`host.tools.handle(tool, (args) => ({ stdout, stderr }))`（未注入的工具 `unavailable`；抛 `HostError` 模拟失败），`host.tools.calls`；`host.system.revealed`、`.opened`（播放器收到的 `system.reveal` / `openPath` 路径；用 `host.handle("system.openPath", …)` 等注入失败）；`host.subtitleLoads`（每次 `load` 的 `format`、`title`、`select` 与字节）；`host.system.urls`（`system.openUrl`） |
| 渲染轮 | `host.videoRender`（`{ owner, values, gpu }`；`gpu: false` 模拟软件渲染，`set` 以 `unsupported` 失败）；`host.setVideoRenderOwner(pluginId \| null)`；`host.failVideoRender("failed" \| "unsupported", message)`（播放器推给拥有者的失败：着色器编译失败、退回软件渲染）；`host.stats.video` 可带 `hwdec`、`transfer`。着色器文件是否存在按真实文件系统检查；`host.producerTransport = "tcp"`（`openStream` 走回环 TCP 加令牌，如 Windows；`producer.admitted` 表示令牌正确） |
| 可移植性轮 | `host.secrets`（`Map`：名称 → 值，可预置）；`host.systemInfo`（`app.system()` 的返回值）；`host.exports`（每次 `jobs.start` 的参数：`kind` 为 `rawPng`、`rawJpeg`、`rawBmp`、`rawMp4` 或对应的 `processed…`，JPEG 带 `quality`） |
| 交互对齐轮 | `host.setMainWindow({ fullscreen })` 同播放器一样把附着的侧栏收起（`effective: false`、`reason: "fullscreen"`），退出全屏时恢复；`host.popSidebar(popped)` 模拟指针到右边缘弹出 / 离开后收起（只在全屏、偏好为显示、未分离时有效）；全屏中 `setVisible(true)` 即弹出；`volume-up` 上限 100（同 §5.5）；`setSource({ phase: "loading" })` 让已挂载的背景层 `effective` 变为 `true` 并推送 |
| 上限 | `commands.register` 按播放器的同一套命令上限拒绝（`limit`，§4）：本插件最多 `COMMAND_LIMITS.perPlugin` 条，总量把 `host.coreCommands` 算在内。SDK 1.5.10 起，窗口、面板、设置页的属性值按 `UI_VALUE_LIMITS` 检查（§6.1），超出时以 `limit` 拒绝，消息与播放器相同 |
| 第 4a 轮批次 2 | `host.videoView`（`{ owner, values }`：拥有者插件 id 与播放器持有的值）；`host.setVideoViewOwner(pluginId \| null)`（模拟别的插件先占用显示层，`null` 释放并复位） |
| 第 4a 轮批次 1 | `frameBlend` 与播放器同一套校验，「全部插件最多一个」按本插件的全部视频滤镜计算（TestHost 只模拟一个插件） |
| 第 3b 轮批次 2 | `host.drop(paths, options)`、`host.dropProviders`、`host.panelDrops`（§6.7）；`host.appearance`、`host.setAppearance(theme)`（§6.8）；`host.settings`、`host.settingsPage(id)`（§6.9） |
| 第 3b 轮批次 1 | `host.timeline.layers` / `.marks`、`host.setTimelineLayer(pluginId, marks)`（§5.6）；`host.images`、`host.imagePath(value)`（§6.1）；`host.backdrop`、`host.setBackdropEffective(bool)`（§6.3） |
| 第 3a 轮批次 2 | `host.bindings.overlay`（`"<plugin>/<command>"` → 输入，可预置；`bindings.set` 写入，`host.press` 按它匹配，可 `host.press("Mouse:LeftClick")`）；`host.packages`（`packages.list` 的返回值）、`host.setPackages(list)`（推送 `onChange`）、`host.packageCalls`；`host.audioDevices`（`setAudioDevice` 更新 `host.source.audioOutput`）；`host.subtitles.selection.secondary` |
| 第 3a 轮 | `host.mainWindow`、`host.setMainWindow(patch)`（§6.6）；`host.app`（§3.5）；`host.dialogAnswers.push({ confirmed, checked })`、`host.confirms`（§7）；`host.chapters`（`player.chapters()` 在媒体就绪时返回它）；`setLoop` 写入 `host.source.loop`，打开新来源时清除 |
| 核心动作 | `host.coreAction(id)`、`host.coreCommands`（播放器的 14 条核心命令，§5.5；`press` 先匹配它们）；`host.coreRuns`；`host.stepSeek`；`host.steps` |
| 注入故障 | `host.handle(method, fn)`：抛 `new HostError(code, message)` 模拟播放器拒绝，抛普通 `Error` 模拟答复丢失；`host.reset(method)` 恢复 |
| 同步 | `await host.settle(ms?)`：等到没有在途请求（可额外等待计时器） |
| 结束 | `await stop()` |

语义是简化的：没有解码，没有 Slint 渲染，导出任务每次查询前进一步。处理节点与音频延迟按播放器的同一套范围校验（越界的 `set` 以 `invalid` 进入 `failed`）。

## 15. 构建工具

命令行工具 `vestra-plugin` 随 npm 包 `@vestra-player/sdk` 提供，在任何插件项目里可用（插件目录默认是当前目录）：

```
vestra-plugin check [插件目录…]                  # 严格类型检查（含 tests/）
vestra-plugin test  [插件目录…]                  # node --test tests/*.test.{ts,mjs}
vestra-plugin build [插件目录…] [--version x.y.z]
vestra-plugin pack  [插件目录…] [--version x.y.z] [--out 目录]   # build 后把 dist/ 打成 <id>-<version>.vxp（zip），输出大小和 sha256
vestra-plugin i18n extract [插件目录…] [--locale <语言>]…       # 收集要翻译的文字 → locales/messages.pot，合并进每个 locales/<语言>.po（SDK 1.5.9）
vestra-plugin i18n check   [插件目录…]                         # 覆盖率与问题；词典过期时失败
```

在本仓库的 `src/` 下有按插件名调用的快捷方式：

```
npm run plugin -- check|test|build|pack <名字|路径…>
npm run plugins:build                   # 构建 src/plugins/ 下全部插件
```

- 名字指 `src/plugins/<名字>`；含 `/` 的参数是相对 `src/` 的路径（如 `examples/bookmark`）。`--all` 只覆盖 `src/plugins/`。
- `check` 沿用插件目录或上层最近的 `tsconfig.json`（到仓库根为止），没有就用内置的严格设置（`strict`、`NodeNext`、`verbatimModuleSyntax`、`erasableSyntaxOnly`）。编译器是 SDK 自带的 TypeScript 5.x。
- 插件只能 `import type` SDK；构建产物里出现 `@vestra-player/sdk` 的运行时引用时 `build` 失败。
- `build` 先 `check`，然后生成 `dist/`：`.ts` 去掉类型、相对导入改为 `.js`；`.slint`、`.mjs`、数据文件、`licenses/` 原样复制；manifest 的 `icon`、`readme`、`changelog`（以及默认的 `README.md`、`CHANGELOG.md`）复制；跳过 `tests/`、`node_modules/`、`vendor/`、其它 `*.md`、`tsconfig*.json`、`package*.json`；写入 `dist/manifest.json`（入口改为 `.js`，写入版本）。版本取 `--version`，否则 manifest，否则 `package.json`。`dist/` 不提交。
- 有 npm 依赖时，插件 `package.json` 的 `"vestra"` 字段：
  - `"bundle": true` 或 `{ "external": [...] }`：用插件自己的 esbuild（devDependency）把入口及依赖打包成一个文件。**有运行时依赖就必须打包**，否则安装后找不到依赖包。
  - `"assets": ["vendor"]`：原样复制额外的文件或目录。
- 开发时不需要构建：`cargo run --bin vestra-host -- --plugin <插件目录>`（在 `src/host/` 下，可重复 `--plugin`）直接运行 `main.ts`。
- 每个命令先按 §2 的规则检查 manifest（与宿主同一张表）；`check` 另外打印不算失败的提示（如 id 没有 `<所有者>.` 前缀）。
- `pack` 的同样输入得到同样字节（条目排序、固定时间戳）。SDK 1.0 的扩展名是 `.zip`，内容相同。
- 翻译（SDK 1.5.9）：`i18n extract` 找出 `.slint` 里的 `@tr`、交给 `commands.add`（`title`、`path`、`reason`）、`Command.set`、`settings.page`、`panels.mount`、`osd.show`、`OsdPanel.set`、`dialogs.*` 的文字（借 TS 类型检查器：字面量、条件的两支、`as const` 表的字面量联合类型）和 manifest 显示字段，写 `locales/messages.pot`，合并进已有的每个 `.po`（保留译文，新文字未翻译，消失的移到末尾作废，回来时恢复译文）；`--locale <语言>` 新建一种语言（`en` 与不支持的语言报错）。`i18n check` 报告每种语言的覆盖率、占位符不一致、缺复数形式、模板过期，以及无法翻译的文字：TS 里拼接的句子、不是字面量的值、非英文原文、Slint 里没包 `@tr` 的文字；这些只警告。`.po` 解析失败时失败；SDK 1.5.12 起 `locales/messages.pot` 过期、有 `.po` 却没有模板、`.po` 缺模板里的文字时也失败（退出码 1，CI 能发现忘了 `extract`）。插件有 `locales/` 时 `check` 也打印这些问题，但只警告。SDK 1.5.12 起 `as const` 表（任意深度）里属性或元素前的 `TRANSLATORS:` 注释同样交给译者。SDK 1.5.11 起还收集 `plugin.i18n.text`/`error`/`resolve` 的 `source` 与 `$vestraText` 字面量（只有占位符和标点的也收录），并按类型读取展开进调用选项的字段（`commands.add({ ...extra })`），读不出时报告（以前静默跳过）。`build` 把 `locales/<语言>.po` 编译成 `dist/locales/<语言>.json`，不复制 `.po`/`.pot`。

## 16. 通用能力

很多插件都需要、行为应当一致的基础能力。剪贴板、阻止休眠、显示与打开文件和链接、密钥存储由播放器提供；外部工具、设置文件由 SDK 在插件进程里实现，不经过播放器。插件里不要再用 `pbcopy`、`caffeinate`、`open`、`security` 等命令自己实现。

### 16.1 剪贴板 `plugin.clipboard`

需要 `clipboard`。

```ts
clipboard.readText(): Promise<string | null>;      // 没有文本为 null
clipboard.writeText(text: string): Promise<void>;  // ≤ 1 MiB（UTF-8）
clipboard.writeImage(path: string): Promise<void>; // PNG/JPEG（按文件内容判断），≤ 16 MB，绝对路径
```

- 写入替换剪贴板的全部内容。macOS：文本以 `public.utf8-plain-text`、图片以 `public.png` / `public.jpeg`（文件字节原样）写入，可在文本编辑、预览等应用中粘贴。Windows：文本为 Unicode 文本；图片解码后以位图（DIB）和 PNG 两种格式写入，JPEG 不保留原字节。其它系统 `unsupported`。
- 失败：`invalid`（类型、大小、非绝对路径、不是 PNG/JPEG）、`missing`（图片文件不存在）、`unavailable`（无界面模式）、`failed`（系统拒绝写入）。

### 16.2 阻止休眠 `plugin.power`

需要 `playback.control`。

```ts
power.preventSleep(mode: "off" | "playing"): Promise<void>;
```

- 任一插件要求 `"playing"` 即生效；插件改回 `"off"` 或停用时撤销它自己的要求。
- 生效期间，来源阶段为 `ready` 且未暂停（含生成流）时，播放器持有系统断言：正在显示视频帧时阻止显示器睡眠，纯音频只阻止系统睡眠；暂停、播完、关闭来源时释放。断言绑定播放器进程，播放器退出时随之结束。
- macOS：`caffeinate -d -i -w <播放器 pid>`（视频）或 `-i -w <pid>`（音频），可用 `pmset -g assertions` 查看。Windows：播放器界面线程的执行状态（`ES_SYSTEM_REQUIRED`，视频另加 `ES_DISPLAY_REQUIRED`），可用 `powercfg /requests` 查看。其它系统不生效（只写日志）。
- 失败语义：`preventSleep` 只登记要求（参数不对时 `invalid`），断言在播放条件满足时由播放器持有。启动断言失败只写播放器日志（`host.error power …`），不通知插件，相同条件下不重试，条件变化（暂停/继续、有无视频）时再试。插件不能据此提示「无法阻止屏保」。

### 16.3 外部工具 `plugin.tools`

```ts
tools.run(tool: "ffmpeg" | "ffprobe", args: string[],
          options: { timeoutMs: number; maxOutput?: number; signal?: AbortSignal }):
  Promise<{ stdout: Uint8Array; stderr: string }>;
tools.available(tool: "ffmpeg" | "ffprobe"): Promise<boolean>;
```

- 可执行文件按环境变量 `VESTRA_FFMPEG` / `VESTRA_FFPROBE`（绝对路径；安装包里宿主把它们设为随包的 ffmpeg / ffprobe，用户自己设置的优先），再按 `PATH` 查找（Windows 依次试 `PATHEXT` 的扩展名，默认 `ffmpeg.exe` 在先）；找到的结果在本插件内缓存，找不到时下次重新查找。
- 不经 shell：参数原样传给程序，路径里的空格、引号不需要转义。
- 超过 `timeoutMs`、`signal` 中止、所在作用域释放（含插件停用）都会结束子进程：先 SIGTERM，300 ms 后仍在则 SIGKILL（Windows 上两者都是立即终止）。
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
- 写入：临时文件 + 改名，按调用顺序一次一个，最后一次 `save` 的值最后落盘。文件权限 0600（只有当前用户可读写；Windows 没有这种权限位，由用户目录的访问控制保护），设置里常有私人路径与播放记录。
- `dispose()` 与插件停用时先写完待写的值。只有 `save` 会写入：原地修改 `value` 的属性不会保存。

### 16.5 文件与链接 `plugin.system`

```ts
system.reveal(paths: string | string[]): Promise<void>; // 在文件管理器中选中
system.openPath(path: string): Promise<void>; // 用默认应用打开文件或文件夹
system.openUrl(url: string): Promise<void>;   // 用默认浏览器打开 http(s) 地址
```

- 由播放器执行（请求 `system.reveal`、`system.openPath`、`system.openUrl`），不需要能力。SDK 先把路径转为插件进程里的绝对路径并检查存在，播放器再检查一次。
- `reveal`：最多 256 个路径；不存在的路径跳过，全部不存在时 `missing`。macOS 一次 `open -R` 全部选中（同一文件夹的多个文件在同一个 Finder 窗口高亮）；Windows 每个文件夹开一个资源管理器窗口，选中该文件夹的第一个路径。
- `openPath` 路径不存在 `missing`。
- `openUrl` 只接受 `http:` / `https:` 地址（按 WHATWG URL 解析并规范化，≤ 8192 字符），其它协议或格式不对为 `invalid`；不检查网址能否访问。
- macOS 用 `/usr/bin/open`，超过 10 秒 `timeout`，失败 `failed`（`message` 为它的最后一行错误）。Windows 用系统的 Shell 打开（`ShellExecute`、`SHOpenFolderAndSelectItems`），失败 `failed`。其它系统 `unsupported`。

### 16.6 密钥存储 `plugin.secrets`

需要 `secrets`。

```ts
secrets.get(name: string): Promise<string | null>;       // 没有为 null
secrets.set(name: string, value: string): Promise<void>; // 新建或替换
secrets.delete(name: string): Promise<void>;             // 不存在也算成功
```

- 每个安装实例一组（§3.4），互不可见：macOS 是登录钥匙串里的通用密码，服务名为 `vestra.plugin.<实例 GUID>`（开发加载为 `vestra.dev.<id>`）、账户名为 `name`；Windows 是凭据管理器里的普通凭据（目标名 `<name>.<服务名>`）。
- 每个插件至多 256 个不同的名字：再 `set` 一个新名字以 `limit` 失败（替换已有的名字、`delete` 后再用不受影响）。播放器在自己的安装库里记下用过的名字（不记值），`uninstall(id, { removeData: true })` 时删除这些项；不保留数据的卸载与替换只让实例休眠，项保留到以后接上，休眠列表满了不再保留的实例同样删除。开始记录名字之前（Slint 1.18 升级之前的播放器）写入的项无法列出，卸载时不会删除。
- `name` 1–64 个 `a-z`、`0-9`、`.`、`_`、`-`；`value` 1–1024 字节 UTF-8，不含 NUL；不合法 `invalid`。删除用 `delete`，不要写空串。
- 读写可能等待系统的授权提示（macOS「…想要使用钥匙串中的机密信息」），在播放器的工作线程上执行，不阻塞播放。没有可用的存储（macOS 找不到或无法写入钥匙串，Windows 没有登录会话）时 `unavailable`；用户拒绝授权和其它系统错误 `failed`（`message` 为系统的原因）。
- macOS 上，播放器重新构建后（开发版没有固定签名），第一次读取以前写入的项会再次弹出授权提示。
- 其它系统 `unsupported`。插件应保留退路（例如只在本次运行内记住，或提示用户），不要把密钥写进设置文件。
- 测试：`host.secrets` 是 `Map`（名称 → 值），可预置；用 `host.handle("secrets.get", …)` 注入失败。

### 16.7 平台差异汇总

| 功能 | macOS | Windows | 其它 |
| --- | --- | --- | --- |
| 剪贴板 §16.1 | 文本、PNG/JPEG 原字节 | 文本；图片转为位图与 PNG | `unsupported` |
| 阻止休眠 §16.2 | `caffeinate` | 线程执行状态 | 不生效 |
| 确认框 §7 | `NSAlert` 表单，自定义按钮与复选框 | 任务对话框，自定义按钮；复选框改为第二个是/否框 | `unsupported` |
| 置顶 §6.6 | `NSFloatingWindowLevel` | 总在最前 | `unavailable` |
| `setVideoArea` §6.6 | 可见区不含菜单栏与 Dock | 按整个显示器 | `unavailable` |
| 画中画 `setPictureInPicture` §6.6 | 系统画中画（AVKit，macOS 14+） | `unsupported`（以后为播放器自己的置顶小窗） | `unsupported` |
| 拖放 §6.7 | 指针与 ⌥⇧⌘ | 指针与 Alt、Shift、Ctrl（作 `cmd`） | 只按视频区规则分发 |
| `system.*` §16.5 | `open`，一个 Finder 窗口选中全部 | Shell，每个文件夹一个窗口 | `unsupported` |
| 密钥 §16.6 | 登录钥匙串 | 凭据管理器 | `unsupported` |
| `app.system()` §3.5 | 版本、构建、机型、显示器 | 版本、构建、机型、显示器 | 只有 `os`、`arch`、显示器 |
| 截图导出 §10 | 可用 | 可用（需要 FFmpeg 6.1+） | 同 macOS |
| 生成流 `openStream` §5.2 | Unix 套接字 | 回环 TCP 加令牌 | 同 macOS |
| 渲染 §5.4 | GPU 渲染（OpenGL），硬件解码 | 软件渲染，回拷硬件解码（`d3d11va-copy` 等） | 同 Windows |
| 渲染选项 §8.2 | 可用（`peakDetect` 不起作用） | `unsupported` | `unsupported` |
| 绑定 §4.1 | `Ctrl` 也认 ⌘ | `Cmd` 是徽标键 | 同 Windows |
| 关闭工具窗口、浮动面板、选项窗口 §4.1 | ⌘W | Ctrl+W、Ctrl+F4 | Ctrl+W |
| 界面字体 §6.4 | 系统默认（中文回退到苹方等） | 系统默认（中文回退待真机确认） | 系统默认 |
| `Theme.font-mono` §6.4 | Menlo | Consolas | DejaVu Sans Mono |

## 17. 变更说明

### SDK 1.5.12：禁用命令让出按键（2026-10，播放器 0.4.3，SDK 1.5.12）

- `TestHost.press` 与播放器（0.4.3，§4.1）一致：禁用的命令不再吞掉按键，按键交给被它压下的命令、再到全局范围；只剩禁用的命令时，第一个带 `reason` 的在 OSD 显示原因并吃掉按键，否则按键交给焦点控件。`press` 改为 resolve 出结果 `"ran"` / `"reason"` / `"passed"`（§14），不再因命令禁用而拒绝；没有绑定、要执行的命令忙碌时仍拒绝。断言「禁用命令的按键被拒绝」的测试改为断言 `"reason"`（及 `host.osd` 里的原因）或 `"passed"`（什么都没执行）。插件代码不用改。
- `vestra-plugin i18n check` 在词典过期时以 1 退出（§15）；`as const` 表里的 `TRANSLATORS:` 注释交给译者。
- SDK 带的控件库副本（`slint/vestra/widgets.slint`）与播放器一致：`Button` 的首选宽度跟随文字（最小宽度不变），译文较长时按钮变宽而不溢出。
- `apiVersion` 仍为 `vestra/1`。

### 多语言第 2 轮：文字值（2026-10，播放器 0.4.3，SDK 1.5.11）

- 第 2 轮插件作者的请求（R2-audio-1、R2-video-1、R2-subtitles-1/2、R2-playlist-1/2、R2-store-1、R2-workshop-1、R2-nes-1、capture #1、P-2，info、hotkeys 的导出文字，marks、subtitle-online 的标题）同一根源：固定文字与数据混排而 TS 不能拼句子。合并为一个机制：`plugin.i18n.text` / `list` / `error`（§6.11），播放器在显示处用插件词典翻译；`resolve` 只给离开播放器的文字。
- 接受文字值：命令标题、路径段、原因（§4）；OSD 文字与参数、`maxColumns`（§6.5）；确认框、文件对话框标题与 `fileName`（§7）；属性值（§6.1，任意深度）；时间轴标题（§5.6）；字幕标题（§9）；解析器错误（§11）。
- `BindingInfo.key`：与语言无关的菜单 key 路径（§4.2，P-2）。
- 模板字符串参数上限 1024 → 4096 字节；参数可以是文字值。协议 `vestra/1` 只有新增：新的 JSON 形状只在插件使用新 API 时出现。
- `i18n extract` 收集文字值的原文，展开字段读不出时报告（§15）。

### 多语言第 1 轮（2026-10，播放器 0.4.3，SDK 1.5.9）

- 界面以英文为原文，7 种语言（§6.10），启动时决定，改语言下次启动生效；`appearance.language` / `setLanguage`（§6.8）；`plugin.i18n.locale`（§6.10）。
- 播放器用插件自己的词典翻译声明（§4、§6.9、§6.3 面板标题、manifest 显示字段 → `PluginInfo.display`）、Slint 的 `@tr`（§6.4）和模板句子：`osd.show(text, args?)`、`OsdPanel.set(text, args?)`、`ConfirmOptions.args`（§6.5、§7）。
- 菜单按英文 key 合并；播放器自己的菜单 key 是英文（§4 表），旧中文标题仍是别名；核心动作的路径是 `Playback ▸ Play/Pause` 等英文 key，显示文字不变（简体中文时与之前逐字相同）。
- `vestra-plugin i18n extract|check`，`build` 编译词典（§15）；TestHost 的 `locale`（§14）。
- 播放器自己的界面文字改为英文原文 + 简体中文词典（`src/host/locales/`），选项窗口、宿主提示与拒绝消息在简体中文下与之前相同。
- 迁移：SDK《Migrate a plugin to English source text》（`docs/how-to/i18n-migration.md`）；参考迁移是 `vestra.preferences`。

### 属性值上限：写明、放宽、TestHost 执行、失败可见（2026-10，播放器 0.4.3，SDK 1.5.10）

- 缺陷 D1（vestra.playlist）：属性值的上限（每个值 4096 个、每个数组 1024 行、文本 1 MiB）只在 Markdown 一节提了一句，`TestHost` 也不执行。播放列表每行 9 个值，超过约 450 行时整个 `rows` 写入都被播放器拒绝，插件测试却全部通过；用户加入 3000 个文件时列表停在旧内容上。
- 上限写进 §6.1，并放宽到每个值 16384 个、每个数组 4096 行；深度、单个字符串、全部文本不变（全部文本受 1 MiB 消息上限约束，放宽没有意义）。放宽的依据：同时把宿主比较同长度列表的工作减半（不再逐行重复转换），一次满额写入在界面线程上约 6 ms（Apple M 系列，release），旧上限下满额约 2 ms。
- 超限的错误码由 `host` 改为 `limit`，消息以属性名开头。面板、设置页的初始属性超限时仍然不带它挂上，但不再悄悄丢掉：播放器发 `ui.error`，SDK 交给 `onError`。设置窗口未打开时的 `settings.set` 当场检查，超出即失败。
- 播放器之后才发现的界面失败（`ui.error`）现在带 `code`，SDK 交给 `onError`；以前 SDK 不处理这个事件。
- 插件运行器在发送前拒绝超过 1 MiB 的请求（`limit`），不再让播放器停止插件。
- `TestHost` 执行同样的检查；`UI_VALUE_LIMITS` 与播放器的常量由 `tests/sdk/host-agreement.test.ts` 核对。

### 修正：左键单击不再等双击间隔（2026-10）

- `Mouse:LeftClick` 绑定可勾选的命令（如「播放/暂停」），或没有同修饰键的双击绑定时，松开左键即触发（§4.1）。之前每次单击都要等过系统双击间隔（Windows 默认 0.5 秒，可在系统里调得更长），把单击绑定为「播放/暂停」时反应明显变慢。双击「全屏」时可勾选的单击命令会执行两次，状态不变。签名不变；插件不用改。
- 诊断：`VESTRA_DEBUG_KEYS=1` 时宿主为每次按键、鼠标按下打印各段耗时（`keys: trace #N +12.3 ms …`：winit 事件、绑定匹配、主循环执行、播放线程收到、libmpv 设置 `pause`、`source.state` 发布）；插件调用 `host.run` 也有同样的记录。

### 软件渲染也开硬件解码（2026-10，播放器 0.4.2 之后）

`apiVersion` 与 SDK 不变。播放器 0.4.2 及之前，`hwdec=auto-safe` 只在 GPU 渲染启动后设置，软件渲染（Windows 总是如此，以及无界面模式、`VESTRA_SOFTWARE_RENDER=1`、退回软件渲染）一直是 `hwdec=no`，`stats().video.hwdec` 恒为 `"no"`，「播放信息」的「硬件解码」显示「未使用」。现在软件渲染用回拷模式 `auto-copy-safe`（§5.4），Windows 上通常报 `d3d11va-copy`（NVIDIA 也可能是 `nvdec-copy`）；GPU 渲染不变。

### 删除试载；有解析器的文件类型；候选包释放（2026-10，播放器 0.4.2，SDK 1.5.8）

`apiVersion` 仍为 `vestra/1`。删除试载会让调用它的代码失效；只有 Vestra Player AI 用过，改为直接安装（装后可回退）。

- **删除试载**：`PackageCandidate.trial()`、`PackageTrial`、`PluginInfo.origin` 的 `"trial"`、错误码 `cleanup`（只有 `PackageTrial.stop` 用）都已删除；播放器不再处理 `packages.trial`、`packages.trialState`、`packages.trialStateByRequest`、`packages.stopTrial`（以 `unsupported` 拒绝），插件列表不再出现 `origin: "trial"`，试载命令的「试载 N · 」标题前缀、临时数据目录 `plugin-data/<id>.trial.<n>/` 也随之去掉。原因：试载只支持 `commands`、`ui.window`、`events`，列表里的 id 被改成 `<id>.trial.<n>`，`trial()` 在激活结束前就返回，扩展到全部能力还要逐项保证停止后不留状态；用 `validate` + `install` + `rollback` 代替。
- **有解析器的文件类型**：`plugin.resolvers.fileTypes` 与 `onFileTypesChange`（§11）。`window.state` 的答复多一个 `fileTypes` 字段，新事件 `resolvers.fileTypes`（`{ fileTypes }`，需要 `events`）。
- **候选包释放**：播放器只保留最近 32 个候选包，旧的以 `stale` 失效，不再在第 33 次校验时 `busy`（§12）。
- SDK 命令行：在 Node 权限模型（`--permission`）下运行时，向上查找 `node_modules/@types`、`tsconfig.json`、`.git` 碰到不可读的目录不再抛 `ERR_ACCESS_DENIED`，按不存在处理（类型检查要子进程，另需 `--allow-child-process`）。

### `vestra.` 前缀只留给内置插件（2026-10，播放器 0.4.1，SDK 1.5.5）

- 官方插件只有随播放器安装的内置插件，id 用 `vestra.<名字>`。其余插件都是第三方插件，包括托管在官方插件仓库里的；它们的 id 用 `<GitHub 用户名或组织>.<插件名>`。插件仓库里的 YouTube、Bilibili 改为 `zysilm.youtube`、`zysilm.bilibili`（播放器还没有公开发布，不做迁移）。
- `packages.validate`、`validateArchive`、`trial`（SDK 1.5.8 已删除）、`install` 拒绝保留 id 的包，错误码 `reserved`（§12、§13）。`rollback` 只对内置实例有效。启动时遗留的非内置保留 id 插件不运行，显示为 `failed`，可以卸载。
- `--plugin` 开发加载保留 id 仍然可以，stderr 打印警告。

### Markdown 进列表行（2026-10，播放器 0.4.1，SDK 1.5.4）

只新增，`apiVersion` 仍为 `vestra/1`（§6.1）。

- `MarkdownRef` 可以出现在属性值的任意深度，所在的 Slint 类型必须是 `[MarkdownBlock]`；SDK 的 `PropValue` 改为 `PropData | ImageRef`，新类型 `PropData` 是可以嵌入 `MarkdownRef` 的 JSON。嵌套的 `ImageRef` 为 `invalid`。旧播放器拒绝嵌套的文档（属性写入失败）。
- `@vestra/markdown.slint` 新增 `MarkdownBlocks`（不滚动、高度随内容）；`MarkdownView` 改为外面套一层 `ScrollView`，外观不变。两者新增可选的 `code-copy` 与 `copy-code(string)`。
- 宿主：每个（插件、视图、属性）按指纹缓存已解析的文档，未变的不重新解析；含文档的值原地 diff（改写变化的行、末尾原地增删、文档只换变化的块）。上限：每文档 256 KB / 4000 块 / 64 张图，每个值文本合计 1 MiB。

### 文件解析器：本地文件交给插件生成流（2026-10，播放器 0.4.1，SDK 1.5.3）

- `resolvers.add({ id, extensions, resolve })`：任何入口 `player.open` 这些扩展名的本地文件时，由登记的插件返回生成流（§11）。`source` 保留路径，`kind` 为 `"stream"`；`openStream` 的来源仍是 `producer://<n>`（§5.1 `source` 的说明）。
- 解析器被别的插件占用（扩展名或网页的 scheme + 主机）时以 `conflict` 失败，不再让激活失败（线上 `reason: "resolver-taken"`，带 `owner`、`target`；`PluginInfo.warnings` 多一种 `code` `"resolver-taken"`，并新增 `target` 字段）；登记请求和面板一样等内置插件先登记。
- 解析失败的文字改为插件给的原文，去掉 `resolver failed: ` 前缀；SDK 1.5.3 发送错误的 `message`。网页解析器同样适用。

### 侧栏页签（2026-10，播放器 0.4.0 之后，SDK 1.5.2）

只增不改；`apiVersion` 仍为 `vestra/1`，老插件不用改。设计见 [panel-slots.md](../architecture/panel-slots.md) 第 1 步，规则见 §6.3。

- 侧栏可以放多个插件的面板（页签，最多 8 个）：第二个侧栏面板不再以 `conflict` 失败，`ready` 照常 resolve。只有一个侧栏面板时界面不变；两个及以上时宿主画 31px 页签条。显示、隐藏、分离作用于整个侧栏；浮动窗口与全屏弹出层带同样的页签条。
- 线上：`layout.state` 的 `bottom` / `sidebar` 多了 `panels: [{ owner, plugin, title }]`（按页签顺序），`owner` 改为当前页签的面板；`layout.set` 多了 `active`（插件 id，未知为 `missing`）；后台页签的 `ui.menu` 以 `unavailable` 失败；面板的拖放坐标从页签条下方算起；`ui.mount` 不给 `title` 时浮动窗口与页签用插件的 `name`。
- 宿主属性 `tabbed`：桥接 global 声明了它时，宿主画页签条期间设为 `true`。
- SDK 1.5.2：`Panel.state` / `onChange` / `show()` / `toggle()`，`PanelSlot.panels` / `activate(plugin)`，`PanelState.reason` 的 `"otherTab"`，`PluginInfo.warnings`；`TestHost` 模拟页签与被占的底栏、背景层。播放器自带的 SDK 运行时（`@vestra-player/sdk`）升级到 1.5.2 之前，插件里这些成员不存在，插件应先判断（例如 `typeof list.toggle === "function"`），否则退回 `panels.sidebar.toggle()`。
- `ui-state.json`：侧栏状态改存 `sidebarSlot { visible, detached, active, order, floating }`；旧的 `panels[*]` 读入时迁移（播放列表那一项优先），之后不再写。

### 面板位置、上一个/下一个被占：不再让激活失败（2026-10，播放器 0.4.x）

- 底栏、侧栏已有别的插件的面板时，`panels.mount` 的 `ready` 在挂载请求时就以 `conflict` 失败（消息「侧栏已被 <id> 使用」）。之前要等组合窗口编译时才以 `ui-prepare` 失败，而激活期间的挂载失败让整个插件激活失败（例如开发加载的面板抢先占了侧栏，内置播放列表连命令都没有了）。背景层原来就是 `conflict`。
- 这类拒绝**不再让激活失败**：插件照常运行，`PluginInfo.state` 为 `running`，`error` 带提示文字；播放器显示屏幕提示。其它挂载失败（编译失败 `ui-prepare` 等）照旧让激活失败。
- 上一个/下一个（`player.provideSequence`）已有提供者时同样以 `conflict` 失败、不让激活失败（§5.4）；之前激活期间 = 激活失败。
- 启动时内置插件先挂载、先登记上一个/下一个，其它插件的这些请求等内置插件激活完成后再处理。
- 线上的拒绝另带 `reason`（`"slot-occupied"`，带 `slot`、`owner`；`"sequence-taken"`，带 `owner`），`packages.list` 的每项另带 `warnings: [{ code, slot?, owner }]`；SDK 目前不转交这些字段，所以插件读 `error.code`（`conflict`）和 `PluginInfo.error`。

### 自己的视觉身份：Aurora 配色与 V-Line 图标（2026-10，播放器 0.4.0，SDK 1.5.0）

只增不改，原有写法照常可用。用到新增项的插件设 `"minPlayerVersion": "0.4.0"`。

- 新配色 `vestra-light`（浅色，视频区仍是深色），`ThemeId` 变为 `"vestra" | "vestra-light"`（§6.8）；「设置 ▸ 外观」有「Vestra 深色」「Vestra 浅色」两项。浅色时 std-widgets 控件由播放器切到浅色。
- `vestra` 的颜色值换成 Aurora（取标志的完整色谱）：强调色 `#f72aa6`，`playing` 由紫色改为青色 `#2bb6ff`，`accent-fill` 为琥珀→洋红→紫→青，`line` 由近黑改为可见的细线色 `#2b2734`。token 名不变，读 `Theme` 的插件不用改。
- 圆角：`radius-control` 4px（原 3px），`radius-panel` 8px（原 6px）。尺寸 token 不变。
- 新库文件 `@vestra/icons.slint`：`Icon` 组件与 `Icons` 全局（§6.4 主题与控件）。
- `@vestra/widgets.slint`：底栏样式滑块的 thumb 是白芯加 2px 强调色环；面板样式滑块的轨道改用 `line`；描边图标按钮用圆头；悬停提示有圆角。尺寸与接口不变。
- 屏幕提示与信息块改为半透明深色圆角底板（§6.5），位置与字号规则不变。
- 内置插件的界面按[界面规格](../product/ui-spec.md)换了图标与外观，交互几何（按钮、点击区、菜单项的大小和位置，菜单顺序）不变；播放列表底栏的 ADD / DEL / SORT 改为同样大小的图标按钮（＋ − ⇅，带悬停提示）。

### 只保留 Vestra 配色（2026-10，SDK 1.4.0）

- 去掉第二套配色：`ThemeId` 只剩 `"vestra"`（§6.8），「设置 ▸ 外观」只有 Vestra 一项。拿 `appearance.theme` 与已去掉的 id 比较的插件在类型检查时报错（TypeScript 认为两者没有交集），运行时这个比较永远为假；删掉该分支即可。SDK 1.4.0 的 `appearance.set` 对 `"vestra"` 以外的值抛 `invalid`。
- 兼容：播放器把不认识的配色 id 当作 `vestra`，用旧 SDK 构建、仍会请求旧配色的插件不会失败；保存过旧配色的用户启动后是 Vestra。

### 滤镜与示例作者发现的缺口（2026-10，播放器 0.3.0，SDK 1.3.0）

新增与修正，原有写法照常可用。用到新增项的插件设 `"minPlayerVersion": "0.3.0"`。

- **视频滤镜在 GPU 渲染下不起作用（缺陷）**：零拷贝硬件解码的帧到不了 FFmpeg 软件滤镜，mpv 静默丢掉整条链，状态却是 `applied`（§8）。现在有视频链时改用回拷硬件解码。所有视频滤镜插件（含内置 video）都受影响，软件渲染（Windows、`VESTRA_SOFTWARE_RENDER=1`）不受影响。
- **视频空链**：`video.filter(id).set([])` 删除这条链（之前以 `invalid` 失败，效果留在画面上）。`TestHost` 与播放器一致：空链删除资源，生成流上的非空链 `unsupported`；一致性由 `tests/sdk/host-agreement.test.ts` 按 `host/tests/fixtures/processing-conformance.json` 的用例检查，宿主的 `processing.rs` 跑同一份用例。
- **生成流上的滤镜**：以 `unsupported` 失败，不再一直 `waiting` 并轮询（§8）。
- **内置菜单 id**：路径第一段写 `@video`、`@audio` 等（§4），与界面语言无关。SDK 常量 `MENU_IDS`。
- **`Theme.danger`**：错误与危险操作的颜色（Vestra `#ff5a4a`；§6.4）。SDK 1.3.0 带同步的 `theme.slint`、`palette.slint`。
- 文档补齐：滤镜需要 `events`（§8）；`openStream` 另需 `playback.control`（§2、§5.2）；生成流的帧大小与采样限制、`onStop` 不为插件自己的 `stop()` 调用（§5.2）；错误码 `staleDigest`、`noSource`、`released`、`disposed`（§13）。
- SDK：重复的窗口 id、命令 id 抛出 `conflict`（之前是不带 `code` 的 `Error`）；不合规的生成帧以 `invalid` 结束流（之前 `code` 为 `error`）；生成帧发送时只复制一次（之前两次）。

### 系统默认字体、标准关闭键（2026-10，SDK 1.1.3）

插件 API 签名不变。

- 不再强制界面字体：之前每个窗口默认用 Hiragino Sans GB（macOS）/ Microsoft YaHei（Windows），现在用系统默认字体，缺字由 Slint 1.18 按字符回退（§6.4）。西文字形、行高、宽度随之变成系统字体的，自己按像素排版文字的插件请检查。`VESTRA_FONT_FALLBACK` 环境变量删除。
- 工具窗口、浮动面板、选项窗口按系统标准关闭键关闭（§4.1、§6.2）：走与标题栏关闭按钮相同的路径（工具窗口照常 `onClose`）。之前选项窗口的 ⌘W 在 macOS 上不起作用（它的 Slint 处理器看的是 `meta`，而 Slint 在 macOS 上把 ⌘ 报告为 `control`）。
- `Theme.font-mono` 由播放器按系统设置（macOS Menlo、Windows Consolas、Linux DejaVu Sans Mono），在所有窗口（主窗口与面板、工具窗口、浮动面板、选项窗口、Markdown 代码块）生效；插件照常写 `Theme.font-mono`。它改为读 `VestraPalette.font-mono`（默认 Menlo），SDK 1.1.3 带同步的 `theme.slint`、`palette.slint`。
- 生成流（§5.2）：`Stream.stop()`、`dispose()`、停用时先关闭来源，再断开连接、释放生成端，停止不再弹「打开失败：producer transport closed」。播放器对两帧之间的断开给 0.5 秒等来源关闭，旧 SDK 的插件也不再误报；崩溃与协议错误照常报告。`TestHost` 的生成端新增 `droppedWhileSelected`。

### Slint 1.18.1（2026-10，SDK 1.1.2）

播放器链接的 Slint 从 1.16.1 升到 1.18.1。插件 API 签名不变；插件 `.slint` 受到的变化：

- `Flickable` / `ScrollView` 的 `viewport-x`、`viewport-y`、`viewport-width`、`viewport-height` 改名 `content-*`，旧名仍可用但编译时出弃用警告。`@vestra/markdown.slint` 已改（SDK 1.1.2 带同步副本）。
- `Window` 有了内置函数 `close()`、`hide()`：窗口组件自己声明同名回调时出「shadows the builtin function」警告，行为不变（调用仍落到自己的回调），建议改名。
- 取消 §6.4 的 ComboBox 例外：`current-index <=> Global.x` 不再让播放器崩溃（上游修复），播放器也不再在注册界面前试改 global 属性；单向写法照常可用。选项页注册时仍单独编译，编译失败以 `ui-prepare` 失败。
- `font-mono`（Menlo）里的中文按系统字体回退显示；Markdown 的行内代码整段显示为等宽胶囊，代码块整块等宽（§6.1）。
- 外观微调（上游）：std-widgets 控件的默认字号与 `Text` 一致（fluent 约小 7%）；`ComboBox` 最小宽度按内容计算（至少 160px）；换行文字的最小宽度为最长单词宽度；滚轮滚动带动画。焦点所在的元素变为不可见时焦点被清除。
- `secrets`：每个插件至多 256 个名字（超出 `limit`），带数据卸载时删除（§16.6）。操作回执满 512 条时淘汰最早结束的（§12）。

### 插件商城核心改动 S4：安装实例、来源、内置插件、manifest 显示字段、Markdown（2026-10，播放器 0.2.0、SDK 1.1.0）

全部是新增，已有签名不变（设计见 [S4 提案](../architecture/store-api-proposal.md)，给插件维护者的说明见 [store-changes.md](store-changes.md)）：

- manifest：可选显示字段与 `minPlayerVersion`（§2）；`versionMismatch` 错误码（§13）。
- 数据目录按安装实例（§3.4）。**文档规则变化一处**：「`<数据根>/<id>` 推出别的插件的数据目录」作废，改读 `PluginInfo.dataDirectory`。`secrets` 同样按实例隔离（已有登录失效一次）。开发加载的数据在 `development/<id>/`。未发版，不迁移旧的 `plugin-data/<id>/`。
- `packages.validateArchive`、`install({ source, replace })`、`PluginInfo.builtin` / `source` / `dataDirectory`；内置插件不可卸载、不可被安装覆盖（§12）。
- `plugin.ui.markdown` 与 `@vestra/markdown.slint`（§6.1）。
- `vestra-plugin`：manifest 全量检查、无前缀 id 提示、`build` 带上 icon/README/changelog、`pack` 产出 `.vxp`（§15）。

### 正式版命名：插件 API `vestra/1`（2026-10）

**破坏性变更，集中切换**（社区工作流 §5）：插件 API 结束实验阶段，协议版本由 `vestra.experimental/0.1` 改为正式版本 `vestra/1`。manifest 的 `apiVersion` 必须写 `vestra/1`；播放器不再接受旧值，仍写旧值的插件在加载与校验时因 API 版本不符被拒绝。接口签名与行为都不变，迁移只改这一个字段：

```diff
-  "apiVersion": "vestra.experimental/0.1",
+  "apiVersion": "vestra/1",
```

受影响：全部内置插件、`src/examples/` 示例、插件工坊生成的插件模板（已一并迁移）；已安装在托管库中的旧版插件需重新安装。产品名称同时改为 Vestra Player（窗口标题、关于窗口、空闲徽标文字、确认框标题栏），插件 id、`@vestra/` 库路径与数据目录不变。

### 交互对齐轮 A 线：核心行为（2026-10）

依据[交互规格](../product/interaction-spec.md)，交互整改第 1 轮。签名都不变，只改行为与语义；改了语义的条目写明受影响的插件。

- **全屏侧栏弹出层**（G-01，§6.3）：全屏时附着的侧栏不再「全屏时不可用」，而是收起在右边缘、指针移过去时弹出。`SlotState` 签名不变，语义变化：`reason: "fullscreen"` 改为「全屏覆盖层当前收起」；弹出时 `effective: true`、`reason: null`。全屏中 `setVisible(true)` 立即弹出。受影响：按 `preferred` / `effective` 画侧栏开关按钮的插件（playback 的列表按钮，见整改文档 G-18：`reason === "fullscreen"` 时按「开」显示）；playlist 的 `setInteraction` 在全屏下开始起作用（锁住弹出层），不用改。
- **右键菜单**（G-03 第一步、G-15、IA-CORE-01；主菜单与 `showMenu` 的面板菜单相同）：右键松开时弹出；子菜单在所属行右侧展开，右侧放不下时翻到左侧，之后更深的级别继续向左；两侧都放不下时放在盖住父级较少的一侧（被盖住的父级行点不到）；某一级比窗口高时，高度等于窗口高，上下各一条 12px 箭头条，指针停在箭头条上每 50ms 滚一行，滚轮按像素滚动，键盘 ↑↓ 移出可见区时自动滚动。指针停在子菜单行上满 400ms（Windows 读系统设置 `SPI_GETMENUSHOWDELAY`）才展开；已有子菜单展开时，停在同级其它行上同样的时间后才切换，期间指针进入已展开的子菜单则不切换；点击与键盘立即展开。Esc 先关最深一级，只剩顶层时才关闭菜单。插件不用改。
- **`ToolTipArea` 修复**（§6.4，作者报告）：以前提示显示后把计时器的 `running` 赋值为 `false`，解除了绑定，每个提示只能出现一次；现在每次悬停都会再次显示，指针离开时关闭。新增只读属性 `showing`。
- **加载中背景层生效**（G-16a，§6.3）：来源阶段为 `loading` 时 `panels.backdrop.state.effective` 为 `true`（以前保持上一状态）。受影响：visualizer 要在 `phase === "loading"` 时画加载界面（整改文档 G-16b），否则加载中视频区是它当前画的内容（空闲徽标或封面）。画中画不受影响：换来源时小窗照旧保持。
- **音量键上限 100**（G-13a，§5.5）：`volume-up`（↑、滚轮上、`host.run`）停在 100，已在 100 以上时不变；`setOutput` 仍接受 0–200。受影响：playback 的音量滑块应改为 0–100（整改文档 G-13b）。
- **屏幕提示字号随视频区**（G-10，§6.5）：不再固定 16px；信息块字号与位置随之变化。插件不用改。
- **拖动画面移动窗口**（G-05，§4.1）：窗口模式下左键拖动视频区移动窗口，拖动不触发 `Mouse:LeftClick`。插件不用改。
- **画中画期间双击视频区**（IA-CORE-12，§6.6）：放回主窗口，不再切换全屏。
- **全屏多了 `Alt+Enter`**（IA-CORE-04 第 1 条，§4.1）：插件若把 `Alt+Enter` 绑给自己的命令，会与宿主冲突（按绑定规则，宿主命令优先）。
- **窗口尺寸记忆**（IA-CORE-03）：最大化时不再记录窗口尺寸，启动时总是以普通尺寸打开。
- **指针在面板上时不隐藏**（G-04，§6.3）：指针停在底栏或全屏侧栏弹出层上时视为锁住，在其上移动算活动。插件不用改；以前为此自己在悬停时调 `setInteraction({ focused: true })` 的写法可以删掉。

### 渲染轮：Windows 缺口（2026-10）

只影响 Windows（macOS 行为不变；Windows 仍只做了交叉编译检查，没有在真机运行）：

- 截图与片段导出（§10）可用了：FFmpeg 用 `fd:` 读持有的源文件，需要 FFmpeg 6.1 或更新。
- 生成流 `openStream`（§5.2）可用了：回环 TCP 加令牌，由 SDK 处理。协议上 `producer.prepare` 的答复可能是 `socket: "tcp://127.0.0.1:<端口>"` 加 `token`；自己实现协议的代码要先发令牌。`TestHost` 新增 `host.producerTransport = "tcp"` 用来测试这条路径（假生成端校验令牌，`producer.admitted`）。
- 确认框（§7）显示插件给的按钮文字（任务对话框；可执行文件内嵌应用清单）。

### 渲染轮：GPU 渲染、硬件解码、渲染选项（2026-10）

默认行为变化（契约不变）：

- **macOS 默认用 GPU 渲染**（§5.4）：libmpv 用 OpenGL 渲染，硬件解码 `hwdec=auto-safe`，界面固定用 Slint 的 femtovg 渲染器。4K 播放时播放器的 CPU 占用大幅下降（原型实测窗口 81% → 19%，全屏 152% → 14%）。软件渲染保留作后备：无界面模式、`VESTRA_SOFTWARE_RENDER=1`、Windows 与 Linux、OpenGL 初始化失败。`stats().backend` 相应为「libmpv · GPU 渲染（OpenGL）」或「libmpv · 软件渲染」。
- GPU 渲染下画面按视频区的物理像素渲染，不再按来源分辨率封顶（缩放由 mpv 完成）；截图、导出、滤镜、显示层的行为不变。生成流的帧照旧按自己的尺寸显示，窗口改尺寸时不会被视频画面盖住。

兼容扩展：

- `plugin.video.render()`（§8.2）：HDR 色调映射、用户着色器、运动补偿；单一拥有者，需要 `processing.video`；软件渲染下 `unsupported`。类型 `RenderSettings`、`RenderOptions`；`Scope.video` 多了 `render`。协议新增 `video.render.set { values }`（返回 `{ state }`）、`video.render.release`，以及播放器推给拥有者的事件 `video.render.status`（`{ state: "failed", error }`）。
- `PlaybackStats.video.hwdec`、`video.transfer`（§5.4）。旧的测试数据可以不写这两个字段，SDK 补 `null`。
- `TestHost`：`host.videoRender`、`host.setVideoRenderOwner`、`host.failVideoRender`（§14）。

### 渲染与画中画轮 B：画中画（2026-10）

兼容扩展（不用它的插件不必改）：

- `plugin.window.setPictureInPicture(on)`（§6.6，能力 `window.control`，协商 R-W5）：系统画中画小窗；小窗的播放 / 暂停、跳转按钮回传到播放，关闭按钮同时暂停。
- `WindowState.pictureInPicture: { supported, active }`（R-W1 的画中画部分）。`window.state` 的回复和事件多了这个字段；旧播放器没有这个字段时 SDK 读作 `{ supported: false, active: false }`。
- 协议：新请求 `window.setPictureInPicture { on }`。`TestHost.mainWindow` 多了 `pictureInPicture`，`setMainWindow` 接受它。

行为变化：直接对 `window.state` 做 `deepEqual` 的测试会多看到 `pictureInPicture` 字段。GPU 渲染（macOS 默认）时小窗的帧来自 GPU 渲染器，软件渲染时来自软件渲染，插件不用区分。

### 可移植性轮 P1：平台模块、文件与链接由播放器执行、密钥、系统信息、JPEG/BMP 导出（2026-10）

兼容扩展（不用它们的插件不必改）：

- `plugin.secrets`（§16.6，新能力 `secrets`，宿主与构建工具同一张能力表）：系统的密钥存储，替代插件自己调用 `security`。
- `plugin.app.system()`（§3.5）：系统名称、版本、构建、机型、架构与显示器，替代 `sw_vers`、`sysctl`、`system_profiler`。`plugin.app` 的类型变为 `App`（`AppInfo` 加 `system()`），`AppInfo` 本身不变，按 `AppInfo` 写的函数和测试数据照常可用。
- `Snapshot.export({ format: "jpeg", quality? })`、`{ format: "bmp" }`（§10），替代导出 PNG 后用 `sips` 转换。协议的任务类型多了 `rawJpeg`、`processedJpeg`（可带 `quality`）、`rawBmp`、`processedBmp`。
- 各节补充了 Windows 的行为与 `unsupported` 的情况（§16.7 汇总）。

行为变化（签名不变）：

- **`system.reveal` / `openPath` / `openUrl` 由播放器执行**：SDK 不再在插件进程里运行 `/usr/bin/open`，而是发请求 `system.*`，播放器再检查路径与网址，在 macOS 上照旧调用 `/usr/bin/open`（同样的 10 秒超时与错误），在 Windows 上用系统 Shell。参数检查和错误码不变。直接对协议计数的测试会看到这些请求。
- **已删除（P4）**：兼容期的 `LocalServices.open` / `openUrl` 已移除，`TestHost` 直接把 `system.*` 请求记录到 `host.system`，不再经 `host.local`；替换 `host.local.open` 的测试改用 `host.handle("system.openPath", …)` / `host.handle("system.reveal", …)` 注入失败。
- `tools.run` 在 Windows 上按 `PATHEXT` 找 `ffmpeg.exe` 等（之前只找无扩展名的 `ffmpeg`）。
- 宿主的系统相关代码集中到 `src/host/src/platform/`（macOS、Windows、其它各一份）；macOS 的行为不变。Windows 版目前只通过了交叉编译检查。

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

### 第 3b 轮批次 1：时间轴标记、运行时图片、背景层、输入链接组件（2026-10）

兼容扩展（不用它们的插件不必改）：

- `plugin.timeline`（§5.6）：`marks`、`onChange`、`layer()`；新能力 `timeline.marks`（宿主与构建工具同一张能力表），新事件 `timeline.changed`。
- `plugin.ui.image` 与 `PropValue = Json | ImageRef`（§6.1）：`View.set` / `bind` 的属性值可以是图片。`Props` 的类型从 `Record<string, Json>` 放宽为 `Record<string, PropValue>`，原有写法不受影响。
- `panels.mount("backdrop", file)` 与 `panels.backdrop.state` / `onChange`（§6.3）。
- `@vestra/widgets.slint` 的 `UrlInputRow`（§6.4）。

行为说明：

- 有 `playback.read` 的插件在激活前多读一次 `timeline.marks`（与 `player.state`、`window.state` 并行）。直接对协议计数的测试会看到这个请求。
- **关闭来源后视频区不再保留最后一帧**：来源阶段变为 `empty` 时播放器清空视频图像，视频区显示背景层或 `Theme.bg-video` 纯色（之前停在最后一帧）。纯音频文件就绪后同样不显示上一个视频的最后一帧。
- **确认框挂在活动窗口上**：`dialogs.confirm` 的表单挂在当前活动的播放器窗口（如发起确认的工具窗口），不再总是挂主窗口（之前从工具窗口调用时主窗口被提到前面、工具窗口被盖住）。签名不变；插件里「确认后 `show()` 自己的窗口」的规避写法可以删除。
- `layout.state` 与 `window.state` 事件的数据多了 `backdrop`（内部字段）。

### 第 3b 轮批次 2：拖放、Theme 下发与外观、设置页与选项窗口（2026-10）

兼容扩展：

- `plugin.drops.provide`、`Panel.onDrop`（§6.7，新能力 `ui.drop`）。
- `plugin.appearance`（§6.8；`set` 需要新能力 `ui.theme`），新事件 `appearance.changed`。
- `plugin.settings.page` / `show`（§6.9，新能力 `ui.settings`）与播放器的选项窗口。

默认行为变化（签名不变）：

- **`Theme` 的颜色由播放器下发**：颜色 token 与 `accent-fill` 现在绑定到播放器设置的配色（宿主内部的 `ui/palette.slint`），默认仍是 Vestra 原值，未切换配色时外观与之前完全相同。插件 Slint 读 `Theme` 的写法不用改；`Theme` 的颜色仍是 `out` 属性，插件不能赋值（之前也不能）。切换配色后，用硬编码颜色的插件界面不会跟着变，请改用 `Theme` token。
- 每个工具窗口的源码末尾由播放器追加一行导出配色的 `export`（插件文件里不要定义名为 `VestraPalette` 的东西）。
- `window.state` 的答复多了 `appearance`（内部字段）。

### 第 4a 轮批次 1：渲染尺寸跟随视频区、渲染节奏、`frameBlend`（2026-10）

兼容扩展：

- 视频节点 `{ kind: "frameBlend", fps }`（§8）：倍帧。全部插件最多一个，播放器把它放在整条视频图的最后；导出帧率随之改变。

默认行为变化（契约不变）：

- **渲染尺寸跟随视频区**（C-SIZE，§5.4）：画面不再固定按 960×540 渲染后放大，而是按视频区物理像素、以来源分辨率封顶渲染。所有插件的画面都更清楚；全屏与高分辨率来源下宿主 CPU 占用相应升高（1080p 来源在 Retina 全屏下约多 15 个百分点，4K 来源约多 50 个百分点）。截图与导出走离线 FFmpeg，不受影响。
- **渲染节奏**：新帧一到就交给界面，不再按 16 ms 轮询，48 / 60 fps 输出不再被跳帧或重复。
- 生成流（`producer`）的帧照旧按自己的尺寸显示。
- 画中画（`window.setPictureInPicture`、`window.state.pictureInPicture`）与 GPU 渲染（`video.render()`、`PlaybackStats.video.transfer`）需要 `unsafe`，用户裁决先在隔离子包里做原型再决定（第 4a 轮任务单 §7），在此之前不发布。

### 第 4a 轮批次 2：显示层（2026-10）

兼容扩展：

- `plugin.video.view()`（§8.1）：宽高比、填充方式、缩放、平移；单一拥有者，需要 `processing.video`。
- `Scope.video` 多了 `view`；`ViewSettings`、`VideoView` 类型。
- 协议新增 `video.view.set { values }`（返回 `{ state: "applied" | "waiting" }`）与 `video.view.release`。

### 第 4a 轮修正：命令上限（2026-10）

- 命令数量上限从每插件 64 / 全部 512 提到每插件 160 / 全部 1024（§4）。video 插件第 2、4a 轮的完整菜单已注册 71 条，在真实播放器里激活失败（`command registry limit reached`），而 `TestHost` 没有执行上限，插件测试没有发现。
- `TestHost` 现在执行与播放器相同的上限；新增常量 `COMMAND_LIMITS`（`sdk/commands.ts`），SDK 测试校验它与播放器一致。
- 签名不变；超出上限的错误码仍是 `limit`（之前未写入 §13，现补上）。

### 第 4a 轮修正：`+` 键与 Option（2026-10）

- 在 `+` 有独立按键的布局（如德语）上，⌥+ 现在识别为 `Alt+Plus`（之前是 `Alt++`，绑定 `Alt+Plus` 不触发）；与无修饰的 `Plus` 和 US 布局的 ⌥⇧= 一致。绑定写法不变。
- macOS 的 ⌃F1–⌃F8 默认是系统键盘导航快捷键，会被系统截走；默认绑定请避开（§4.1）。

### 第 4a 轮修正：选项窗口布局（2026-10）

- 选项窗口（§6.9）的分类树与设置页区域现在占满底部按钮行以外的全部高度；之前按钮行与内容平分高度，设置页只得到上半个窗口（高的页面，如快捷键页，下半部分看不到），「关闭」按钮被拉成高块。设置页的写法不用改。

### 第 3b 轮修正：背景层与封面（2026-10）

- 挂着背景层时，mpv 不再把音频文件的封面选作视频轨（§6.3）；之前带封面的 MP3 让 `effective` 恒为 `false`，背景层插件的封面界面永远不出现。卸载后恢复 mpv 默认。签名不变；没有背景层的插件不受影响。
