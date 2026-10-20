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

（P1a、P1b、P3 完成时填写；签名与裁决不同的条目写明原因）

### 6.3 作者交付

（各作者完成时填写：完成项、未完成项与原因、删除的自制实现、API 请求）

### 6.4 验收结果

（P4 填写）
