# 第 3a 轮：输入、窗口、插件管理、音频输出、章节与循环、次字幕、确认框 + 插件接入

分支：`rebuild/features-r3a`（自第 2 轮合并到 `main` 之后的提交新建，由协调者创建）。依据：[协商记录](2026-10-features-negotiation.md)「第 3 步：裁决」（下称「裁决」）的 3a 排期与 3.3 通用能力、[功能需求](../../product/feature-requirements.md)（下称「需求文档」）、[社区工作流](../community-workflow.md) §3、§4、§5、[第 2 轮任务单](2026-10-features-r2.md) §6.2 的转入项。

## 1. 范围

只做裁决里标为「3a」的条目，加上第 2 轮转入本轮的项；不提前做 3b 以后的条目，不新增裁决之外的接口。签名做不到时维护者在本文 §6.2 记录原因与调整后的签名。

| 类别 | 条目 | 需求 |
| --- | --- | --- |
| 窗口 | R-W1 `window.state` / `onChange`、R-W2 `setVideoArea`、R-W3 `setAlwaysOnTop`、R-W4 `setCursorAutoHide`；能力 `window.control` | window H03–H05、B30 |
| 应用信息 | R-W6 `plugin.app` | info I09（第 2 轮转入）、I05 的 mpv / FFmpeg 版本行 |
| 插件管理 | R-G1 `packages.list` / `onChange`、R-G2 `setEnabled`、R-G3 `uninstall`；R-H10 停用持久化 | plugins J07；info I09 |
| 绑定 | R-K1 `plugin.bindings.*`；R-K3 `Mouse:LeftClick` + R-H11 单击判定；能力 `bindings.manage` | hotkeys J01–J03 |
| 音频输出 | R-A2 `audioDevices`、R-A3 `setAudioDevice`、R-A4 `PlayerState.audioOutput`、R-H9 生成流跟随设备 | audio G16（第 2 轮转入） |
| 章节与循环 | R-M1 `player.chapters`、R-M2 `setLoop` / `PlayerState.loop`、R-H13 换来源清除循环 | marks B21、B23 |
| 次字幕 | R-T3 `select(…, { role })`、`SubtitlesState.secondary`、`load({ select: "secondary" })`、R-T7 余项 `secondaryPosition` | subtitles A08（第 2 轮转入） |
| 通用能力（宿主） | `plugin.dialogs.confirm`（裁决 3.3） | playlist（替换 `confirm.ts`）、plugins J07、hotkeys |

已在第 2 轮完成、本轮不再列入：`Window.onClose`（第 2 轮修复提交）。

不在本轮：画中画（R-W5、R-H8，4a）、`pictureInPicture` 字段（4a）、时间轴标记层（R-M3，3b）、设置页 / 外观 / 运行时图片 / 拖放 / 背景层（3b）、`UrlInputRow`（3b）、C-SIZE、C-GPU。

## 2. 两条线与顺序

| 阶段 | 执行 | 内容 | 完成条件 |
| --- | --- | --- | --- |
| P0 | 协调者 | 建分支；记录基线（§6.1） | 基线写进 §6 |
| P1 | 维护者 | §3 全部核心项（均为兼容扩展，不需要切换开关） | 静态检查全过；参考文档、TestHost 同步；协调者审查通过 |
| P2 | 插件作者 | §4 分派 | 各插件 `check/build/test` 全过 |
| P3 | 协调者 | §5 验收，提交推送，合并到 `main` | — |

P1 可以分两批交付（先窗口、应用信息、确认框、章节与循环，再绑定、插件管理、音频输出、次字幕），作者按依赖开工（§4.2）。

## 3. 维护者清单（P1）

每一项都要同时完成：宿主实现（`src/host/`）、SDK 类型与实现（`src/sdk/`）、TestHost 模拟（`src/sdk/plugin/testing.ts`）、参考文档（`doc/plugins/reference.md`）、单元测试；新增能力名同步 manifest 校验（宿主 `plugins.rs` 与构建工具 `scripts/plugin.mjs`）。第一方 Rust 不得用 `unsafe`。签名以裁决 3.1 / 3.3 为准，下面只写实现要点。体积预算：宿主 Rust 净增约 3 000 行（含测试），超过就在 §6.2 说明并拆出。

### 3.1 窗口（R-W1–R-W4，能力 `window.control`）

```ts
plugin.window.state: { fullscreen: boolean; videoArea: { width: number; height: number } };   // 同步只读
plugin.window.onChange(listener: (state) => void): Disposable;
plugin.window.setVideoArea(size: { width: number; height: number }): Promise<{ width; height; clamped: boolean }>;
plugin.window.setAlwaysOnTop(onTop: boolean): Promise<void>;
plugin.window.setCursorAutoHide(mode: "fullscreen" | "playing"): Promise<void>;
```

- `state`：由 `window_state.rs` 现有的布局快照给出（视频区为逻辑点，不含底栏、侧栏、分隔条）；随现有 `window.state` 事件推送，变化才推送。读取不需要能力，写入需要 `window.control`。
- `setVideoArea`：核心按布局规则加上栏位，夹到屏幕可见区与最小尺寸；全屏时 `unavailable`；返回实际视频区与 `clamped`。
- `setAlwaysOnTop`：主窗口、浮动侧栏与之后新开的工具窗口都设为浮动层级（`NSWindow` level，`objc2-app-kit` 安全方法）；多插件最后写入生效；该插件停用时恢复。
- `setCursorAutoHide`：现有规则 `cursor-hidden = fullscreen && idle && playing` 中，`"playing"` 模式把 `fullscreen` 放宽为「任意窗口模式」；空闲计时与遮挡规则不变；停用时恢复 `"fullscreen"`。
- TestHost：`host.mainWindow: { fullscreen, videoArea, onTop, cursorAutoHide }`，`host.setMainWindow(patch)` 模拟用户改变窗口（名字不与工具窗口的 `host.window(id)` 冲突）。

### 3.2 应用信息（R-W6）

```ts
plugin.app: { readonly version: string; build: string | null;
              components: { mpv: string | null; libav: Record<string, string>; slint: string } };
```

- 同步只读，不需要能力。宿主在启动插件时随握手下发（环境变量或 `createPlugin` 参数，不加 RPC）：`version` = `CARGO_PKG_VERSION`；`build` = 构建时的 git 短哈希（构建脚本不可用时 `null`）；`mpv` = mpv `mpv-version`；`libav` = `ffmpeg-version` 与 `libav*` 各库版本（mpv 能给出的）；`slint` = 链接的 Slint 版本。
- TestHost：`host.app` 可预置。

### 3.3 插件管理（R-G1–R-G3、R-H10）

```ts
interface PluginInfo { id: string; version: string | null; origin: "installed" | "development" | "trial";
  state: "starting" | "running" | "disabled" | "failed" | "stopped"; error: string | null;
  capabilities: string[]; directory: string; previousVersion: string | null }
packages.list(): Promise<PluginInfo[]>;                          // 不需要能力
packages.onChange(listener: (plugins: PluginInfo[]) => void): Disposable;   // 不需要能力
packages.setEnabled(id: string, enabled: boolean): Promise<void>;           // plugins.manage
packages.uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>; // plugins.manage
```

- `list` 合并运行表、安装库（`package_store.rs` 的 current / previous）与停用集合；数据目录规则写进参考文档（`<数据根>/<id>`），不放字段。
- `setEnabled`：停用 = 现有 `disable(owner)`；启用 = 重新加载该包；停用自己 `invalid`；试载 `unsupported`；开发加载只在本次运行有效；已安装插件的停用集合写进安装库 catalog（与安装记录同一事务边界），启动时跳过。
- `uninstall`：只对已安装插件；先停用，再在 store 里写删除记录（日志式提交 + 现有 `gc`）；`removeData` 由核心删除 `plugin-data/<id>/`；覆盖层里该插件的绑定保留为 orphan。复用现有 `PackageChange` 句柄语义。
- `onChange`：加载、停用、失败、安装、卸载时推送新列表（新事件 `packages.changed`）。
- TestHost：`host.packages`（可预置列表），记录 `setEnabled` / `uninstall` 调用。

### 3.4 绑定（R-K1、R-K3、R-H11，能力 `bindings.manage`）

```ts
interface BindingRef { plugin: string /* manifest id；宿主为 "host" */; command: string }
interface BindingInfo extends BindingRef {
  title: string; path: string[] | null; scope: "global" | "panel";
  defaults: string[]; current: string[]; overridden: boolean;
  shadowed: { input: string; by: BindingRef }[]; labels: string[];
}
plugin.bindings.list(): Promise<{ commands: BindingInfo[]; orphans: { ref: BindingRef; inputs: string[] }[] }>;
plugin.bindings.set(ref: BindingRef, inputs: string[]): Promise<void>;  // [] = 解绑；校验失败 invalid；命令不存在且非 orphan missing
plugin.bindings.reset(ref?: BindingRef): Promise<void>;                 // 省略 = 清空覆盖层
plugin.bindings.onChange(listener: () => void): Disposable;
```

- 不做 `ignoreDefaults`（裁决 R-K2 拒绝，hotkeys 用 K1 实现）。
- `list` 从 `bindings::Table` 组装；`labels` 用菜单右侧文字的同一函数；`shadowed` 来自 `rebuild()` 的冲突计算。`set` / `reset` 写 `Table.overlay` 后 `rebuild()`，并经 `ui_state.rs` 去抖保存。变化（声明、注销、覆盖层）推送新事件 `bindings.changed`（只发给声明了 `bindings.manage` 的插件）。
- `Mouse:LeftClick`：新输入名（宿主与 SDK 两处语法同步）。只有当它有生效的绑定时，视频区左键按下延迟判定：系统双击间隔内、移动 ≤ 4 px 没有第二次按下才算单击；无绑定时保持现在的立即聚焦，双击判定不变。
- TestHost：`host.bindings`（覆盖层与列表模拟），`host.press("Mouse:LeftClick")` 可用。

### 3.5 音频输出（R-A2–R-A4、R-H9）

```ts
player.audioDevices(): Promise<{ id: string; name: string }[]>;   // playback.read
player.setAudioDevice(id: string | null): Promise<void>;          // playback.control；null = 系统默认
PlayerState.audioOutput: { requested: string | null; active: string | null };
```

- mpv 路径：`audio-device-list` / `audio-device`；`active` 为实际使用的设备（设备被拔后回退为默认，重新插入后切回 `requested`）；本次运行内记住，不跨启动保存（audio 插件负责保存并在激活时设置）。
- 生成流路径（`producer_audio.rs`，cpal）：按同一个 `id` 打开设备，断开后回退默认、恢复后切回。先确认 cpal 设备与 mpv CoreAudio 设备能一一对应（名称或 UID）；做不到时在 §6.2 记录，并让生成流只跟随系统默认（参考文档写明）。
- 不提供 `devicesRevision`：新插入且未被选中的设备由插件轮询发现（裁决）。
- TestHost：`host.audioDevices` 可预置；`setAudioDevice` 更新 `audioOutput`。

### 3.6 章节与循环（R-M1、R-M2、R-H13）

```ts
player.chapters(): Promise<{ sourceId: number; chapters: { title: string | null; start: number }[] }>;  // playback.read
player.setLoop(loop: { a: number; b: number; count?: number } | null, opts?: Conditional): Promise<void>; // playback.control
PlayerState.loop: { a: number; b: number; count: number | null } | null;
```

- `chapters`：mpv `chapter-list`，按需读取，不进 `source.state`。
- `setLoop`：mpv `ab-loop-a` / `ab-loop-b` / `ab-loop-count`（`count` 省略 = 无限）；`0 ≤ a < b ≤ duration`，否则 `invalid`；不可跳转的来源 `unsupported`；条件提交同其它控制；不提供剩余次数。
- 来源变化（新 `sourceId`）时核心清除 `ab-loop-*`（H13）。
- TestHost：`host.chapters` 可预置；`setLoop` 写入 `source.loop`。

### 3.7 次字幕（R-T3、R-T7 余项）

```ts
subtitles.select(track: SubtitleTrack | null, opts?: { role?: "primary" | "secondary"; ifCurrent? }): Promise<SubtitlesState>;
SubtitlesState.secondary: SubtitleTrack | null;
subtitles.load(bytes, { select?: "primary" | "secondary" | "none", … });
SubtitleStyle.secondaryPosition?: "top" | "aboveMain";
```

- mpv `secondary-sid`；`secondaryPosition` → `secondary-sub-pos`（`top` = 顶部，`aboveMain` = 主字幕上方）。
- 主、次不能是同一条，图形轨不能作次字幕（`invalid`）；来源变化清空次字幕；共用 `trackConfirmation`。
- `subtitle_state.rs` 增加一个选择槽；移除资源时同时清理次选择。
- TestHost：`host.subtitles.selection` 增加 `secondary`。

### 3.8 确认对话框（裁决 3.3，能力 `ui.dialog`）

```ts
plugin.dialogs.confirm(options: { message: string; detail?: string; confirm: string; cancel?: string;
  destructive?: boolean; checkbox?: string }): Promise<{ confirmed: boolean; checked: boolean }>;
```

- `NSAlert`（`objc2-app-kit` 安全方法：`addButtonWithTitle`、`setShowsSuppressionButton` 作复选框、按钮 `setHasDestructiveAction`）；以主窗口为父窗口的表单（sheet），没有主窗口时为模态窗口；与文件对话框共用「同一时间一个对话框」的忙碌规则；无界面模式 `unavailable`。
- 文字长度：`message` 1–200、`detail` ≤ 2000、按钮与复选框 1–40 个字符。
- TestHost：`host.dialogAnswers` 队列同样用于 `confirm`（推入 `{ confirmed, checked }`）。

### 3.9 文档

- 参考文档：上述全部接口；§2 能力表加 `window.control`、`bindings.manage`；§4.1 加 `Mouse:LeftClick`；§3.4 写明数据目录规则（供 J07「打开数据目录」使用）；§17 本轮变更说明。
- 示例：`window` 状态与 `setVideoArea`、`bindings.list/set`、`dialogs.confirm` 各一个短例（≤ 30 行）。

## 4. 插件接入分派（P2）

作者规则同第 2 轮（[2026-10-features-r2.md](2026-10-features-r2.md) §4）：只写自己的插件目录；按需求文档对应小节实现；缺核心能力时按工作流 §4.1 写请求，同时继续不依赖它的部分。插件里已有的、本轮核心提供了的自制实现（如 playlist 的 `confirm.ts`）必须删除并改用核心能力，报告列出删除的文件与行数。

### 4.1 分派表

| 作者 | 插件 | 新功能（编号 → 依赖） | 迁移到核心能力 | 降级与文档 |
| --- | --- | --- | --- | --- |
| A | window（新） | H03 倍数尺寸（`setVideoArea` + `stats().video.displayWidth/Height`）、H04 适应视频比例、H05 置顶（`setAlwaysOnTop`；「全屏时」见需求文档该节）、B30 播放时隐藏指针（`setCursorAutoHide`） | — | H07 画中画推到 4a：本轮可选「迷你尺寸」形态（W2 + W3），作者决定并写进需求文档 |
| B | marks（新） | B21 章节/书签菜单与跳转（`player.chapters`；书签用 `storage.json`）、B23 A-B 区段循环（`setLoop` / `state.loop`） | — | B22 进度条标记推到 3b（时间轴标记层）；书签列表窗口本轮做 |
| C | hotkeys（新）、plugins（新） | hotkeys：J01 快捷键设置、J02 鼠标动作（含 `Mouse:LeftClick`）、J03 忽略默认快捷键（按裁决 R-K2 的替代方案，用 `bindings.set` 与本插件的 `storage.json` 记录）。plugins：J07 列表、停用、启用、卸载、回退（现有 `rollback`）、打开数据目录（`system.openPath`） | 卸载与停用确认用 `dialogs.confirm` | J04 首选项挂载推到 3b：本轮两插件都用自己的工具窗口 |
| D | audio、subtitles | audio：G16 输出设备（`audioDevices` / `setAudioDevice` / `state.audioOutput`；设备选择用 `storage.json` 保存，激活时恢复；5 秒轮询发现新设备）。subtitles：A08 次字幕（`select({ role: "secondary" })`、`load({ select: "secondary" })`、`secondaryPosition`） | — | G16 若宿主记录生成流不能跟随设备（§3.5），菜单说明写明 |
| E | info、playlist | info：I09 关于（`plugin.app`、`packages.list`）；I05 的 mpv / FFmpeg 版本行改用 `plugin.app.components`。playlist：清空、删除等确认改用 `dialogs.confirm` | playlist `confirm.ts` → `dialogs.confirm`（删除该文件）；info 若有自拼的版本探测（`ffmpeg -version` 等）改用 `plugin.app` | — |

不在本轮的插件：preferences、visualizer（3b 起）；其余插件没有本轮条目，若用到 `Window.onClose`（第 2 轮已交付）可顺带接入（info 播放属性窗口的轮询、capture 连拍窗口勾选状态）。

### 4.2 准入

- 维护者交付顺序（§2）：第一批（窗口、应用信息、确认框、章节与循环）审查通过后作者 A、B、E 开工；第二批（绑定、插件管理、音频输出、次字幕）审查通过后作者 C、D 开工。
- 作者不等依赖的部分可以先做：界面、菜单、设置保存、TestHost 下的测试。
- 同时运行的作者不超过 3 个（含协调者共 4 个）。

## 5. 验收

### 5.1 静态检查与测试

工作流 §6.1 全部命令；各插件测试数不减少；新插件 `check/build/test` 通过；菜单快照只多出本轮新增项（`doc/process/rounds/menus-snapshot.test.ts`）。

### 5.2 真实 GUI 测试（协调者）

工作流 §6.2 的全部必测场景，加上本轮：

| 范围 | 场景 | 证据 |
| --- | --- | --- |
| 窗口 | 1 倍 / 2 倍后视频区与源视频尺寸一致（侧栏开关两种情况）；超出屏幕时 `clamped` 且窗口在可见区内；全屏时菜单灰；置顶后切到别的应用窗口仍在最上层，停用 window 后恢复；「播放时隐藏指针」在窗口模式下 3 秒后隐藏，移动后出现 | 截图、窗口尺寸 |
| 应用信息 | 关于窗口显示 Vestra 版本、mpv、FFmpeg 各库版本与 Slint 版本，与 `mpv --version` 一致 | 截图 |
| 插件管理 | 列表含全部插件及状态；停用 video 后其菜单消失、画面效果撤销，重启播放器后仍停用；启用后恢复；卸载工坊安装的插件后列表消失、数据目录按选项删除；停用自己时菜单灰 | 截图 |
| 绑定 | 改绑 Space 为 K 后 K 播放/暂停、Space 不再触发，重启后保持；恢复默认；冲突时显示占用者；「忽略默认快捷键」后默认键失效、改绑仍有效；给左键单击绑定「播放/暂停」后单击切换、双击仍全屏 | 物理按键记录 |
| 音频输出 | 设备列表与「系统设置 ▶ 声音」一致；切到耳机后声音从耳机出；拔掉耳机回退扬声器、`active` 变化，插回后切回；NES 生成流同样跟随（或按 §6.2 记录的降级） | 听感记录 |
| 章节与循环 | 带章节的 mkv 菜单列出章节并可跳转；A-B 循环在 B 点精确回跳；换文件后循环清除 | 截图、时间 |
| 次字幕 | 中英两条轨同时显示，次字幕在顶部 / 主字幕上方两种位置；主次不能选同一条 | 截图 |
| 确认框 | playlist 清空时出现系统确认框，取消不清空；带复选框的确认记住选择 | 截图 |

缺陷归属同工作流 §6.2。

## 6. 记录

### 6.1 基线

（P0 填写）

### 6.2 维护者交付与签名调整

#### P1 批次 1（2026-10-21，维护者）：窗口、应用信息、确认框、章节与循环

**实现清单**（每项：宿主 + SDK 类型与封装 + TestHost + 参考文档 + 单元测试）

| 任务单 | 内容 | 宿主 | SDK / TestHost / 文档 |
| --- | --- | --- | --- |
| §3.1 | `window.state` / `onChange`（R-W1） | `host.slint` 新增 `video-width/height`（`canvas` 尺寸）；`window_tick` 的布局事实加 `videoArea`，随现有 `window.state` 事件变化才推送；新方法 `window.state`（不需要能力） | `sdk/plugin/window.ts`：激活前读一次（与 `player.state` 并行），插件第一次读 `state` / `onChange` 后跟随事件；`host.mainWindow`、`host.setMainWindow` |
| §3.1 | `setVideoArea`（R-W2） | `window_control::fit`（加底栏与附着侧栏、按比例缩小到屏幕可见区、放大到最小 318×120 / 窗口 320×182）；`platform_window::resize_content`（`NSWindow` `setFrame_display`，保持左上角、挪回可见区）；布局显示目标尺寸（±1）或 1 秒后才答复 | resolve 后刷新 `state`；参考 §6.6 |
| §3.1 | `setAlwaysOnTop`（R-W3） | `platform_window::set_floating`：本应用所有普通层级窗口 ↔ `NSFloatingWindowLevel`（`setLevel` 安全方法）；窗口数变化（新开工具窗口、浮动侧栏）时重新应用；`window_control::LastWrite` 最后写入生效、插件停用后回退 | `host.mainWindow.onTop` |
| §3.1 | `setCursorAutoHide`（R-W4） | `cursor-hidden = playing && ((fullscreen && 底栏空闲) \|\| ("playing" && Policy::idle && (fullscreen \|\| 主窗口为活动窗口)))`；`Policy::idle` 与全屏底栏同一空闲与遮挡规则；`LastWrite`，停用回退 `"fullscreen"` | `host.mainWindow.cursorAutoHide` |
| §3.1 | 能力 `window.control` | `plugins.rs` 与 `scripts/plugin.mjs` 两处 | 参考 §2 |
| §3.2 | `plugin.app`（R-W6） | 后端启动时读 `mpv-version`、`ffmpeg-version`；`build.rs` 从 `Cargo.lock` 取 Slint 版本、`git rev-parse --short HEAD` 取构建哈希（取不到不设，`build: null`）；`plugins::set_app_info` 后经环境变量 `VESTRA_APP_INFO` 交给每个插件进程（不加 RPC） | `runner.ts` 读环境变量传给 `createPlugin` 第 5 个参数；`host.app`；参考 §3.5 |
| §3.6 | `player.chapters`（R-M1） | `source_backend::marks`：`chapter-list` → `{ title, start }`（≤ 4096 条，空标题为 `null`）；没有媒体、生成流为空 | `host.chapters` |
| §3.6 | `setLoop` / `PlayerState.loop`（R-M2） | `ab-loop-a/b/count`；校验 `0 ≤ a < b ≤ duration`（`invalid`）、可跳转（`unsupported`）、就绪媒体（`unavailable`）、生成流 `unsupported`；条件提交复用 `check_expected`；`source.state.loop` 为请求值 | `setLoop` 写 `host.source.loop` |
| §3.6 | R-H13 | `Sources::loop_state`：来源代数变化时清除 `ab-loop-*` 与请求；`count` 用完（mpv `remaining-ab-loops == 0`）时清除，`state.loop` 变 `null` | TestHost 打开新来源时清除 |
| §3.8 | `dialogs.confirm`（裁决 3.3） | `confirm.rs`：`NSAlert`（`addButtonWithTitle`、确认按钮 `setHasDestructiveAction`、取消按钮 `setKeyEquivalent("\u{1b}")`、`setShowsSuppressionButton` 作复选框）；有主窗口时 `beginSheetModalForWindow_completionHandler`（`block2::RcBlock`），否则 `runModal`；与文件对话框共用 `dialog_busy`；长度按字符计 | `host.dialogAnswers.push({ confirmed, checked })`、`host.confirms`；参考 §7 |
| §3.9 | 文档 | — | 参考 §2、§3.3、§3.5（新）、§5.1、§5.2、§6.6（新）、§7、§14、§17 批次 1；示例 §22–§24 |

**签名调整与实现取舍**（签名均与裁决一致）

1. `PlayerState.loop` 在 `types.ts` 写成可选（`loop?: … | null`）：playlist 的测试手写 `PlayerState` 对象（`tests/queue.test.ts`），必选字段会让 `npm run check` 失败，而插件目录不归维护者改。SDK 总是给出该字段（没有循环为 `null`），参考文档已写明。批次 2 的 `audioOutput` 同样处理。
2. `plugin.app` 放在 `Scope` 上（与其它模块一样子作用域也有），不只在 `Plugin` 根上。
3. `libav` 只有 `{ ffmpeg: <ffmpeg-version> }`：libmpv 0.41 没有报告 libavcodec 等各库版本的属性；参考 §3.5 写明需要时用 `tools.run("ffmpeg", ["-version"])`。实测值：`mpv: "mpv v0.41.0"`、`libav: { ffmpeg: "9.0.2" }`、`slint: "1.16.1"`、`build: "289b26b"`。
4. 剩余次数：mpv 0.41 实际有 `remaining-ab-loops` 属性（裁决写「给不出就不提供」）。按裁决签名不对外提供，只在宿主内部用它判断循环结束；以后需要时可作为兼容字段加入。`count` 的语义写明为「回跳次数」（`count: 2` 时 a–b 段共播 3 遍）。
5. `window.state` 的跟随是惰性的：每个插件激活前读一次，但只有在插件读过 `state` 或注册 `onChange` 后才订阅事件（拖动缩放窗口时每帧都有布局变化，不让所有插件都收到）。
6. `setVideoArea` 的「附着侧栏」按偏好判断（`preferred && !detached`），不按当前是否显示：窗口变大后侧栏会出现，所以宽度预先算进去；放不下时按现有规则隐藏（`reason: "space"`）。
7. TestHost 的 `setLoop` 先判断生成流（`unsupported`）再判断条件；宿主先判断条件。只影响两者同时不满足时的错误码。
8. `runner.test.ts` 的协议顺序断言更新为 `["window.state", "commands.register"]`（新增的激活前读取）。

**检查**（worktree 内实际执行）

- `src/`：`check`、`format:check`、`boundaries`、`plugins:build` 0；`test:sdk` 76/76（新增 `tests/sdk/round3a.test.ts` 4 条）；插件测试 audio 30、bilibili 15、capture 24、history 22、info 20、nes 11、playback 45、playlist 61、subtitle-online 13、subtitles 46、video 22、workshop 17、youtube 14 全过（workshop `vendor/sdk` 已按测试开头的命令重同步，同一提交）。
- `src/host/`：`cargo fmt --check`、`clippy --locked --all-targets -D warnings`（含 `--no-default-features`）0；`cargo test --locked` 187/187（新增 `window_control` 2 条、`confirm` 1 条、章节解析 1 条）。
- 无界面实测（`--headless --silent` + 临时探针插件，未进仓库；10 秒带两章的 mkv）：`plugin.app` 如上；`chapters()` → `[{Opening, 0}, {Main, 4}]`；`setLoop({ a: 1, b: 2, count: 2 })` 后位置 1→1.93→1→1.87→1→2.53（回跳两次后继续），之后 `state.loop` 为 `null`；`setLoop({ a: 5, b: 60 })` → `invalid`；重新打开同一文件后 `state.loop` 为 `null`；无界面时 `confirm`、`setAlwaysOnTop`、`setVideoArea` → `unavailable`。
- 宿主 Rust：`src/host/src/*.rs` 26 306 → 27 068（+762，含测试），另 `build.rs` 40 行；SDK 与运行器 `src/sdk/plugin/*.ts` + `src/runtime/` +330（含 TestHost 120），`src/sdk/*.ts` +41。

**给协调者的 GUI 实测要点**（需要一个调用这些接口的插件：作者 A 的 window、B 的 marks，或临时探针）

1. `setVideoArea({ width: 1280, height: 720 })`：视频区正好 1280×720（截图量），窗口左上角不动；打开侧栏再调用：窗口宽 = 1280 + 3 + 侧栏宽；在 1440×900 屏上要 3840×2160：`clamped: true`，窗口完整在可见区内（不压菜单栏和 Dock），比例 16:9；要 100×50：视频区 318×120（窗口 320×182）且 `clamped`；全屏时 `unavailable`。
2. `setAlwaysOnTop(true)` 后点 Finder 或别的应用：主窗口、浮动侧栏仍在最上；此时新开一个工具窗口（如播放属性）也在最上；`false` 或停用该插件后恢复普通层级；系统文件对话框照常。
3. `setCursorAutoHide("playing")`：窗口模式播放时指针在视频上静止 3 秒消失，移动即出现；暂停不消失；指针在底栏上不消失；菜单打开时不消失；切到别的应用（主窗口不是活动窗口）不消失；停用插件后只在全屏时消失。
4. `dialogs.confirm`：作为主窗口的表单弹出（随窗口移动），回车 = 确认、Esc = 取消；`destructive: true` 时确认按钮为红字；带 `checkbox` 时勾选后 `checked: true`；确认框打开时再调一次 `confirm` 或 `openFile` 得 `busy`；确认框打开期间快捷键不触发。
5. `plugin.app`：与终端 `mpv --version` 第一行、`ffmpeg -version`（同一 Homebrew 库时）一致。
6. 章节与循环：带章节的 mkv `chapters()` 与 `mpv --term-playing-msg='${chapter-list}'` 一致；A-B 循环在 B 点回到 A（误差约一帧）；换文件后不再回跳。

### 6.3 作者交付

（各作者完成时填写）

### 6.4 验收结果

（P3 填写）
