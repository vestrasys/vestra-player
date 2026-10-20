# 第 2 轮：核心第一批（播放、轨道、处理节点、基础动作移交）+ 插件接入

分支：`rebuild/features-r2`（自 `rebuild/features-r1` 合并到 `main` 之后的提交新建，由协调者创建）。依据：[协商记录](2026-10-features-negotiation.md)「第 3 步：裁决」（下称「裁决」）、[功能需求](../../product/feature-requirements.md)（下称「需求文档」）、[社区工作流](../community-workflow.md) §3、§4、§5。轮次总计划见裁决 3.5：2 → 3a → 3b → 4a → 4b。

## 1. 范围

本轮只做裁决里标为「2」的条目，不提前做 3a 之后的条目，也不新增裁决之外的接口。实现中发现裁决的签名做不到时，维护者在本文 §6 记录原因与调整后的签名，再交给作者，不自行扩大。

| 类别 | 条目 |
| --- | --- |
| 基础动作移交（§5 自上而下） | R-H1、R-P3、R-P4、R-P1 + R-P2（`player.step`） |
| 播放信息 | R-P6 + R-P7（`PlaybackStats` 扩展）；宿主行为「阶段变化立即推送」（R-P5 的替代方案） |
| 音轨 | R-T1、R-T2 |
| 字幕 | R-T4（`select` 只有 `"primary"`、`"none"`）、R-T6（含 `streamIndex`）、R-T7（不含 `secondaryPosition`）、R-T8、R-H12 |
| 处理节点 | R-V1–R-V7（八种视频节点）、R-A1、R-A5 |
| 绑定缺陷 | R-H5、R-H4 |
| 通用能力（宿主） | 剪贴板 `plugin.clipboard`、阻止休眠 `plugin.power` |
| 通用能力（runtime） | `plugin.tools`、`plugin.storage`、`plugin.system` |
| 文档契约 | R-U4：底栏不裁切写进参考文档 §6.3 |

不在本轮：次字幕（R-T3）、章节与 A-B 循环（R-M1、R-M2、R-H13）、音频输出设备（R-A2–R-A4、R-H9）、确认对话框、倍帧（R-V8），以及其余 3a 之后的条目。

## 2. 两条线与顺序

| 阶段 | 执行 | 内容 | 完成条件 |
| --- | --- | --- | --- |
| P0 | 协调者 | 建分支；记录基线（§6.1 静态检查、各插件测试数、宿主与 SDK 行数） | 基线写进 §6 |
| P1a | 维护者 | 批次 A（§3.2–§3.6）：不破坏兼容的扩展与通用能力 | 静态检查全过；参考文档、TestHost 同步；协调者审查通过 |
| P1b | 维护者 | 批次 B（§3.1）：基础动作移交，按 §5 带开关落地 | 同上；开关打开时 playback 旧命令仍可用（见 §3.1 切换规则） |
| P2 | 插件作者 | §4 分派。依赖批次 A 的作者在 P1a 审查通过后开工；playback 的移交迁移在 P1b 审查通过后开工 | 各插件 `check/build/test` 全过 |
| P3 | 维护者 | 删除移交开关与只服务于旧路径的代码（§5 P4） | 没有代码引用开关；全量静态检查 |
| P4 | 协调者 | §5 验收，提交推送，合并到 `main` | — |

P1a 与 P1b 可以由维护者顺序做；作者不等 P1b 也能开始做不依赖移交的部分（§3 准入规则）。

## 3. 维护者清单（P1）

每一项都要同时完成：宿主实现（`src/host/`）、SDK 类型与实现（`src/sdk/`）、TestHost 模拟（`src/sdk/plugin/testing.ts`）、参考文档（`doc/plugins/reference.md`）、单元测试；新增能力名同步 manifest 校验（宿主与构建工具两边）。宿主侧第一方 Rust 不得用 `unsafe`。签名以裁决 3.1 / 3.3 为准，下面只写实现要点。

### 3.1 批次 B：基础动作移交（R-H1、R-P3、R-P4、R-P1 + R-P2）

**接口**

```ts
type HostAction =
  | "play-pause" | "stop" | "previous" | "next"
  | "frame-back" | "frame-forward"
  | "seek-backward" | "seek-forward"
  | "rate-down" | "rate-up" | "rate-normal"
  | "volume-up" | "volume-down" | "mute";
plugin.host.run(action: HostAction): Promise<void>;                        // 需 commands；禁用时 unavailable
player.setStepSeek(options: { seconds: number }): Promise<void>;          // 需 playback.control；0.1–3600，一位小数
player.step(unit: "frame" | "keyframe", direction: "backward" | "forward",
            opts?: { ifCurrent?: PlayerState }): Promise<void>;            // 需 playback.control
```

**宿主改动点**

- 核心动作注册：与现有宿主命令（`main.rs` 中 `fullscreen`、`exit-fullscreen`、`quit` 的注册处，约 1440–1510 行）同一机制，`owner = HOST`。菜单路径、order、group、默认绑定、`repeat` 与需求文档「基础播放动作移交核心」一节的表一致；`rate-normal` 对应「播放 ▶ 速度 ▶ 正常」（220，Z）。
- 行为从 `src/plugins/playback/controller.ts` 原样搬到宿主，文字不变：播放/暂停（`ended` 时重播）、相对跳转累加（从最新未完成目标累加、夹在 0 与时长之间、步长取 `setStepSeek`）、倍速 ±0.1（0.01 取整、夹在 0.25–4）、音量 ±5（夹在 0–200、先设音量再取消静音）、上一个/下一个交给播放序列提供者、停止 = 关闭来源。每个动作的 `enabled` / `busy` / `reason` 与 OSD 文案逐条对照现有 playback 测试。
- `frame-back` / `frame-forward` 调 `player.step` 的同一实现；在途时丢弃重复触发（宿主动作内实现，`player.step` 本身只执行一步）。
- `player.step`：`frame` → mpv `frame-step` / `frame-back-step`；`keyframe` → `seek ±0 relative+keyframes`（方向由符号决定，实测确认「严格越过当前关键帧」，不成立时改用 `seek <当前 ± 0.001> absolute+keyframes` 并在 §6 记录）。没有视频轨、生成流、后退时不可跳转：`unsupported`。
- `setStepSeek`：记在调用者名下，调用者停用时恢复 5；多个调用者最后写入生效。
- `host.run`：只接受上表 id，其它字符串 `invalid`；等价于执行该核心命令（同样的 `enabled` 判断、同样的 OSD）。
- 覆盖层迁移：`ui_state.rs` 读入覆盖层时，把下列旧键一次性改写为新键并保存：`vestra.playback/toggle-pause → host/play-pause`、`close-source → stop`、`previous → previous`、`next → next`、`backward-5 → seek-backward`、`forward-5 → seek-forward`、`rate-down`、`rate-up`、`rate-10 → rate-normal`、`volume-up`、`volume-down`、`mute`。新键已存在时保留新键、丢弃旧键。

**切换规则（工作流 §4.2 第 4 条、§5）**

这是破坏性变更：核心动作与 playback 现有 12 条命令占用同样的菜单路径和按键，两者同时注册会 `conflict`。按集中切换做：

1. 维护者实现核心动作时加启动开关 `VESTRA_CORE_ACTIONS=1`（宿主环境变量）。**开关关闭时（默认）宿主不注册这 14 个动作**，现有 playback 完全照旧；`host.run`、`setStepSeek`、`step` 不受开关影响，随时可用。
2. TestHost 增加 `new TestHost({ coreActions: true })`：模拟核心动作表（按 id 记录调用、提供 `host.coreAction(id)` 断言 OSD 与参数），供 playback 迁移测试用。
3. playback 作者在开关打开的条件下迁移（§4 作者 A），迁移后的 playback 在开关关闭时**不能**单独发布，所以 playback 的迁移与 P3 删除开关在同一次提交里落地，由协调者合并。
4. P3：维护者删除开关，核心动作始终注册；删除只服务于旧路径的测试夹具。
5. 迁移例子（写进参考文档的迁移说明）：

   ```ts
   // 之前：playback 自己注册命令，底栏按钮调用控制器
   plugin.commands.add({ id: "toggle-pause", path: ["播放", "播放/暂停"], order: 201, bindings: ["Space"], run: () => controller.togglePause() });
   bar.on("playPause", () => controller.togglePause());
   // 之后：命令由核心注册，底栏按钮执行核心动作
   bar.on("playPause", () => plugin.host.run("play-pause"));
   ```

**测试要求**

- 宿主：14 个动作的启用条件、累加与钳制、OSD 文案的单元测试（从 playback 现有测试逐条移植断言）；覆盖层迁移（旧键改写、新键优先）；`step` 的四种组合与 `unsupported`；`setStepSeek` 撤销。
- SDK：`host.run` 的错误码；`step`、`setStepSeek` 的参数校验与 `ifCurrent`。
- 菜单快照：开关打开后与 `menus-after.json` 对比，除新增项外与移交前一致（沿用 `doc/process/rounds/menus-snapshot.test.ts` 的做法）。

### 3.2 播放信息与阶段推送（R-P6、R-P7、R-P5 替代）

- `backend.rs::stats` 增加裁决 R-P7 列出的字段，全部按 mpv 属性透传，读不到为 `null`：`video-codec`（decoder）、`video-params/pixelformat`、`video-params/colormatrix`、`video-out-params/dw`、`dh`、`estimated-display-fps`、`video-bitrate`；`audio-codec`、`audio-params/format`、`audio-params/hr-channels`、`audio-bitrate`；`avsync × 1000`、`demuxer-cache-duration`。属性名以 mpv 0.41 文档为准，实测为空的在参考文档里注明。
- 阶段立即推送：`backend.rs` 事件循环里，来源阶段（`session.rs::Phase`）变化后立即调用一次 `sources.snapshot` 并广播 `source.state`，周期推送不变。测试：阶段变化后的广播不等待 100 ms 计时（用注入时钟或计数断言）。
- TestHost：`host.stats` 已有，类型随之扩展。

### 3.3 音轨与字幕（R-T1、R-T2、R-T4、R-T6、R-T7、R-T8、R-H12）

- `player.addAudioTrack(path, { select = true, ifCurrent })`：本地路径走现有来源路径校验；mpv `audio-add <path> select|auto`；成功后返回新 `AudioTrack`（等 `track-list` 出现该轨），失败 `backend`（mpv 原因）。外部音轨 `external: true`、`exportable: false`。
- `player.selectAudioTrack(id, { ifCurrent })`：mpv `aid`；id 不在 `audioTracks` 中 `invalid`。
- `subtitles.load` 选项：`format: "srt" | "ass"`（默认 `srt`；`ass` 按内容识别 `[Script Info]` 与 `[Events]`，SSA 视同 ASS）；ASS 有界校验（UTF-8、≤ 2 MiB、Dialogue ≤ 20 000），临时文件扩展名 `.ass`；`title`（1–200 字符，否则 `invalid`）；`select: "primary" | "none"`（默认 `primary`，即现有行为）。改动集中在 `subtitle_upload.rs`、`subtitle_format.rs`。
- `SubtitleTrack.codec`、`text`、`streamIndex`：`subtitle_backend.rs` 读 `track-list` 时已有 codec 与 `ff-index`，开放即可。`text` 的判定表（`subrip`、`ass`、`ssa`、`webvtt`、`mov_text`、`text` 为文本；其余为 `false`）写进参考文档。`streamIndex` 按裁决 3.2 第 4 条：lavf 下的内嵌轨给值，其它 `null`。
- `SubtitleStyle` 新字段：`bold` → `sub-bold`；`shadow` 0–4 → `sub-shadow-offset`（按 1080p 参考高度缩放，与现有描边同法）、`sub-shadow-color` 固定 50% 黑；`box` 0–100 → `sub-border-style=opaque-box` + `sub-back-color` 黑色带透明度，`null` 恢复 `outline-and-shadow`；`assOverride` → `sub-ass-override=scale`（keep）/ `force`。全部可选，省略时与现在一致。改动在 `subtitle_style.rs`。
- R-H12：宿主启动时不再写死 `sub-ass-override=force`，默认 `scale`（keep）。这是默认行为变化，写进参考文档的变更说明，并在 GUI 回归里覆盖「内嵌 ASS + 字幕插件样式」。
- `offsetMs` 校验放宽到 ±600 000。
- TestHost：`host.subtitles` 记录 `load` 的新选项；轨道样例数据带 `codec`、`text`、`streamIndex`；`host.audioTracks` 可设、`addAudioTrack` / `selectAudioTrack` 有默认处理并可用 `host.handle` 注入失败。

### 3.4 处理节点（R-V1–R-V7、R-A1、R-A5）

在 `effects.rs` 增加节点，每个节点同时进实时图与离线图（`render_lavfi`），给出范围校验与 runtime 标记；runtime 标记只给实测确认能用 `vf-command` / `af-command` 改的参数。

| 节点 | FFmpeg | 范围 | 运行时参数 |
| --- | --- | --- | --- |
| `hue { degrees }` | `hue=h=` | −180–180 | `h`（待实测） |
| `sharpen { amount; size }` | `unsharp`（`small` 3×3、`large` 7×7） | amount 0–2 | 无 |
| `denoise { lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` | `hqdn3d` | 0–20 | 无 |
| `deblock { strength }` | `deblock=filter=weak\|strong` | — | 无 |
| `deband { threshold; range }` | `deband=1thr=…:r=…` | threshold 0.003–0.5，range 1–64 | 无 |
| `transform { rotate; flipH; flipV }` | `transpose` / `hflip` / `vflip` 组合 | 枚举 | 无 |
| `deinterlace { when; output }` | `bwdif=mode=send_frame\|send_field:deint=interlaced\|all` | 枚举 | 无 |
| `reframe { mode; aspect; x; y }` | `crop` / `pad`，用 `iw`、`ih` 表达式 | aspect 0.25–4，x、y −1–1 | 无 |
| 音频 `channels { mode }` | `pan`（左、右、单声道、互换）；多于 2 声道时前置 `aresample=ochl=stereo` 下混 | 枚举 | 无（模式切换走重配置，验证无明显断音） |

- `transform` 作用于宿主方向处理（`source_geometry.rs`）之后的画面；写测试覆盖 90° 与容器旋转叠加时的输出尺寸。
- 音频 `delayMs` 放宽到 ±10 000（10 ms 倍数），`processing.rs` 的正负预算同步放宽，补齐边界测试。
- 64 节点总上限不变；video 插件最坏链长写进参考文档。
- SDK：`VideoNode` / `AudioNode` 联合类型与校验；TestHost 的 `host.processing` 能断言新节点。

### 3.5 绑定缺陷（R-H5、R-H4）

- `bindings.rs::from_slint_key` 之外，增加按 winit `PhysicalKey` 识别的路径（`main.rs` 已在 `KeyboardInput` 中拿到 `physical_key`）：带 Option 时以物理键名匹配 `Alt+<键>`，不再依赖 Option 打出的字符；死键（⌥E、⌥N、⌥U、⌥I、⌥`）同样能触发。
- 小键盘：`Numpad0`–`Numpad9`、`NumpadPlus`、`NumpadMinus`、`NumpadMultiply`、`NumpadDivide`、`NumpadEnter`、`NumpadDecimal` 为独立输入名；主键盘数字不匹配小键盘，反之亦然。语法校验（宿主与 SDK 两处）同步。
- 核实并修复 `Ctrl+[` 以 `\u{1b}` 到达被识别为 `Ctrl+Escape` 的问题。
- 测试：Option + 字母、Option + 符号、死键、小键盘与主键盘区分、`Ctrl+[`。验收需要实机（协调者 GUI 测试）。

### 3.6 通用能力

**宿主：剪贴板**（能力 `clipboard`）

```ts
plugin.clipboard.readText(): Promise<string | null>;   // 无文本为 null
plugin.clipboard.writeText(text: string): Promise<void>; // ≤ 1 MiB
plugin.clipboard.writeImage(path: string): Promise<void>; // PNG/JPEG，≤ 16 MB，绝对路径
```

- 用 `objc2-app-kit` 的 `NSPasteboard`（增加该 feature；`generalPasteboard`、`clearContents`、`setString_forType`、`setData_forType`、`stringForType` 都是安全方法）。类型用 UTI 字符串 `public.utf8-plain-text`、`public.png`、`public.jpeg` 构造，不读 extern static。
- 失败码：`invalid`（类型、大小、非绝对路径）、`unavailable`（无界面模式）、`failed`（系统拒绝写入）。
- TestHost：`host.clipboard: { text: string | null; images: string[] }`，可预置 `text` 测 `readText`。

**宿主：阻止休眠**（需 `playback.control`）

```ts
plugin.power.preventSleep(mode: "off" | "playing"): Promise<void>;
```

- 任一插件要求 `playing` 即生效；插件停用时撤销它的要求。
- 宿主在「来源阶段 ready 且未暂停」（含生成流）时持有断言；有视频帧时启动 `/usr/bin/caffeinate -d -i -w <宿主 pid>`，纯音频时 `-i -w <宿主 pid>`；条件变化时结束旧进程再起新进程；宿主退出时随之结束。
- 测试：状态机（要求者集合 × 播放状态 × 有无视频）产生的期望进程参数；进程启动用可注入的启动器，单测不真起 `caffeinate`。
- TestHost：`host.power: "off" | "playing"`（合并后的有效值）。

**runtime（SDK 层）：外部工具、设置文件、文件夹**

在 `src/sdk/plugin/` 实现，经 `plugin` 对象提供，不加宿主协议。签名见裁决 3.3。

- `plugin.tools.run(tool, args, { timeoutMs, maxOutput, signal })`：`spawn` 不经 shell；可执行文件按 `VESTRA_FFMPEG` / `VESTRA_FFPROBE` / PATH 查找（查找结果缓存）；超时、`signal`、作用域释放都结束进程（先 SIGTERM，300 ms 后 SIGKILL）；`maxOutput` 默认 16 MiB，超出 `invalid`；`plugin.tools.available(tool)`。
- `plugin.storage.json(name, { defaults, validate, debounceMs })`：`name` 只能是 `[a-z0-9-]{1,64}`；读坏或校验失败时改名为 `<name>.json.bad-<时间戳>`；`save` 去抖（默认 300 ms）后临时文件 + 改名；`flush()`；`dispose` 与插件停用时先 flush。
- `plugin.system.reveal(path)` → `/usr/bin/open -R <path>`；`openPath(path)` → `/usr/bin/open <path>`；不存在 `missing`；超时 10 秒。
- TestHost：`host.tools.handle(tool, (args) => ({ stdout, stderr }) | HostError)` 注入工具结果（默认 `unavailable`），`host.tools.calls` 记录参数；`storage` 直接用 `host.dataDirectory` 的真实文件；`host.system.revealed`、`host.system.opened` 记录路径。

### 3.7 文档

- `doc/plugins/reference.md`：上述全部接口；§6.3 增加「底栏面板的内容可以画到槽位上方的视频区（负 `y`），宿主不裁切；侧栏与浮动窗口在其上层」，并在 `composition.rs` 的组合测试里加断言（底栏外层不带 `clip`）；§4.1 绑定语法增加小键盘名与 Option 说明；变更说明列出默认行为变化（`sub-ass-override`、基础动作归属）。
- `doc/plugins/examples.md`：`host.run`、`tools.run`、`storage.json` 各一个短例（每个不超过 30 行，§5 P2 标准）。
- 迁移说明：基础动作移交的旧 id → 新 id 表与 §3.1 的迁移例子。

## 4. 插件接入分派（P2）

作者规则同第 1 轮（[2026-10-features-r1.md](2026-10-features-r1.md) §2）：只写自己的插件目录；按需求文档对应小节实现；某项发现仍需核心改动时跳过并按 §4.1 写请求。另加两条：

- **迁移到通用能力**：本轮采纳的通用能力（剪贴板、阻止休眠、`tools`、`storage`、`system`）上线后，插件里对应的自制实现**必须**删除并改用新能力，报告列出删除的文件与行数。`sips`、钥匙串、`fs.watch`、系统信息命令不在此列，保留。
- **降级项**：裁决里被拒绝或收窄的功能，按裁决的替代方案实现，同时修改需求文档对应小节（协调者合并时审）。

### 4.1 分派表

| 作者 | 插件 | 新功能（编号 → 依赖） | 迁移到通用能力 | 降级与文档 |
| --- | --- | --- | --- | --- |
| A | playback | B07 上一帧/下一帧、B08 关键帧（`player.step`）；B06 第 1 档步长（`setStepSeek`）；基础动作移交迁移（P1b 之后）：删除 12 条命令（`toggle-pause`、`close-source`、`previous`、`next`、`backward-5`、`forward-5`、`rate-down`、`rate-up`、`rate-10`、`volume-up`、`volume-down`、`mute`），底栏按钮改用 `host.run`，`controller.ts` 中被核心接管的累加、钳制逻辑删除；第 2–4 档跳转改 id 为 `backward-2…4` / `forward-2…4` | `system.ts` 的 `pbcopy`/`pbpaste` → `clipboard.writeText` / `readText`（I08 复制、A02 预填链接）；`sleep.ts` 的 `caffeinate` → `power.preventSleep`（B29，删除 `sleep.ts`）；`settings.ts` → `storage.json` | B07/B08 的 `enabled` 用 ready 后查一次 `stats()` 的缓存（裁决 R-P1）；I07 仍待 3b（`ui.image`）；I06、I07 的悬停提示按 R-U4 用负 `y` 画在视频区 |
| B | playlist | B26 改为「快速推进到下一个」：提前算下一项与 `fs.stat`，`onEnd` 里只 `open` 一次（受益于阶段立即推送） | `finder.ts` → `system.reveal`（C23）；`probe.ts` 的 ffprobe 子进程 → `tools.run("ffprobe")`；`store.ts` → `storage.json`（保留现有 `.bad` 语义） | B26 菜单名与验收改为「同格式切换 < 150 ms」（需求文档）；`confirm.ts` 本轮保留，3a 换 `dialogs.confirm` |
| C | subtitles、subtitle-online（新） | subtitles：D17 外部 ASS 与「原样式」开关（`load({ format: "ass" })`、`assOverride`）；D18 多格式（ASS 原样上传；VTT、SAMI、MicroDVD 插件内转 SRT；`title`）；D09 粗体、D11 阴影与背景框（`bold`、`shadow`、`box`）；D05 范围 ±600 000；B11 上一句/下一句（本插件载入的外部轨直接解析；内嵌文本轨用 `tools.run("ffmpeg", ["-map", "0:" + streamIndex, …])`，超时按文件大小约 1 s/100 MB、下限 15 s；`streamIndex` 为 `null` 或网络来源时禁用）。subtitle-online：D16 全部（`load` 的 `format`、`title`、`select`；网络用 `fetch`；凭据进钥匙串） | subtitles 的设置读写 → `storage.json`；subtitle-online 新写的设置直接用 `storage.json`、子进程用 `tools.run` | A08 次字幕整体推到 3a；D18 菜单里的 VTT 在插件内转换（两个插件各带一份转换代码） |
| D | video、audio | video：E02 色调（`hue`）、E04 锐化/边缘锐化（`sharpen` small/large）、E05 三种降噪（`denoise` 预设）、E06 去块/消色带、E08 翻转与 E09 旋转（`transform`）、E10 反交错（`deinterlace`）、E12 拉伸/裁剪（`reframe`）；窗口四页签中「色彩」「效果」「画面」页的相应控件（E11、E13、E16–E18 的控件先置灰并写原因）。audio：A11 载入外部音频（`addAudioTrack`，含同名自动载入）、G03 选择音轨与 G04 依次切换（`selectAudioTrack`）、G05 声道（`channels`）、G09 范围放宽到 ±10 000 | 两个插件的设置读写 → `storage.json` | E10 删除算法单选（固定 BWDIF）；E13 删除帧宽/帧高四条命令；E18 倍帧推到 4a；E17 删除 `S` 键说明；G16 推到 3a |
| E | capture、info、history | capture：无新功能。info：I05 播放属性（`stats()` 扩展字段；文件信息用 `tools.run("ffprobe")`；系统信息用 `sw_vers`、`sysctl`、`system_profiler`；复制用 `clipboard.writeText`；mpv、FFmpeg 版本一行显示「未知」直到 3a 的 `plugin.app`）。history：无新功能 | capture：`system.ts` 的 `osascript` 写图片 → `clipboard.writeImage`（F03）、`open` → `system.openPath`（F09）、`settings.ts` → `storage.json`；`sips` 保留（F07）。history：设置与记录读写 → `storage.json` | info I05「硬件解码」行固定「未使用」；I09 推到 3a |

不在本轮的插件：window、marks、hotkeys、preferences、plugins、visualizer（3a 起）。youtube、bilibili、nes、workshop 没有本轮条目；它们若有自制的设置文件读写，可在本轮顺带迁移到 `storage.json`（不强制，报告说明）。

### 4.2 准入

- 作者 B、C、D、E：P1a 审查通过即开工。
- 作者 A：B07、B08、B06 第 1 档与通用能力迁移在 P1a 之后开工；基础动作移交迁移在 P1b 之后开工，与维护者 P3 同批合并（§3.1 切换规则第 3 条）。
- 同时运行的作者不超过 3 个（含协调者共 4 个）：先 A、C、D，B、E 随后。

## 5. 验收

### 5.1 静态检查与测试

工作流 §6.1 全部命令，另加：菜单快照测试（移交前后对比）、各插件迁移前后的测试数（不得减少）。

### 5.2 真实 GUI 测试（协调者）

工作流 §6.2 的全部必测场景，加上本轮：

| 范围 | 场景 | 证据 |
| --- | --- | --- |
| 基础动作 | 停用 playback 后 Space、←/→、D/F、↑/↓、M、PageUp/PageDown、X/C/Z、F4 仍可用且 OSD 文字不变；重新启用后底栏按钮与快捷键效果一致 | 截图 + OSD 文字 |
| 步长 | 第 1 档设 10 秒后 ← 后退 10 秒；停用 playback 后恢复 5 秒 | OSD |
| 逐帧 | 播放中按 F 暂停并前进一帧；连按 D 五次回退约 5/fps；按住 F 松开后不再前进；纯音频时菜单灰 | 截图、时间 |
| 关键帧 | H.264 片源连按 Ctrl+Shift+→ 十次，时间单调增加且与 `ffprobe -skip_frame nokey` 列出的关键帧一致 | 时间记录 |
| Option 键 | Alt+O、Alt+H、Alt+B、Alt+K、Alt+A、Ctrl+Alt+Y 实机触发；死键组合触发；小键盘 5 与主键盘 5 区分 | 物理按键操作记录 |
| 字幕 | 带 `\pos`、`\fad` 的 ASS 按原样式显示，关闭开关后变为插件样式；粗体、阴影、背景框可见；GB18030 SRT、VTT 载入正确；上一句/下一句在内嵌字幕上可用 | 截图 |
| 画面节点 | 八种节点分别生效；「含效果」截图包含这些效果；拖动色调滑块播放不中断 | 截图、导出产物 |
| 音频 | 载入外部 mp3 并被选中；切换音轨；左/右/单声道/互换用左右不同的测试音核对；延迟 −5000 生效 | 听感记录 + 参数 |
| 通用能力 | 复制文件名后在文本编辑器粘贴中文正确；截图复制到剪贴板后在预览中粘贴；播放视频时 `pmset -g assertions` 有 Vestra 的显示器断言、暂停后消失；在 Finder 中显示定位正确 | 粘贴结果、`pmset` 输出 |
| 快速推进 | 同格式两首 flac 切换间隙 < 150 ms（宿主日志时间戳） | 日志 |
| 底栏不裁切 | 进度条悬停时间提示画在视频区上方，全屏与窗口模式都可见 | 截图 |
| 停用 | 停用 video 只撤销它的节点；停用 audio 撤销声道节点但外部音轨保留到换来源 | 截图 |

缺陷归属同工作流 §6.2。全部通过后协调者提交推送并合并到 `main`。

## 6. 记录

### 6.1 基线

（P0 填写）

### 6.2 维护者交付与签名调整

#### P1a 批次 A（2026-10-20，维护者）

**实现清单**（每项：宿主 + SDK 类型与封装 + TestHost + 参考文档 + 单元测试）

| 任务单 | 内容 | 宿主 | SDK / TestHost |
| --- | --- | --- | --- |
| §3.2 | `PlaybackStats` 扩展（R-P6、R-P7） | `backend.rs::stats` 透传 mpv 属性 | 线上新字段可选，`player.stats()` 补 `null`；`host.stats` 旧写法照常 |
| §3.2 | 阶段变化立即推送 | `backend.rs::StatePush`：阶段或来源变化当轮发 `source.state`，周期 100 ms 不变；单测不依赖计时 | 无契约变化 |
| §3.3 | `addAudioTrack`、`selectAudioTrack`（R-T1、R-T2） | `source_backend.rs::audio_track`，线上方法 `audio.addTrack` / `audio.selectTrack`（`playback.control`） | 默认以 `player.state` 为条件；TestHost 默认处理 |
| §3.3 | `subtitles.load` 的 `format`、`title`、`select`（R-T4） | `subtitle_format::validate_ass`（UTF-8、≤ 2 MiB、两节、1–20 000 行 Dialogue）；ASS 原样写 `.ass` 临时文件；`select: "none"` 只登记不选中；`title` 进 `sub-add` | 默认值不上线；`host.subtitleLoads` |
| §3.3 | `SubtitleTrack.codec`、`text`、`streamIndex`（R-T6） | `streamIndex` 仅内嵌轨且 `current-demuxer == lavf` 且 `path` 为 `fd://`（宿主租约路径，即 media_tracks 的 lavf 条件） | 线上字段可选 |
| §3.3 | `SubtitleStyle.bold/shadow/box/assOverride`、`offsetMs` ±600 000（R-T7、R-T8） | `subtitle_style.rs`；离线截图 `force_style` 同步 Bold/Shadow/BorderStyle | 类型 |
| §3.3 | R-H12 | 启动默认 `sub-ass-override=scale`（keep） | 变更说明 |
| §3.4 | 8 种视频节点 + `channels`、`delayMs` ±10 000（R-V1–R-V7、R-A1、R-A5） | `effects.rs` 节点可展开为多个滤镜（标签 `name@nK`、`name@nKxJ`）；只有 `hue.h` 为运行时参数；`processing.rs` / `offline_audio.rs` 预算放宽 | `nodeProblem` 校验，TestHost `processing.replace` 用同一规则 |
| §3.5 | R-H5、R-H4、Ctrl 控制码 | `bindings::from_key` + winit 钩子记录物理键（`observe_key`）；死键在钩子里直接解析 | `NAMED` 增加小键盘名 |
| §3.6 | 剪贴板 | `clipboard.rs`（`objc2-app-kit` `NSPasteboard` 安全方法，新增 `objc2-foundation` 依赖，已在 lock 中）；能力 `clipboard`（宿主 `plugins.rs` 与构建工具两处） | `host.clipboard` |
| §3.6 | 阻止休眠 | `power.rs`（可注入启动器的状态机）；`/usr/bin/caffeinate [-d] -i -w <pid>`；「有视频」= 1 秒内收到过视频帧 | `host.power` |
| §3.6 | `tools`、`storage`、`system`（runtime） | 无 | `sdk/plugin/local.ts`（真实子进程）、`services.ts`；TestHost 通过 `Transport.local` 注入假实现 |
| §3.7 | 文档 | `composition.rs` 测试断言底栏外层无 `clip` | 参考文档 §2、§3、§4.1、§5、§6.3、§8、§9、§13、§14、§16（新）、§17 变更说明；示例 §18–§20 |

协调者追加的缺陷（Ctrl+Y 被识别为 Shift+Tab）已并入 §3.5：有 Ctrl/⌘ 时控制码一律按 Ctrl+字母（0x1b 为 `Ctrl+[`），无修饰时才映射 Tab/Backspace/Enter/Escape/Shift+Tab；有物理键时按物理键（Ctrl+Tab 仍是 Tab，单按修饰键不算输入）。单测 `control_codes_are_ctrl_letters_only_with_ctrl` 覆盖五个码有/无修饰。

**签名调整与实现取舍**（签名均与裁决一致，以下是行为细节）

1. `box`：任务单写「`sub-border-style=opaque-box` + `sub-back-color`」。mpv 0.41 文档中 opaque-box 的框颜色取 `sub-outline-color`、且逐行包裹，`sub-back-color` 只给阴影框上色；改用 `background-box`（颜色取 `sub-back-color`、边距取 `sub-shadow-offset`，libass 扩展 BorderStyle=4）。因此有框时 `shadow` 不生效（两者共用 `sub-shadow-offset`），已写进参考文档 §9。
2. `channels` 的下混用 `aformat=channel_layouts=stereo`（滤镜图自动插入重采样）代替 `aresample=ochl=stereo`，效果相同、不依赖 swr 选项名；`"stereo"` 渲染为 `anull`（不改声道）。
3. `denoise` 某项为 0 时发送 0.001：hqdn3d 把 0 当作「按其它项推导默认值」。
4. `deband` 四个平面用同一阈值（任务单只写了 `1thr`）。
5. `reframe` 的宽高比按显示宽高比（像素宽 × sar），尺寸取偶数。
6. `PlaybackStats.video.decoder` 取 `current-tracks/video/decoder`（短名 `h264`）；原有 `codec` 已是 `video-codec` 的描述串。实测 `estimated-display-fps` 在 libmpv 软件渲染下为 `null`，`video-bitrate` 开播约 1 秒内为 `null`，已在参考文档注明。
7. TS 线上类型（`sdk/wire.ts` 的 `PlaybackStats`、`sdk/subtitles.ts` 的轨道）把新字段设为可选：插件测试直接构造这些线上对象（info、subtitles 的测试），设为必选会让现有插件 `check` 失败；公开类型（`types.ts`）字段是完整的。
8. 线上方法名：`audio.addTrack`、`audio.selectTrack`、`clipboard.readText|writeText|writeImage`、`power.preventSleep`。
9. 构建工具原本不校验能力名；按「两边同步」加了一张与 `plugins.rs` 相同的表（`plugin -- check/build/test` 遇到未知能力失败）。

**已知缺口**

- 死键组合（⌥E、⌥N、⌥U、⌥I、⌥\`）只在主窗口视频区获得焦点时触发（winit 钩子里拿不到 Slint 焦点控件；输入框里需要正常组字）；浮动侧栏窗口里不触发死键组合。非死键的 Option 组合在任何焦点下都按物理键解析。
- `Alt+Shift+数字/符号` 按美式键位换算出符号名；字母取当前布局（`key_without_modifiers`）。
- ASS 外部轨不能进入「含字幕」截图（`unsupported`），与内嵌 ASS 现状一致。
- 外部音轨在换来源前保留；停用插件不撤销（裁决如此）。
- 阻止休眠的「有视频」判断基于 1 秒内是否收到渲染帧：封面图类音频只渲染一帧，按纯音频处理。
- 插件载入的 SRT 不传 `title` 时，轨道标题仍是临时文件名（原有行为）。
- workshop 的 `vendor/sdk` 镜像测试因 SDK 新增 `local.ts`、`services.ts` 及改动失败（1/17），按该测试文件开头的命令同步后通过；不在维护者可写范围，请协调者同步（前几轮同样处理）。
- capture 的「one running and four queued」与 subtitles 的「host refusal of the upload」在机器负载高时各偶发失败一次，单独重跑通过（与本批改动无关的计时敏感用例）。

**检查**（worktree 内实际执行）

- `src/`：`npm run check` 0；`format:check` 0；`boundaries` 0；`test:sdk` 66/66（新增 `tests/sdk/round2.test.ts` 12 条）；`plugins:build` 0；插件测试 audio 15、bilibili 15、capture 13、info 4、nes 11、playback 21、playlist 35、subtitles 19、video 10、youtube 14 全过；workshop 16/17（上条 vendor 镜像）。
- `src/host/`：`cargo fmt --check` 0；`cargo clippy --locked --all-targets -- -D warnings` 0；`cargo test --locked` 176/176；`cargo check --no-default-features` 0。
- 无界面实测（`--headless` + 临时探针插件，未进仓库）：`stats` 新字段有值；`addAudioTrack`（外部 mp3）返回 `{ id: 2, external, exportable: false }` 并选中，`selectAudioTrack(1)` 切回；ASS `load({ format: "ass", title, select: "none" })` 成功且不改选择；八种视频节点同时应用 `applied`，transform 90° + pad 1:1 后 `displayWidth/Height` 为 640×640；`hue` 连续改值走 `vf-command`；`channels swap` + `delayMs −5000` `applied`；无界面剪贴板 `unavailable`。另用 mpv 0.41 命令行逐个验证节点滤镜图被接受、`vf-command … h … hue@n0` 成功而 `unsharp` 的 `la` 不能运行时修改。
- 宿主 Rust 净增：`src/host/src/*.rs` 22 770 → 24 785 行（+2 015，含测试）；SDK 与运行器 `src/sdk/plugin/*.ts` + `src/runtime/` 4 812 → 5 565 行（+753），另 `src/sdk/*.ts` 线上类型与节点校验 +192。

**给协调者的 GUI 实测要点**

1. 阻止休眠：临时插件或后续 playback 迁移调 `power.preventSleep("playing")` 后播放视频，终端 `pmset -g assertions | grep -i caffeinate` 应看到 `PreventUserIdleDisplaySleep` 与 `PreventUserIdleSystemSleep`（进程 caffeinate，`-w` 为播放器 pid）；暂停后约 16 ms 内断言消失；播放纯音频（mp3/flac）只有 `PreventUserIdleSystemSleep`；退出播放器后 `pgrep caffeinate` 无残留。
2. 剪贴板：`clipboard.writeText("中文文件名.mkv")` 后在「文本编辑」⌘V 粘贴正确；`writeImage(<截图 png>)` 后在「预览 ▶ 文件 ▶ 从剪贴板新建」得到同一图片；在别的应用复制一段文字后 `readText()` 返回它；复制图片后 `readText()` 为 `null`。
3. Option 键：视频区获得焦点时按 ⌥O、⌥H、⌥B、⌥K、⌥A、⌃⌥Y 分别触发声明为 `Alt+O` 等的命令（subtitles 的 `Alt+H` 现成可测）；⌥E、⌥N、⌥U、⌥I、⌥\` 死键触发 `Alt+E` 等；在 playlist 搜索框等输入框里按 ⌥E 再按 e 仍打出 é，不触发命令。
4. Ctrl 控制码：⌃Y（playback「重开当前/最后文件」）、playlist 列表焦点下 ⌃I（添加文件）按键触发；Tab、Esc、回车、退格单按仍是原功能；⌃[ 触发声明为 `Ctrl+[` 的命令。
5. 小键盘：给一个命令声明 `Numpad5`、另一个声明 `5`，分别按小键盘 5 与主键盘 5 各自只触发自己的命令；小键盘回车不再触发「全屏」（`Enter`），菜单快捷键文字显示 `Num5`。
6. ASS 默认保留样式（R-H12）：打开带内嵌 ASS（含 `\pos`、`\fad`、彩色样式）的 mkv，字幕按脚本位置与颜色显示；subtitles 插件设置样式后 ASS 仍保留原样式，SRT 轨按插件样式显示；传 `assOverride: "force"` 后 ASS 改为插件样式。
7. 样式新字段：`bold: true`、`shadow: 3` 可见粗体与阴影；`box: 60` 出现半透明黑色背景框（阴影消失）；`box: null` 恢复描边加阴影。
8. 底栏不裁切：底栏组件里放一个负 `y` 的元素，窗口与全屏模式都画在视频区上方，侧栏展开时被侧栏盖住。

#### P1a 修复：Option / Ctrl 组合在实机不触发（协调者 GUI 实测反馈）

现象：System Events 发送的 ⌥O、⌥H、⌃Y 不触发（单测通过）。

原因（对照 Slint 1.16.1 与 winit 0.30.13 源码自查）：

- 钩子挂载：`bind_shell` 在主界面每次重建（插件面板挂载）后对新实例重新调用，钩子在；浮动侧栏另由 `observe_keys` 挂。不是原因。
- 先后顺序：winit 的 `on_winit_window_event` 过滤器在同一次 `KeyboardInput` 分派中先于 Slint 的 `process_key_input` 运行，`key-input` 回调同步发生在其后；`physical.take()` 只在 `key-input` 里取，松键和合成事件不覆盖。假设成立。
- **根因：修饰键状态的来源。** Slint 不处理 winit 的 `ModifiersChanged`，它只根据「修饰键本身的按下/松开事件」维护 `event.modifiers`。System Events 的 `keystroke … using option/control down` 把修饰标志放在按键事件上，不发修饰键自身的按下事件（实体键盘会发），于是 Slint 报 `alt = false` / `meta = false`：⌥O 成了打出的 `ø`，⌃Y 的 `\u{19}` 在无 Ctrl 时按规则是 Shift+Tab。winit 在 `keyDown` 里先按事件自带标志发 `ModifiersChanged`，再发 `KeyboardInput`，所以 winit 的修饰状态是对的。

修复：钩子把按键时 winit 的修饰状态记进 `bindings::Physical.mods`，`from_key` 有它时以它为准、不用 Slint 的标志；单测 `window_system_modifiers_win_over_missing_slint_flags`（Slint 标志全为假、winit 报 Alt / Ctrl）。

诊断开关：`VESTRA_DEBUG_KEYS=1` 时宿主向 stderr 打印每个 winit 键盘事件（`keys: winit Pressed|Released synthetic=… physical=… logical=… text=… repeat=… modifiers=…`）、`ModifiersChanged`（`keys: winit modifiers …`）、死键解析结果，以及每次 Slint `key-input`（`keys: slint scope=… text=… control/alt/shift/meta=… physical=… → 解析结果 bound=是否有绑定`）。实测若仍不触发，按这几行判断：没有 `keys: winit` 行 = 钩子没收到；有 winit 行没有 slint 行 = 事件被焦点控件或 IME 吃掉；slint 行解析正确但 `bound=false` = 绑定表里没有该输入（检查插件声明或用户覆盖层）。

#### 裁决：`power.preventSleep` 断言启动失败的语义（作者 A 报告）

问题：`preventSleep` 只登记要求，`caffeinate` 在播放条件满足时才由宿主启动，启动失败只写宿主日志，插件收不到，需求 B29 第 5 条「无法阻止屏保」提示无从实现。

结论（§4.3 保守原则）：**不加通知事件，写明失败语义**。参考文档 §16.2 增加：启动失败只写宿主日志（`host.error power …`），不通知插件，同条件不重试、条件变化再试。理由：`/usr/bin/caffeinate` 是系统自带程序，启动失败在受支持的 macOS 上实际不会发生，不满足「需要宿主内部状态的通用能力缺口」的门槛；为一个几乎不出现的失败增加一条事件通道（订阅、退订、作用域、TestHost 模拟）不值得。作者 A 据此把 B29 第 5 条降级（删除「无法阻止屏保」提示），并在需求文档对应小节注明。若以后出现可复现的失败场景，再按 §4.1 带证据提出。

#### 裁决：`system.reveal` 接受多个路径（作者 B，playlist C23）

请求：`system.reveal(path | path[])`，一次 `/usr/bin/open -R a b c`；全部不存在 `missing`，部分不存在跳过。证据：同一文件夹选中 3 个文件按 Ctrl+F，单路径接口只能高亮 1 个（逐个调用会开多个 Finder 窗口或只留最后一个），原 `open -R a b c` 能全部高亮——用户可见的功能缺失，替代方案做不到。

结论：**接受**（兼容扩展，只在 runtime 层，不加宿主协议）。签名 `reveal(paths: string | string[]): Promise<void>`；最多 256 个；路径先转绝对路径（避免以 `-` 开头被当成 `open` 的选项）并去重；不存在的跳过，全部不存在 `missing`。`openPath` 不变。TestHost 的 `host.system.revealed` 按顺序记录每个被显示的路径（原有断言不受影响）。随批次 B 提交实现，参考文档 §16.5 已更新。

#### P1b 批次 B（维护者）

**实现清单**

| 项 | 宿主 | SDK / TestHost |
| --- | --- | --- |
| 14 个核心动作（R-H1） | `core_actions.rs`：声明（路径、order、group、绑定、重复与 playback 现状一致；新增 `frame-back/forward` 204/205 group `frame`，第 1 档跳转 group `step`，`rate-normal` 单选）、启用/勾选/原因、执行计划与 OSD 文案（从 `controller.ts` 逐条搬来）；`backend.rs::CoreActions` 在控制线程按顺序执行（宿主自己的操作簿 owner 0）；`main.rs` 注册、状态随 `source.state` 与序列标志更新、`ActionDone` 显示 OSD | `host.run` |
| 开关 `VESTRA_CORE_ACTIONS=1` | 关闭（默认）时不注册 14 条命令；`host.run`、`player.step`、`setStepSeek` 始终可用 | `new TestHost({ coreActions: true })`、`host.coreAction(id)`、`host.coreRuns` |
| `player.step`（R-P1、R-P2） | 线上方法 `player.step`（`playback.control`）；`frame` → `frame-step` / `frame-back-step`，`keyframe` → `seek ±0.001 relative+keyframes`；无活动视频（`source.state.hasVideo`，新增内部字段：选中的非封面视频轨）、生成流、后退时不可跳转 → `unsupported` | `host.steps` |
| `player.setStepSeek`（R-P3） | 按插件记录、最后写入生效，插件停用后回退；值变化时重新注册两条跳转命令（标题「后退 N秒」） | `host.stepSeek` |
| `host.run`（R-P4） | 线上方法 `host.run`（`commands`）；未知 id `invalid`；禁用 `unavailable`（原因作 message）；执行完才答复；同一独占动作在途时丢弃重复并直接答复 | 执行后刷新 `player.state` |
| 覆盖层迁移 | 开关打开时启动即把 12 个 `vestra.playback/<旧 id>` 改写为 `host/<新 id>` 并保存；新键已存在则保留新键 | — |
| 文档 | — | 参考 §5.2（`step`、`setStepSeek`）、§5.5（新，核心动作表）、§4.1、§17 批次 B 变更说明（旧→新 id 表、迁移例子、开关）；示例 §21 |

**签名调整与实现取舍**（签名与裁决一致）

1. 关键帧：任务单写「`seek ±0 relative+keyframes`，实测不成立时改用 `seek <当前 ± 0.001> absolute+keyframes`」。mpv 0.41 实测（H.264 片源，关键帧 8.333/16.667/24.021/32.355 …）：`seek 0 relative+keyframes` 前进时反而回到更早的关键帧；`absolute+keyframes`（当前 ± 0.001）同样不对；**`seek ±0.001 relative+keyframes` 两个方向都严格越过当前关键帧**（21.5 → 24.021 → 32.355 …；56.376 → 48.043 → 40.688 …），采用它。
2. `frame-back/forward` 在核心动作里不显示 OSD（playback 现有命令里没有对应项，需求未定文案）。
3. 相对跳转的「最新未完成目标」：上一次核心跳转在同一来源、且 mpv `seeking` 为真或距今 < 300 ms 时，以它的目标为基准累加，否则以当前位置为基准。
4. 核心动作执行失败（来源拒绝等）把 message 显示在视频上（与底栏一致）；`host.run` 的失败返回给调用的插件，不显示。
5. 音量 ±5 以控制线程上当前 mpv 音量为基准（请求在同一线程顺序执行，等价于 playback 的草稿值规则）。

**检查**

- `src/`：`check`、`format:check`、`boundaries`、`plugins:build` 0；`test:sdk` 69/69（新增 `tests/sdk/core-actions.test.ts` 3 条）；插件测试 audio 22、bilibili 15、capture 22、info 9、nes 11、playback 37、playlist 56、subtitles 30、video 10、youtube 14 全过；workshop 16/17（SDK 又有变化，`vendor/sdk` 需按测试开头的命令重新同步，协调者处理）。
- `src/host/`：`cargo fmt --check`、`clippy --all-targets -D warnings` 0；`cargo test` 183/183（`core_actions.rs` 6 条：声明与 playback 菜单一致、启用条件与原因、计划与 OSD 文案及钳制/累加、`step` 四种组合与 `unsupported`、步长校验与格式、覆盖层迁移）；`--no-default-features` 编译通过。
- 无界面实测（`VESTRA_CORE_ACTIONS=1 --headless` + 临时探针插件）：`play-pause` 暂停；连续两次 `seek-forward` 0.5 → 10.5（累加）；`setStepSeek(10)` 后 `seek-backward` 10.5 → 0.5；`frame-forward` / `step("frame")` 每次 +1/30 s，`step("frame","backward")` −1/30 s；`step("keyframe","forward")` 两次 0.53 → 8.333 → 16.667，`backward` → 8.333；`rate-up` 1.1、`rate-normal` 1、`volume-up` 105、`mute` 开；没有序列提供者时 `next` → `unavailable 没有下一个`；`stop` 后 `play-pause` → `unavailable`、`step` → `unsupported`。
- 宿主 Rust：`src/host/src/*.rs` 24 785 → 26 032（批次 B 及修复 +1 247，第 2 轮累计 +3 262，超过约 3 000 的预算约 9%，主要是测试；P3 删除开关会减少少量代码）；SDK 与运行器 5 565 → 5 977。

**给协调者的 GUI 实测要点**（以 `VESTRA_CORE_ACTIONS=1` 启动，不加载 playback，或加载迁移后的 playback）

1. 菜单：「关闭」(F4)、播放 ▶ 播放/暂停 (Space)、上一个/下一个、跳转 ▶ 上一帧 (D) / 下一帧 (F) / 后退5秒 (←) / 前进5秒 (→)、速度 ▶ 正常 (Z) / 减速 0.1 (X) / 加速 0.1 (C)、声音 ▶ 音量 + (↑) / 音量 − (↓) / 静音 (M) 都在，位置与移交前一致。
2. 不加载 playback：Space、←/→（按住连续）、D/F、↑/↓、M、PageUp/PageDown（有 playlist 时）、X/C/Z、F4 可用，OSD 文字与 playback 时一致（「已暂停」「前进 5秒: 00:37 / 01:40」「播放速度: 1.1x」「音量: 85%」「静音: 开」）。
3. 逐帧：播放中按 F 暂停并前进一帧；连按 D 五次回退约 5/fps；纯音频（mp3）时「上一帧/下一帧」菜单灰。
4. 关键帧：H.264 片源调 `player.step("keyframe","forward")` 十次，时间单调增加，与 `ffprobe -v error -skip_frame nokey -select_streams v:0 -show_entries frame=pts_time -of csv=p=0 <文件>` 的关键帧时间一致。
5. 步长：插件 `setStepSeek({ seconds: 10 })` 后菜单变为「后退10秒/前进10秒」，← 后退 10 秒；停用该插件后恢复「5秒」。
6. 覆盖层迁移：在界面状态文件的 `bindings` 里放 `"vestra.playback/toggle-pause": ["P"]`，以开关启动后文件里变成 `"host/play-pause": ["P"]`，按 P 播放/暂停。

#### 裁决：`system.openUrl`（作者 C，subtitle-online「网页中打开」）

请求：`plugin.system.openUrl(url)`，只接受 http/https，默认浏览器打开；不需要能力。证据：`openPath` 先检查路径存在，网址必然 `missing`；参考 §16 要求插件不再自己调用 `open`，插件没有合规的替代做法。

结论：**接受**（runtime 层兼容扩展，不加宿主协议）。满足 §4.3 第 4 条第四款：打开网页是通用基础能力（帮助链接、关于页、更新页都会用），各插件自己 spawn `open` 会重复且行为不一致。签名 `openUrl(url: string): Promise<void>`；按 WHATWG URL 解析并规范化，只允许 `http:` / `https:`、≤ 8192 字符，否则 `invalid`；`/usr/bin/open <规范化地址>`，10 秒 `timeout`、失败 `failed`；不检查网址可达。TestHost 记录在 `host.system.urls`。随 P3 提交实现，参考 §16.5 已更新。

#### 键盘布局（协调者实测反馈：德语 QWERTZ）

确认：Option 与 Ctrl 组合的字母、符号取 winit `key_without_modifiers`（当前布局不带修饰的字符），所以 QWERTZ 上 ⌃Y（物理 KeyZ）是 `Ctrl+Y`、⌥Z（物理 KeyY）是 `Alt+Z`，与键帽一致；单测覆盖。`Alt+Shift+数字/符号`：macOS 在按住 Option 时不报告该布局 Shift 后的字符，只能按美式键位换算（QWERTZ 上 ⌥⇧. 是 `Alt+>`，虽然该布局 Shift+. 打出 `:`）；美式键位没有的符号键（ß、´ 等）与 Option+Shift 同按时不匹配。这是已知取舍，参考 §4.1 已写明「键盘布局」一段。

#### P3 删除开关（维护者）

- 宿主：删除 `VESTRA_CORE_ACTIONS` 与 `Host.core_actions`；14 个核心动作总是注册，覆盖层迁移在每次启动时执行（已迁移的覆盖层不再变化）。
- TestHost：删除 `new TestHost({ coreActions })` 选项，核心命令总是注册，放在新的 `host.coreCommands`（不放进 `host.commands`：audio、capture、info、nes、video、youtube 的测试按 `host.commands` 枚举并断言自己的菜单，放进去会让它们全部失败）；`host.press` 与 `host.conflicts` 把核心命令排在前面（核心赢得冲突，与绑定表一致）。
- 文档：参考 §5.5、§14、§17 去掉开关说明；§17 写明「仍注册旧路径或旧按键的插件，菜单注册 `conflict`、按键被核心压下」。
- 影响面（在合并前的插件代码上实测）：除 playback 外的 10 个插件测试全部通过；**playback 2 条失败**（「menu follows the allocation table」：`host.conflicts` 出现 F4、Space 等与核心动作的冲突；「previous/next go through the player sequence」：PageUp/PageDown 被核心动作接住），这正是作者 A 的迁移要删除的旧命令，须与 playback 迁移同批合并。真实宿主上未迁移的 playback 会因菜单路径冲突激活失败。

### 6.3 作者交付

（各作者完成时填写：完成项、未完成项与原因、删除的自制实现、API 请求）

### 6.4 验收结果

（P4 填写）
