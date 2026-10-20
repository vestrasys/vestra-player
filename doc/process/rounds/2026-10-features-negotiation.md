# 功能第 1 轮：核心接口协商记录

依据：[社区工作流](../community-workflow.md) §4.2、§4.3；请求来源：[功能需求](../../product/feature-requirements.md)附录 A（下称「需求文档」）；轮次见 [2026-10-features-r1.md](2026-10-features-r1.md)。

本文分三步记录：第 1 步维护者回复（本节），第 2 步作者代表反驳（每条最多一次，必须附证据），第 3 步维护者裁决与排期。第 1 步只是**初判**，不是批准，也没有任何实现。

读法约定：

- **§4.3-4 的四种条件**下文简写为：〔宿主〕需要宿主内部状态或宿主独有能力；〔共享〕能力本身要跨插件共享；〔核心职责〕本来属于核心；〔通用〕多个插件需要或是以后大量插件会用到的基础能力。四条都不满足的，拟拒绝。
- **核心体积影响**按量级估：「极小」= 一个 mpv 属性透传或一个校验范围（几十行以内）；「小」= 一个新方法加状态字段（约 100–300 行，含 SDK、TestHost、文档）；「中」= 新状态机、新挂载位或新持久化（约 300–1000 行）；「大」= 改渲染或窗口架构（千行以上，有回归风险）。
- 代码示例只用现有 API（[API 参考](../../plugins/reference.md)），除非标明「改写后的形态」。
- 宿主现状以 `src/host/src`（本分支）为准；涉及的事实在各条「理由」里注明出处。

---

## 第 1 步：维护者回复

### 播放器

### R-P1 `player.stepFrame` 逐帧

- **请求方**：playback B07；核心基础动作 `frame-back` / `frame-forward`（H1）
- **初判**：拟改写
- **替代方案**（不改 API）：用 `seek` 按 1/fps 做相对跳转：
  ```ts
  const { video } = await plugin.player.stats();
  const fps = video?.fps ?? 25;
  const pos = plugin.player.state.position ?? 0;
  await plugin.player.setPaused(true);
  await plugin.player.seek(pos + 1 / fps);   // 精确 seek
  ```
  代价：VFR 片源和时间戳不规则的文件会跳过或重复帧；精确 seek 每次都从关键帧重新解码，前进方向比 mpv `frame-step` 慢一个数量级，按住 F 时明显卡顿。用户层面「逐帧」的可靠性做不到，所以这条不能拒绝。
- **改写建议**：
  1. 与 P2 合并成一个方法：`player.step(unit: "frame" | "keyframe", direction: "backward" | "forward", opts?: { ifCurrent? }): Promise<void>`。
  2. 去掉 `can.frameStep` / `can.frameBackStep` 两个状态字段。插件用已有信息推出可用性：`stats().video !== null` 表示有视频；后退另需 `state.can.seek`。条件不满足时核心以 `unsupported` 失败，不返回假成功。这样 PlayerState 不因一个功能增加字段，每次状态推送也不必为它多读 mpv 属性。
  3. 「在途时丢弃重复按键」留在调用方（插件或核心动作）：方法本身只执行一步。
- **理由**：〔宿主〕`frame-step` / `frame-back-step` 是 mpv 命令；〔核心职责〕H1 的基础动作要用它。
- **核心体积影响**：极小到小。一个 RPC 映射到两条 mpv 命令，加 `unsupported` 判断；无新状态。

### R-P2 `player.stepKeyframe` 关键帧跳转

- **请求方**：playback B08
- **初判**：拟改写（并入 R-P1）
- **替代方案**：本地文件可以在插件里用 ffprobe 只读当前位置附近的关键帧，再精确 seek：
  ```ts
  // 读 [pos-30, pos+30] 秒内视频关键帧时间，不经 shell，带超时
  execFile(ffprobe, ["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey",
    "-read_intervals", `${pos - 30}%${pos + 30}`, "-show_entries", "frame=pts_time",
    "-of", "csv=p=0", path], { timeout: 3000, signal });
  // 取严格大于 pos 的第一个 → plugin.player.seek(t)
  ```
  代价：网络来源、模拟器流做不到；每次按键起一个 ffprobe 进程（约 50–200 ms），按住重复时跟不上；GOP 很长时 ±30 秒窗口可能落空，要扩窗重试。
- **改写建议**：并入 R-P1 的 `player.step("keyframe", direction)`，核心映射为 mpv `seek ±0 relative+keyframes` 加方向；一个方法、一份错误语义，不单独开 API。
- **理由**：〔宿主〕mpv 知道解复用器的关键帧索引，插件只能另起进程重新扫描；替代方案对网络来源不可用。但它只是 P1 的另一个单位，不值得一个独立方法。
- **核心体积影响**：极小（P1 的一个分支）。

### R-P3 `player.setStepSeek` 第 1 档步长

- **请求方**：playback B06；基础动作移交（H1）
- **初判**：拟接受（随 H1 一起做）
- **替代方案（已考虑）**：
  - 让 playback 不用核心的 ←/→，自己再注册第 1 档命令并抢绑定：与 H1「卸载 playback 后 ←/→ 仍可用」的目标冲突，两个命令争同一键还要靠加载顺序裁决。
  - 降低需求：第 1 档固定 5 秒，B06 只开放第 2–4 档。代价是参考播放器用户最常调的就是第 1 档。
- **理由**：〔核心职责〕H1 把 ←/→ 交给核心之后，步长就是核心动作的参数；只有核心能改它。设置仍由 playback 保存，核心不持久化。
- **约束**：「调用插件停用后恢复 5 秒」沿用滤镜的撤销规则（每个作用都记在调用者名下）；多个插件调用时最后写入生效。
- **核心体积影响**：极小。一个数字状态与撤销钩子。

### R-P4 `plugin.host.run(action)` 执行核心动作

- **请求方**：playback 底栏按钮（基础动作移交）
- **初判**：拟接受（随 H1）
- **替代方案（已考虑）**：底栏按钮直接调 `setPaused`、`seek`、`setOutput` 等，自己显示 OSD。代价：按钮和快捷键是两份实现，累加规则（连按 → 从最新目标累加）、钳制、提示文字会慢慢分叉，这正是交互要求 §1「共享状态与行为」要避免的。
- **约束**：
  - `action` 只能是核心注册的动作 id，不是任意命令 id。插件不能借它执行别的插件的命令（否则就是插件间通信）。
  - 动作禁用时以 `unavailable` 失败，不排队。
  - 不新增能力名，沿用 `commands`。
- **理由**：〔核心职责〕核心是固定存在的一方，执行核心动作不依赖任何插件。
- **核心体积影响**：极小（H1 已有动作表，这里只是一个按 id 分派的 RPC）。

### R-P5 `player.preload` 无缝播放

- **请求方**：playlist B26
- **初判**：拟拒绝（本轮不做；带测量证据可再提）
- **替代方案**：
  1. 维持现在的 `media.onEnd → player.open(next, { ifCurrent: ended })`。playlist 可以把推进路径缩到最短：提前算好下一项、提前对本地文件做 `fs.stat`，`onEnd` 回调里只做一次 `open`。
  2. 降低需求：B26 改为「快速推进」，不承诺无间隙；对专辑类内容，用户可以把多首合成单文件（不在本插件范围）。
- **代价**：切换处会有一段静音和一帧黑屏。目前**没有测量值**：它可能是 100 ms，也可能是 500 ms。
- **反驳所需证据**：在当前宿主上测两段相接 flac 的切换间隙（录音波形），并给出用户可感知的标准。若间隙确实超过约 150 ms，再讨论核心方案。
- **理由**：只有一个插件使用，四条条件里只沾〔宿主〕。但它的代价特别高：要把 mpv 内部播放列表（`loadfile … append`、`prefetch-playlist`、`gapless-audio`）引入现在「一次一个来源」的会话状态机（`session.rs`、`source_backend.rs`），还要在当前来源结束前为下一个本地文件建立 fd 租约（`source_lease.rs`）；同时改变「旧来源 ended → 新 sourceId」的顺序契约，所有用 `ifCurrent` 的插件都受影响。属于 §4.2 第 3 条所说「改生命周期也是破坏兼容」。先要证据。
- **核心体积影响**：中到大。新状态机分支、新的资源租约时序、跨插件的契约改动。

### R-P6 `PlayerState.videoSize` 显示尺寸

- **请求方**：window H03、H04、H07
- **初判**：拟改写（并入 R-P7，不进 PlayerState）
- **替代方案**：window 插件在**执行命令时**查一次统计，不需要状态推送：
  ```ts
  run: async () => {
    const { video } = await plugin.player.stats();
    if (!video?.width || !video.height) return plugin.osd.show("没有视频");
    await plugin.window.setVideoArea({ width: video.width * k, height: video.height * k }); // W2
  }
  ```
  现有 `video.width/height` 是解码尺寸（`video-params/w/h`），不含 DAR，所以 720×576（16:9）会算错。
- **改写建议**：在 `PlaybackStats.video` 里加 `displayWidth`、`displayHeight`（mpv `video-out-params/dw`、`dh`，含 DAR 和宿主方向处理后的尺寸），即 P7 已列的同名字段。命令的 `enabled` 用 `player.state.phase` 判断，「没有视频」在执行时提示（降低一点需求：菜单不预先灰掉纯音频的情况；或者 window 插件在 `sourceId` 变化后的 ready 时查一次 `stats()` 缓存起来，用于 `enabled`）。
- **理由**：显示尺寸属于〔宿主〕信息，但 PlayerState 每次推送都要重建（`source_backend.rs::snapshot` 每次都读 mpv 属性），把只在执行命令时用到的数据塞进去不划算。按需查询足够。
- **核心体积影响**：极小（P7 的两个字段）。

### R-P7 `PlaybackStats` 扩展字段

- **请求方**：info I05；window H07 验收引用 `avSyncMs`；并入 P6、P8
- **初判**：拟改写（接受，字段收窄）
- **替代方案（已考虑）**：ffprobe 只能读文件，读不到正在解码的会话：硬解是否生效、当前帧率、音画同步、缓冲时长都没有。网络来源更读不到。这部分没有插件内办法。
- **改写建议**：
  - 字段只透传 mpv 原始值，全部可为 `null`，不做二次加工：`video.decoder`（`video-codec`）、`pixelFormat`、`colorspace`（`video-params/colormatrix`）、`displayWidth/displayHeight`（P6）、`displayFps`、`bitrate`、`transfer`（P8，原样给 `video-params/gamma` 字符串，不映射成枚举）；`audio.decoder`、`format`、`channelLayout`、`bitrate`；`avSyncMs`、`cacheSeconds`。
  - `hwdec` 暂不加：宿主固定 `hwdec=no`（`backend.rs`），现在这个字段恒为 `null`，等硬解路径（C-GPU 之后）再加。
  - 仍是按需查询（`stats()`），不推送。
- **理由**：〔宿主〕全部来自 mpv 属性；`backend.rs::stats` 本来就是透传函数，扩展是加几行属性读取。
- **核心体积影响**：极小。

### R-P8 `PlaybackStats.video.transfer`

- **请求方**：video E16
- **初判**：拟改写（并入 R-P7）
- **替代方案**：本地文件可在插件里 `ffprobe -show_entries stream=color_transfer` 读到 `smpte2084` / `arib-std-b67`；网络来源读不到。
- **改写建议**：作为 P7 的 `video.transfer: string | null`，原样给 mpv `video-params/gamma`（如 `"pq"`、`"hlg"`、`"bt.1886"`），判断「是不是 HDR」由插件做。它只在 E16 有意义，而 E16 依赖 C-GPU，所以排到第 4 轮随 V10 一起发布。
- **理由**：〔宿主〕；但不值得单独一个枚举类型。
- **核心体积影响**：极小。

### 轨道与字幕

### R-T1 `player.addAudioTrack` 载入外部音轨

- **请求方**：audio A11
- **初判**：拟接受
- **替代方案（已考虑）**：插件用 ffmpeg 把视频和外部音频封装成临时 mkv 再 `open`：要整片 remux（大文件几十秒到几分钟），换掉了当前来源（`sourceId` 变化、位置要恢复），其它插件的状态全部重置。用户层面做不到「为当前视频加一条音轨」。
- **约束**：默认以当前 `player.state` 为条件；外部音轨随来源消失；`AudioTrack.external = true`、`exportable = false`（已有规则）；本地路径同样走宿主的路径校验。
- **理由**：〔宿主〕mpv `audio-add` 只有宿主能调。
- **核心体积影响**：小。一个命令，加 `audioTracks` 状态刷新（已有）。

### R-T2 `player.selectTrack("audio", id)` 选音轨

- **请求方**：audio G03、G04
- **初判**：拟改写（形态收窄）
- **替代方案（已考虑）**：没有。`aid` 只有宿主能写，插件看得到 `audioTracks` 但改不了。
- **改写建议**：去掉为「以后扩展」预留的 `kind` 参数（E15 视频流已删除，现在只有音频一个用途），写成 `player.selectAudioTrack(id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>`。以后真要选视频流时再加独立方法，不为假想需求预留参数。
- **理由**：〔宿主〕mpv `aid`。另外 playback 以后的「流选择」按钮也会用它，满足〔通用〕。
- **核心体积影响**：极小。

### R-T3 `subtitles.selectSecondary` 次字幕

- **请求方**：subtitles A08
- **初判**：拟改写
- **替代方案**：
  - 插件把两条轨合并成一个 ASS（主轨在底部、次轨用 `\an8` 放顶部），整体 `load`：只适用于本插件能拿到文本的轨；内嵌轨要先提取（见 R-T5 的替代方案），样式与同步要插件自己合成，主字幕换轨时要重新合成上传。代价大且行为与普通主字幕不一致。
  - 降低需求：次字幕只支持外部文件。
  这条确实需要 `secondary-sid`。
- **改写建议**：
  - 不新增方法，扩展现有 `select`：`subtitles.select(track | null, opts?: { role?: "primary" | "secondary"; ifCurrent? })`，默认 `"primary"`，与现在一致。
  - `SubtitlesState` 只加 `secondary: SubtitleTrack | null`；不加独立的 `secondaryConfirmation`，沿用 `trackConfirmation`（两次选择都由同一个确认字段报告最近一次提交）。
  - 「主次不能同一条」「图形轨不可作次字幕」由核心校验，`invalid` 失败。
- **理由**：〔宿主〕mpv `secondary-sid`、`secondary-sub-pos`。
- **核心体积影响**：小。`subtitle_state.rs` 多一个选择槽和来源变化时的清空。

### R-T4 `subtitles.load` 支持 ASS/VTT、标题与选择方式

- **请求方**：subtitles A08、A09、D17、D18；subtitle-online D16
- **初判**：拟改写（接受，范围收窄）
- **替代方案**：
  - VTT：插件转 SRT 上传（丢 `::cue` 样式，本来也被忽略），可以不进核心。
  - ASS：转 SRT 会丢失定位和特效，D17 的意义就没了；而宿主现在只用 SRT 解析器校验（`subtitle_upload.rs`），ASS 字节不能上传。
- **改写建议**：
  - `format?: "srt" | "ass"`（`ssa` 按 `ass` 处理，宿主按内容识别 `[Script Info]`）。**不加 `"vtt"`**：VTT 由插件转成 SRT，和 SAMI、MicroDVD 走同一条插件内转换路径，核心只维护两种校验器。
  - `title?: string`：接受，几行代码，解决宿主临时文件名显示给用户的问题。
  - `select?: "primary" | "secondary" | "none"`：接受，`"secondary"` 依赖 R-T3。
  - ASS 校验按需求文档的有界规则（UTF-8、≤ 2 MiB、两个必需段、Dialogue ≤ 20 000）。
- **理由**：〔宿主〕临时文件扩展名与校验在宿主；〔通用〕subtitles 与 subtitle-online 两个插件都要用。
- **核心体积影响**：小。一个 ASS 有界校验器加两个选项。

### R-T5 `subtitles.cues` 读取逐句时间表

- **请求方**：subtitles B11
- **初判**：拟拒绝
- **替代方案**：
  - 本插件载入的外部轨：插件手里就有字节，直接解析（SRT/ASS 解析几十行，npm 上也有成熟的 `subtitle` 包）。
  - 内嵌文本轨（本地文件）：插件用 ffmpeg 抽出该流转 SRT，再按同一解析器处理：
    ```ts
    // track.embeddedId 是宿主给出的内嵌轨编号；按 ffprobe 的字幕流顺序换成 ffmpeg 流索引
    execFile(ffmpeg, ["-v", "error", "-nostdin", "-i", source, "-map", `0:s:${n}`, "-f", "srt", "pipe:1"],
             { timeout: 15000, maxBuffer: 8 << 20, signal });
    ```
    结果按 `track.id` 缓存，换轨、换来源失效（需求文档本来就这样设计）。
  - 网络来源的内嵌字幕、其它插件载入的外部轨：命令禁用，原因「此字幕不支持按句跳转」。
- **代价**：大 MKV 抽字幕要扫整个文件，可能几秒（需求文档已有「正在读取字幕…」提示）；`embeddedId` 与 ffmpeg 流索引的对应要插件核对；网络来源不支持。
- **反驳所需证据**：典型片源（如 4 GB MKV）上 ffmpeg 抽取的实测耗时；或 `embeddedId` 与 ffprobe 顺序对不上的样例。宿主确实已有 `native_subtitle.rs::extract`，若证据成立，开放成本不高，可以再议。
- **理由**：只有 B11 一个功能用，不满足〔共享〕〔通用〕；〔宿主〕只对网络来源成立，而这是边缘情况。
- **核心体积影响（若接受）**：小到中（新查询、20 000 句上限、文本清洗）。

### R-T6 `SubtitleTrack.codec`、`SubtitleTrack.text`

- **请求方**：subtitles A08、B11、D18
- **初判**：拟接受
- **替代方案（已考虑）**：本地文件可 ffprobe 读 `codec_name` 再映射到轨道；网络来源读不到，而且映射要靠 R-T5 里同样脆弱的编号对应。
- **约束**：`text` 由核心按 codec 判定（`subrip`、`ass`、`webvtt`、`mov_text` 等为文本；`hdmv_pgs_subtitle`、`dvd_subtitle` 为图形；未知为 `false`），判定表写进参考文档。
- **理由**：〔宿主〕宿主已从 mpv `track-list` 读到 codec，只是没开放；R-T3 的「图形轨不可作次字幕」校验本身也要它。
- **核心体积影响**：极小。

### R-T7 `SubtitleStyle` 新增样式字段

- **请求方**：subtitles D09、D11、D17、A08
- **初判**：拟改写（接受，字段合并）
- **替代方案（已考虑）**：插件把样式写进上传的 ASS（`Style:` 行）：只对本插件载入的外部轨有效，内嵌轨不受控，与现有 `setStyle` 的作用范围不一致。不可行。
- **改写建议**：
  - `bold?: boolean`（`sub-bold`）、`shadow?: number`（0–4，`sub-shadow-offset` 按 1080p 缩放）：接受。
  - `box` 与 `boxOpacity` 合成一个字段：`box?: number | null`（`null` 或省略 = 关；0–100 = 不透明度）。少一个互相依赖的字段组合。
  - `assOverride?: "keep" | "force"`：接受，即 H12。
  - `secondaryPosition?: "top" | "aboveMain"`：随 R-T3 一起做；libmpv 不支持 `secondary-sub-pos` 时以 `styleReason` 说明并降级为顶部。
  - 全部可选，省略即保持现在的效果；旧调用不变。
- **理由**：〔宿主〕mpv 样式属性只有宿主能写。
- **核心体积影响**：小。`subtitle_style.rs` 几个属性映射与范围校验。

### R-T8 `SubtitleStyle.offsetMs` 放宽到 ±600 000

- **请求方**：subtitles D05
- **初判**：拟接受
- **替代方案（已考虑）**：插件对外部轨把时间轴整体平移后重新上传：内嵌轨做不到，且每次调同步都要重传。
- **理由**：〔宿主〕`sub-delay` 只有宿主能写；改的只是校验范围，mpv 本身无此上限。
- **核心体积影响**：极小（一个常量与测试）。

### 画面与渲染

处理节点的共同规则：每个新 `VideoNode` 都要同时进入实时 mpv 图与离线导出图（`effects.rs::render_lavfi` 共用），受 64 个节点总上限约束，要有范围校验、运行时参数标记和单元测试。现有节点每个约 30 行（`effects.rs`），加上 SDK 类型、参考文档、离线导出验证，实际每个节点约 80–120 行。所以下面按「能否少一个节点类型」逐个审。处理节点本身属于〔宿主〕（插件不能写滤镜字符串，这是安全与离线一致性的前提），因此原则上接受，只收窄数量。

### R-V1 `hue` 节点

- **请求方**：video E02
- **初判**：拟接受
- **替代方案（已考虑）**：用 `eq` 节点模拟色相：`eq` 没有色相参数，做不到。
- **约束**：FFmpeg `hue=h=<deg>`，`degrees` −180–180，标为运行时参数（拖动滑块不重建图）；需在实时图上验证 `hue` 的运行时命令确实生效后再标 runtime。
- **理由**：〔宿主〕；色相是基础画面调节。
- **核心体积影响**：极小。

### R-V2 `sharpen`、`cas` 节点

- **请求方**：video E04
- **初判**：拟改写（只加一个节点）
- **替代方案**：降低需求：「锐化」与「边缘锐化」用同一种滤镜的两组参数实现。
- **改写建议**：只加 `{ kind: "sharpen"; amount: number; size: "small" | "large" }` → FFmpeg `unsharp`（`size` 决定 `luma_msize_x/y` 3 或 7；`amount` 映射到 `luma_amount`，0–2）。「锐化」用 `small`，「边缘锐化」用 `large`（大窗口的 unsharp 主要增强轮廓）。不加 `cas`。
- **代价**：参考播放器的「边缘锐化」算法不同，观感有差别；作者如能给出对比截图说明 `unsharp` 大窗口在用户层面明显达不到效果，再考虑 `cas`。
- **理由**：〔宿主〕；但两个节点对一个功能是不必要的扩张。
- **核心体积影响**：极小。

### R-V3 `denoise3d`、`temporalDenoise` 节点

- **请求方**：video E05
- **初判**：拟改写（只加一个节点）
- **替代方案**：降低需求：三种降噪（3D / 瞬态 / 渐进）作为插件内预设，全部映射到 `hqdn3d` 的四个参数（空间亮度、空间色度、时间亮度、时间色度）。「瞬态」= 时间参数大、空间参数小；「渐进」= 中等平衡值。
- **改写建议**：只加 `{ kind: "denoise"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` → `hqdn3d`，范围 0–20。不加 `temporalDenoise`。
- **理由**：〔宿主〕；`hqdn3d` 的时间分量已经覆盖了时间降噪。
- **核心体积影响**：极小。

### R-V4 `deblock`、`deband` 节点

- **请求方**：video E06
- **初判**：拟接受
- **替代方案（已考虑）**：两者是不同算法（块效应 vs 色带），互相替代不了；插件也不能自己加滤镜。
- **约束**：`deblock` 的 `strength` 只给 `weak | strong` 两档（FFmpeg `deblock=filter=weak|strong`）；`deband` 的 `threshold`、`range` 给出明确范围。
- **理由**：〔宿主〕。
- **核心体积影响**：极小（两个节点）。

### R-V5 `flip`、`transpose` 节点

- **请求方**：video E08、E09
- **初判**：拟改写（合成一个节点）
- **替代方案（已考虑）**：mpv `video-rotate` 在软件渲染器下无效（宿主已改用 `source_geometry.rs` 的滤镜处理容器方向），插件无替代。
- **改写建议**：一个节点 `{ kind: "transform"; rotate: 0 | 90 | 180 | 270; flipH: boolean; flipV: boolean }`，核心展开为 `transpose` / `hflip` / `vflip` 的组合。一个节点对应「画面方向」这一个概念，插件的链也更短。
- **约束**：与宿主自己的方向处理（容器旋转元数据）叠加的顺序要写清：宿主方向在前，插件的 `transform` 作用于「已摆正」的画面。
- **理由**：〔宿主〕。
- **核心体积影响**：极小到小（组合展开与尺寸交换的测试）。

### R-V6 `deinterlace` 节点

- **请求方**：video E10
- **初判**：拟改写（参数收窄）
- **替代方案**：全局 mpv `deinterlace` 属性不随插件生命周期撤销，不适合作为插件效果；插件内没有替代。
- **改写建议**：`{ kind: "deinterlace"; when: "interlaced" | "all"; output: "frame" | "field" }`，算法固定 `bwdif`。去掉 `method` 选项（`yadif` 是 `bwdif` 的前代，画质不如后者，两种都提供只是对照参考播放器菜单）。
- **理由**：〔宿主〕。
- **核心体积影响**：极小。

### R-V7 `crop`、`pad` 节点

- **请求方**：video E12
- **初判**：拟改写（合成一个节点）
- **替代方案**：插件用 `stats().video.width/height` 自己算像素尺寸再发通用 `crop`：插件在 `transform` 之后、宿主方向处理之后的真实输入尺寸并不总是确定，换来源还要重算，容易错。不可取。
- **改写建议**：一个节点 `{ kind: "reframe"; mode: "crop" | "pad"; aspect: number; x: number; y: number }`（`aspect` 0.25–4；`x`、`y` −1–1 为偏移）。核心用 FFmpeg 表达式（`iw`、`ih`）生成 `crop` / `pad`，不需要知道输入尺寸，也就不需要核心在运行时计算。
- **理由**：〔宿主〕。
- **核心体积影响**：小（表达式生成与边界测试）。

### R-V8 `frameBlend` 节点

- **请求方**：video E18（倍帧）
- **初判**：拟接受（带性能验收门槛）
- **替代方案（已考虑）**：无插件内办法；mpv `interpolation` 依赖 C-GPU。
- **约束**：FFmpeg `framerate=fps=N`（混合插帧），`fps` 24–120，非运行时参数；它改变输出帧率，必须验证软件渲染路径的帧调度（宿主按什么节奏取帧）在 48/60 fps 下不丢帧，并验证离线导出帧率随之变化。验收不过就退回。
- **理由**：〔宿主〕。
- **核心体积影响**：极小（节点），但有渲染节奏回归风险。

### R-V9 `plugin.video.view()` 显示层

- **请求方**：video E11、E13
- **初判**：拟改写
- **替代方案**：
  - 用 R-V7 的 `reframe(pad)` 模拟宽高比：会进截图和导出，违背「显示层效果不影响截图」，而且不改变窗口里的显示框。
  - 降低需求：只做裁剪/拉伸（E12），不做显示层。代价是丢掉宽高比覆盖、填满窗口、缩放平移整组功能。
  这些是 mpv vo 级属性，插件内确实没有办法。
- **改写建议**：
  - 字段收窄为 `aspect: number | null`、`fit: "keep" | "stretch" | "fill"`、`zoom`、`panX`、`panY`。去掉 `scaleX` / `scaleY`（参考播放器的「帧宽 / 帧高 ±」是很少用的边缘功能；需要时用 `aspect` 达到同样的变形）。E13 相应删掉帧宽、帧高四条命令。
  - 所有权改为**单一拥有者**，与 `provideSequence` 相同：第一个调用 `view()` 的插件拥有它，第二个以 `conflict` 失败；拥有者停用时核心恢复默认。不做「多插件合并、最后写入生效」，省掉字段级的归属记录。
  - 状态沿用 `FilterStatus` 四态。
  - 前提 C-SIZE（H2）。C-SIZE 之前不开放此接口，避免发布一个结果错误的 API（需求文档已说明不在 16:9 窗口里结果是错的）。
- **理由**：〔宿主〕mpv vo 属性。
- **核心体积影响**：小（属性映射与撤销），排在 C-SIZE 之后。

### R-V10 `plugin.video.render()` GPU 渲染选项

- **请求方**：video E16、E17、E18（运动补偿）
- **初判**：拟改写（推迟到 C-GPU 同批发布）
- **替代方案**：软件路径没有办法：本机 FFmpeg 没有 `zscale` / `libplacebo`，CPU 上 4K 色调映射也不可能实时；GLSL 着色器与 mpv 插帧只在 GPU 渲染器里有效。
- **改写建议**：
  - 不提前发布一个「`status` 恒为 `failed / unsupported`」的接口。C-GPU 之前插件的相关命令直接写死禁用原因（需求文档已经这样设计），不需要核心配合。
  - C-GPU 完成后与 V9 合并为同一个句柄的两组字段，还是分开，届时按 V9 的实际形态再定；所有权同 V9 用单一拥有者。
  - `shaders` 上限 16、要求绝对路径且文件存在，编译失败以 `failed` 报告 mpv 的原文。
- **理由**：〔宿主〕；但时机取决于 H3。
- **核心体积影响**：小（属性映射），前提 H3 是大改动。

### R-V11 `Snapshot.export` 支持 JPEG

- **请求方**：capture F07
- **初判**：拟拒绝
- **替代方案**：导出 PNG 后用 macOS 自带 `sips` 转换，需求文档已按此设计：
  ```ts
  const snap = await plugin.media.capture({ processed: true });
  const job = await snap.export({ format: "png", destination: tmpPng });
  await job.done;
  execFile("/usr/bin/sips", ["-s", "format", "jpeg", "-s", "formatOptions", String(q), tmpPng, "--out", dest],
           { timeout: 10000 });
  ```
- **代价**：多一次编码和临时文件，单张截图多约 100 ms；用户不可见。
- **理由**：需求文档自己标「可选、不做不影响功能」；四条条件都不满足。
- **核心体积影响（若接受）**：小。

### 音频

### R-A1 `channels` 音频节点

- **请求方**：audio G05
- **初判**：拟接受
- **替代方案（已考虑）**：插件不能加滤镜；现有节点（音量、均衡、高架、动态归一）都做不了声道路由。
- **约束**：`mode: "stereo" | "left" | "right" | "mono" | "swap"`，`stereo` 时插件不发；多于 2 声道时核心前置下混为立体声（`aresample` / `pan` 下混），下混是这个节点内部的事，不暴露参数；运行时可调需实测确认，不能确认就标非运行时。
- **理由**：〔宿主〕。
- **核心体积影响**：极小到小。

### R-A2 `player.audioDevices` 设备列表

- **请求方**：audio G16
- **初判**：拟改写（与 A3、A4 合成一个设计，见合并议题 3）
- **替代方案**：插件可用 `system_profiler SPAudioDataType -json` 列设备，但拿到的不是 mpv 能用的设备 id，也就无法用来切换；单独列出没有意义。
- **改写建议**：见 R-A3。
- **理由**：〔宿主〕mpv `audio-device-list`。
- **核心体积影响**：极小。

### R-A3 `player.setAudioDevice` 选择输出设备

- **请求方**：audio G16
- **初判**：拟改写
- **替代方案**：切换 macOS 系统默认输出（如 `SwitchAudioSource` 命令行工具）：改的是全系统设置，用户其它应用的声音也跟着变，不可接受。
- **改写建议**（A2–A4 合并后的形态）：
  ```ts
  player.audioDevices(): Promise<{ id: string; name: string }[]>   // playback.read
  player.setAudioDevice(id: string | null): Promise<void>          // playback.control；null = 系统默认
  PlayerState.audioOutput: { requested: string | null; active: string | null }
  ```
  - 去掉 `devicesRevision`：设备插拔会让 `active` 变化（回退或切回）时自然推送一次状态；只是新插入一个**未被选中**的设备时没有推送，插件在用户打开「输出设备」子菜单所在窗口、或每 5 秒轮询一次 `audioDevices()`（一次 RPC，读一个 mpv 属性，成本可忽略）发现。代价：新设备出现在菜单里最多延迟 5 秒。作者若认为不可接受，可以把 `devicesRevision` 作为反驳内容。
  - 设备选择由核心记住在**本次运行**内；跨启动保存由 audio 插件负责（激活时调用一次），核心不持久化。
  - 生成流（cpal）路径见 H9。
- **理由**：〔宿主〕mpv `audio-device`，回退与切回也在宿主。
- **核心体积影响**：小。

### R-A4 `PlayerState.audioOutput`

- **请求方**：audio G16
- **初判**：拟改写（并入 R-A3，字段收窄为 `requested`、`active`）
- **替代方案**：插件无法得知回退（设备被拔）这一宿主内事实。
- **理由**：〔宿主〕。
- **核心体积影响**：极小。

### R-A5 音频 `delayMs` 放宽到 ±10 000

- **请求方**：audio G09、A11
- **初判**：拟接受
- **替代方案（已考虑）**：无；延迟只能由处理链执行。
- **约束**：`processing.rs` 现有「正负贡献各不超过 2000 ms，保证某个插件移除后仍可表示」的预算要同步放宽到 10 000，保留按 10 ms 取整；补齐预算边界测试。
- **理由**：〔宿主〕；外部音轨（A11）偏差超过 2 秒是常见情况，A11 已拟接受。
- **核心体积影响**：极小。

### 窗口与应用

### R-W1 `window.state` / `window.onChange`

- **请求方**：window H03–H05、H07
- **初判**：拟改写（字段收窄）
- **替代方案**：插件读不到窗口尺寸与全屏状态（宿主内部）。没有替代。
- **改写建议**：`WindowState { fullscreen; videoArea: { width; height }; pictureInPicture: { supported; active } }`。去掉 `onTop`：置顶只能由调用 `setAlwaysOnTop` 的插件改变，它自己知道；核心撤销时插件也已停用。`pictureInPicture` 随 W5 在第 4 轮才出现，之前不放字段。
- **理由**：〔宿主〕。`fullscreen` 与 `videoArea` 以后其它插件（比如依赖全屏判断的界面）也会读，满足〔通用〕。
- **核心体积影响**：小（`window_state.rs` 已有快照，只是开放并推送）。

### R-W2 `window.setVideoArea`

- **请求方**：window H03、H04
- **初判**：拟接受
- **替代方案（已考虑）**：`osascript` + System Events 调整窗口大小：需要辅助功能授权；插件不知道底栏、侧栏、分隔条与最小尺寸规则，算出的视频区必然有偏差。不可行。
- **约束**：参数是视频区（逻辑点），核心按现有布局规则（`window_state.rs`）加上栏位并夹到屏幕可见区；全屏时以 `unavailable` 失败；返回实际值与 `clamped`。
- **理由**：〔宿主〕窗口几何只有宿主知道。
- **核心体积影响**：小。

### R-W3 `window.setAlwaysOnTop`

- **请求方**：window H05
- **初判**：拟接受
- **替代方案（已考虑）**：插件进程无法改别的进程的 NSWindow level；AppleScript 没有置顶。没有替代。
- **约束**：作用于主窗口、浮动侧栏与之后新开的工具窗口；插件停用时核心恢复。
- **理由**：〔宿主〕。
- **核心体积影响**：小（`objc2-app-kit` 已有 `NSWindow` 特性；要处理新窗口继承层级）。

### R-W4 `window.setCursorAutoHide`

- **请求方**：window B30
- **初判**：拟接受
- **替代方案（已考虑）**：插件收不到视频区指针事件，也不能隐藏宿主的指针。没有替代。
- **约束**：现在的规则是 `cursor-hidden = fullscreen && idle && playing`（`main.rs`），本接口只把 `fullscreen` 这一项放宽为「或 mode == playing」，空闲计时、遮挡规则全部沿用；插件停用时恢复。
- **理由**：〔宿主〕。
- **核心体积影响**：极小。

### R-W5 `window.setPictureInPicture`

- **请求方**：window H07
- **初判**：拟接受（第 4 轮，随 H8）
- **替代方案**：
  - 原需求「迷你尺寸」：用 W2 把视频区缩到最小（再配 W3 置顶）即可实现，不需要新 API。
  - 降低需求为上面这一形态，代价是没有 macOS 系统画中画（跨 Space、系统管理位置）。
  画中画本身只能由宿主经 AVKit 发起，没有插件内办法。
- **理由**：〔宿主〕窗口、图层与帧都在宿主。
- **核心体积影响**：大（见 H8）。因此排在第 4 轮；第 2–3 轮 window 插件可以先用「迷你尺寸」形态上线，作者自行决定。

### R-W6 `plugin.app.info` / `onChange`

- **请求方**：info I05、I09
- **初判**：拟改写
- **替代方案**：
  - Node 版本：插件用 `process.versions.node`（与宿主给插件用的是同一个 Node）。
  - 插件 API 版本：manifest 的 `apiVersion` 就是 `vestra.experimental/0.1`。
  - mpv、libav、Slint、Vestra 版本与构建号：宿主链接期的信息，插件拿不到。
  - 已加载插件列表：见 R-G1。
- **改写建议**：
  - `plugin.app: { version: string; build: string | null; components: { mpv: string | null; libav: Record<string, string>; slint: string } }`，**同步只读属性**，在激活时随握手下发，不需要方法调用，也不需要 `onChange`（运行期间不会变）。不需要能力。
  - 去掉 `AppInfo.plugins` 与 `app.onChange`：插件列表统一走 R-G1 的 `packages.list` / `onChange`，不保留两份。
- **理由**：〔宿主〕版本信息；〔通用〕「关于」「播放属性」以及以后任何报错、诊断类插件都会用。
- **核心体积影响**：极小（构建期常量与握手字段）。

### 界面与输入

### R-U1 `plugin.ui.image(path)` 运行时图片

- **请求方**：playback I07；visualizer J11、H12
- **初判**：拟接受
- **替代方案（已考虑）**：
  - Slint 编译期 `@image-url`：只能用包内固定文件，缩略图、封面是运行时生成的。
  - 用 Slint 画像素（把图片编码进 JSON 数组）：数据量大、慢，桥也有大小上限。
  没有替代。
- **约束**：
  - `ImageRef` 不透明，只能由该函数生成；PNG/JPEG，≤ 16 MB 且 ≤ 4096×4096。
  - 路径允许在本插件 `dataDirectory` **或本插件包目录**下（后者让插件不必把包内资源复制到数据目录）；越界、符号链接越界拒绝。
  - 同一路径文件内容变化时，插件需重新调用 `ui.image` 得到新引用（宿主按引用缓存解码结果，不监视文件）。
  - `ui_values.rs` 现在明确拒绝 image，要扩展 `PropValue` 的转换与面板 / 窗口两条写入路径。
- **理由**：〔通用〕两个插件已经需要，以后显示封面、头像、预览的插件都会用；〔宿主〕Slint 图片只能在宿主进程构造。
- **核心体积影响**：小到中（解码、尺寸校验、缓存与释放、SDK 类型）。

### R-U2 拖放：`drops.provide`、`Panel.onDrop`、`Panel.onDragOver`

- **请求方**：playlist A15
- **初判**：拟改写
- **替代方案**：降低需求：不支持拖放，用「打开文件…」「添加文件夹…」代替。代价：拖放是 macOS 用户最常用的打开方式之一，缺失明显。插件内没有办法收到系统拖放（winit 事件只到宿主窗口）。
- **改写建议**：
  - 视频区：`plugin.drops.provide({ extensions: string[]; drop(e: { paths: string[]; modifiers }): unknown }): Disposable`，能力 `ui.drop`。核心按扩展名分发；同一扩展名多个提供者时按「先注册的生效」并写日志（与绑定冲突同规则）；没有提供者的扩展名，核心什么也不做。
  - 面板：`Panel.onDrop(listener: (e: { paths; at: { x; y }; modifiers }) => unknown)`，只给落点，不给持续的 `onDragOver`。插入线降级为「落点所在行之前插入」，拖动过程中不画插入线。代价：拖动时没有实时位置反馈。作者若能说明没有插入线导致误放（例如长列表里无法预期位置），可作为反驳。
  - 字段名统一用 `extensions`（小写、不带点），不用含义更宽的 `kinds`。
- **理由**：〔宿主〕系统拖放只到宿主窗口；〔通用〕视频区拖放以后字幕（拖 `.srt`）、音频（拖 `.mka`）插件都会用，按扩展名分发是通用机制。
- **核心体积影响**：中（winit 拖放事件、落点换算到面板坐标、分发表）。去掉 `onDragOver` 省掉高频事件转发这一块。

### R-U3 背景层挂载位 `panels.mount("backdrop")`

- **请求方**：visualizer J11、H12
- **初判**：拟改写
- **替代方案**：
  - 用侧栏或底栏显示封面：不是视频区，需求本身就落空。
  - 降低需求：只做 OSD 信息块文字（`osd.panel()` 显示曲名），不显示封面与徽标。
  视频区由宿主绘制，插件内没有办法。
- **改写建议**：
  - `panels.mount("backdrop", file)`；`panels.backdrop.state: { effective: boolean } | null`。去掉 `mode: "idle" | "audio"`：插件用已有的 `player.state.phase`（`empty` = 空闲）与 `stats().video`（`null` = 无视频）自己区分该画哪种内容；核心只负责「没有视频帧时显示背景层」这一件事。
  - 同时只有一个背景层（第二个 `conflict`）；不接收输入，层次按需求文档：在宿主输入层之下、屏幕提示与信息块之下。
  - 复用面板挂载的组合机制（`composition.rs`），不另做一套。
- **理由**：〔宿主〕视频区与「有无视频帧」都在宿主；它是通用挂载位，不含业务。
- **核心体积影响**：中（新的挂载位、可见性规则、首次挂载的界面重建）。

### R-U4 （条件性）面板 `PopupWindow` 越出边界

- **请求方**：playback I06、I07
- **初判**：拟拒绝（条件未触发）
- **替代方案**：悬停提示放在进度条上方的面板内部区域（底栏高 62px，提示框可画在按钮行上方的进度条区域内）；或把底栏在悬停时临时加高。先实现，确有裁切再提。
- **理由**：需求文档自己标为条件性请求，目前没有复现。
- **核心体积影响（若接受）**：中（Slint 面板组合下的弹出层）。

### R-U5 （可选）`plugin.commands.popup(path)`

- **请求方**：video E17（`S` 键弹出「像素着色」子菜单）
- **初判**：拟拒绝
- **替代方案**：`S` 不绑（需求文档已写明可以接受）；或者绑到「使用着色器」开关本身。若以后需要在面板里弹出本插件命令，已有 `Panel.showMenu`。
- **理由**：单个插件的便捷键，四条条件都不满足；弹出**主菜单子菜单**还会让插件拿到别的插件的菜单项入口。
- **核心体积影响（若接受）**：小。

### 时间轴与标记

### R-M1 `PlayerState.chapters`

- **请求方**：marks B21、B22
- **初判**：拟改写（改为按需查询，不进 PlayerState）
- **替代方案**：本地文件 `ffprobe -show_chapters -of json`；网络来源（多为解析器给出的流地址）读不到，而 marks 也不知道解析后的地址。
- **改写建议**：`player.chapters(): Promise<{ sourceId: number; chapters: { title: string | null; start: number }[] }>`（需 `playback.read`），插件在每次 `sourceId` 变化、`phase` 变为 `ready` 后查一次并缓存。去掉 `index`（数组下标即是）。不放进 PlayerState：`source_backend.rs::snapshot` 每次推送都重读 mpv 属性，章节列表静态不变，不该随每次位置更新一起序列化。
- **理由**：〔宿主〕mpv `chapter-list`，网络来源只有宿主能读。
- **核心体积影响**：极小。

### R-M2 `player.setLoop` / `PlayerState.loop`

- **请求方**：marks B23；playback 绘制区间（B22）
- **初判**：拟接受
- **替代方案（已考虑）**：插件监听位置再 `seek(A)`：位置事件粒度约 0.1 秒，回跳点抖动；回跳时与用户操作产生 `stale` 竞争；在 B 之后的一小段音画仍会播出。用户层面「精确循环」做不到。
- **约束**：`remaining` 若 mpv 不能直接给出（`ab-loop-count` 的剩余次数），就不提供该字段，只给 `count`；来源变化时清除（H13）。
- **理由**：〔宿主〕mpv `ab-loop-*`；〔共享〕循环状态要给另一个插件（playback）绘制。
- **核心体积影响**：小。

### R-M3 时间轴标记层 `plugin.timeline`

- **请求方**：marks B22、B23；playback 绘制
- **初判**：拟接受
- **替代方案（已考虑）**：marks 自己挂一个面板画标记：进度条在 playback 的底栏里，marks 画不进去；插件间不能通信。需求文档已比较过三种方案，C 是唯一不含业务的。
- **约束**：按需求文档的上限（单层 1000、标题 100 字符、来源变化清空、每插件一层）；`kind` 固定三种（核心不解释，只用于读取方配色）。TestHost 要新增 `host.timeline`。
- **理由**：〔共享〕典型的跨插件数据层，与 `osd.panel()`、播放序列标志同一模式。
- **核心体积影响**：小（按所有者分层的列表与推送）。

### 绑定

### R-K1 `plugin.bindings` 列表、改绑、恢复、变化通知

- **请求方**：hotkeys J01、J02
- **初判**：拟接受
- **替代方案（已考虑）**：插件看不到别的插件的命令，也写不了覆盖层。没有替代。
- **约束**：
  - 能力 `bindings.manage`（可以改别的插件的键，必须显式声明）。
  - `BindingInfo.labels` 由核心生成（与菜单右侧文字同一个函数），插件不重复实现按键显示规则。
  - 写入沿用 `ui_state.rs` 的去抖保存；`set` 的校验与 `commands.add` 绑定校验共用。
- **理由**：〔宿主〕绑定表与覆盖层都在核心，覆盖层本来就是为此预留的。
- **核心体积影响**：小到中（列表组装、shadowed 计算、orphans、推送）。

### R-K2 `ignoreDefaults` 忽略默认快捷键

- **请求方**：hotkeys J03
- **初判**：拟拒绝
- **替代方案**：hotkeys 插件用 R-K1 自己实现，并在自己的数据目录记录「哪些覆盖是我为忽略默认而写的」：
  ```ts
  const { commands } = await plugin.bindings.list();
  const written: BindingRef[] = [];
  for (const c of commands)
    if (!c.overridden && c.defaults.length && !(c.plugin === "host" && ["quit", "exit-fullscreen"].includes(c.command))) {
      await plugin.bindings.set(c, []); written.push(c);
    }
  saveJson("ignore-defaults.json", written);           // 关闭开关时只 reset 这些
  plugin.bindings.onChange(() => applyToNewCommands()); // 新插件加载后补写
  ```
- **代价**：新插件加载后、hotkeys 收到 `onChange` 补写之前，有一个很短的窗口默认键会生效；hotkeys 自己被停用时开关失效（但覆盖层仍在，键仍被忽略，状态与菜单勾选一致需要 hotkeys 启动时校对）。
- **反驳所需证据**：说明用户在什么具体场景下会碰到那个竞态窗口并造成可见问题。
- **理由**：单个插件的单个开关；它是参考播放器的冷门设置，四条条件只有〔宿主〕勉强沾边，而替代方案在用户层面几乎等价。
- **核心体积影响（若接受）**：小（裁决时一个分支加一个持久字段）。

### R-K3 可绑定输入 `Mouse:LeftClick`

- **请求方**：hotkeys J02
- **初判**：拟接受（与 H11 合并）
- **替代方案（已考虑）**：插件收不到视频区指针事件，也无法区分单击与双击。没有替代。
- **约束**：只在有绑定时延迟判定，无绑定时保持立即聚焦（H11）；双击判定不变。
- **理由**：〔宿主〕视频区指针事件；鼠标绑定表在核心。
- **核心体积影响**：小。

### 设置与外观

### R-S1 设置页挂载 `plugin.settings`

- **请求方**：preferences J04、H10；hotkeys J01、J02；plugins J07；visualizer H12（页面形态）
- **初判**：拟改写（接受，形态收窄）
- **替代方案**：需求文档的方案 C（各插件把设置入口放进「设置 ▶」菜单），零核心成本；代价是没有集中的选项窗口。方案 B 违反插件独立。
- **改写建议**：
  - `plugin.settings.page(file, { id, category, title, order? }): SettingsPage`，组件与 global 的推断规则与 `panels.mount` 相同，**不另加** `component` / `global` 选项（面板已有同样规则，文件导出多个时再沿用面板的处理方式）。
  - `SettingsPage` 就是 `View & Disposable`，去掉 `ready` 之外的 `visible`（页面是否在显示，对插件没有行为意义）。
  - `plugin.settings.show(pageId?)` 只能打开本插件的页面。
  - 分类表固定在核心，九类即可；`order` 只在同一分类内排序。
  - 容器窗口见 H7。
- **理由**：〔共享〕只有核心能把多个插件的 Slint 页面组进一个窗口；〔通用〕四个插件已经需要，以后几乎每个插件都会有设置页。
- **核心体积影响**：中（新挂载位、分类树、页面切换；复用 `composition.rs`）。

### R-S2 外观 `plugin.appearance`

- **请求方**：preferences H10
- **初判**：拟改写
- **替代方案**：插件无法改别的插件窗口的 `Theme`，没有替代；或者降低需求，不做配色切换（只有一套配色）。
- **改写建议**：
  - `plugin.appearance: { readonly theme: string; onChange(listener); set(theme: string): Promise<void> }`，`set` 需 `ui.theme`。
  - 去掉 `themes` 列表与 `preview` 元数据：配色表在核心，固定两套 id（`vestra`、`classic`），写进参考文档；preferences 的预览块用自己的 Slint 画（预览只是三块颜色）。以后真有可扩展配色再加列表。
  - 去掉独立的 `corners`：圆角跟随配色，不单独可调（降低需求）。
  - 选择由核心持久化（它影响宿主自己的界面，属于核心状态）。
- **理由**：〔共享〕Theme 是所有视图共享的全局；〔宿主〕宿主界面也要跟着变。
- **核心体积影响**：小（API），实际成本在 H6（中）。

### 插件管理

### R-G1 `packages.list` / `onChange`

- **请求方**：plugins J07；info I09
- **初判**：拟改写
- **替代方案**：插件之间不能互相查询运行状态；数据目录可以 `fs.readdir` 看到哪些 id 有数据，但看不到运行状态与安装记录。没有替代。
- **改写建议**：
  - `PluginInfo` 去掉 `dataDirectory`（规则固定为 `<数据根>/<id>`，插件可自己拼出；`VESTRA_PLUGIN_DATA` 改根目录时也由同一规则推出，参考文档写明）与 `self`（调用者比较 `info.id === plugin.id` 即可）。
  - **`list` 与 `onChange` 不需要能力**（只读公开信息），这样 info I09 不必为了显示列表去声明能安装、卸载插件的 `plugins.manage`；写操作（G2、G3、已有的 install / rollback）仍要 `plugins.manage`。
  - 合并 W6 的 `AppInfo.plugins`（见 R-W6）。
- **理由**：〔宿主〕运行状态与安装库在核心；〔共享〕两个插件读同一份列表。
- **核心体积影响**：小（宿主已有 `packages.installed` 线上方法与运行表）。

### R-G2 `packages.setEnabled`

- **请求方**：plugins J07
- **初判**：拟接受（与 H10 合并）
- **替代方案（已考虑）**：插件无法撤销别的插件的宿主资源。没有替代。
- **约束**：停用自己 `invalid`；试载 `unsupported`；开发加载只在本次运行有效；已安装的停用集合跨启动保存。
- **理由**：〔宿主〕；〔核心职责〕插件生命周期本来就属于核心（宿主内部已有 `disable(owner)`）。
- **核心体积影响**：小到中（对外接口 + 停用集合持久化 + 启动时跳过）。

### R-G3 `packages.uninstall`

- **请求方**：plugins J07
- **初判**：拟接受（与 H10 合并）
- **替代方案（已考虑）**：插件直接删安装库目录：绕过 `package_store.rs` 的日志式提交，会破坏库的一致性。不可行。
- **约束**：只对已安装插件；`removeData` 由核心删除 `plugin-data/<id>/`（不让一个插件去删另一个插件的数据目录）；覆盖层里的绑定保留为 orphan。
- **理由**：〔宿主〕安装库只有核心是写入者；〔核心职责〕。
- **核心体积影响**：小（store 增加删除记录的日志操作 + gc 已有）。

### 宿主行为改动

### R-H1 基础播放动作移交核心

- **请求方**：playback（全章）；hotkeys J01
- **初判**：拟接受（第 2 轮，自上而下流程）
- **替代方案（已考虑）**：保持基础动作在 playback：卸载或替换 playback 后 Space、←/→ 等全部失效，快捷键设置插件无从绑定基础操作。与「默认插件可替换」的原则冲突。
- **约束**：
  - 14 个动作与需求文档表一致，菜单路径、order、默认绑定、提示文字不变；行为从 `controller.ts` 原样搬过来，菜单快照回归。
  - **`rate-10`（速度 ▶ 正常，Z）一起移交**，即 `rate-normal`：它和 X / C 是同一组基础键，留在 playback 会让「减速、加速」在核心、「复位」在插件，卸载 playback 后就复位不了。
  - 覆盖层里 `vestra.playback/<旧 id>` 的绑定在升级时一次性映射到 `host/<新 id>`。
  - 按 §5 自上而下：维护者先做核心动作与 P3、P4，再由 playback 作者迁移；切换前旧命令继续可用。
- **理由**：〔核心职责〕。
- **核心体积影响**：中（14 个动作、累加与钳制逻辑、OSD 文案、覆盖层迁移）。这是第 2 轮最大的一项。

### R-H2 C-SIZE：渲染尺寸跟随视频区

- **请求方**：video E11、E13；window H07
- **初判**：拟接受（第 4 轮）
- **替代方案**：在 960×540 固定缓冲上只支持「保持宽高比」；「不保持」「填满」与放大画质不做。代价是 V9 整组功能的一半结果是错的。
- **约束**：渲染尺寸按视频区物理像素（或至少比例）跟随，窗口拖动时要有节流，避免每像素重建渲染上下文；软件渲染下 4K 视频区的 CPU 成本要实测。
- **理由**：〔宿主〕渲染是宿主独有；同时改善所有画面的清晰度（现在全屏也是 960×540 放大），不只服务单个插件。
- **核心体积影响**：中到大（渲染缓冲重建、帧缓冲共享、与截图路径隔离）。

### R-H3 C-GPU：libmpv 改 GPU 渲染

- **请求方**：video E16、E17、E18（运动补偿）；window H07（可选的零拷贝）
- **初判**：拟接受（第 4 轮；开工前单独出设计，可能需要按 §7 请用户确认范围）
- **替代方案**：保持软件渲染，E16、E17、运动补偿三项不做（插件里写死禁用原因）。产品其它部分不受影响。
- **约束**：Safe Rust（第一方禁止 `unsafe`）下接 libmpv OpenGL 渲染接口并嵌入 Slint，需要先确认可用的安全封装（rsmpv 的 render 特性、Slint 的 OpenGL 底层接口）；做不到时这是「既定技术路线满足不了关键场景」，按 §7 升级。截图与离线导出路径不受影响（仍是离线 ffmpeg）。
- **理由**：〔宿主〕渲染；它同时是硬件解码（`hwdec`）、更高质量缩放的前提，长期属于核心演进方向（技术选型 §6 已列为待设计）。
- **核心体积影响**：大，且有平台与安全约束风险。

### R-H4 绑定语法增加小键盘按键名

- **请求方**：video E13
- **初判**：拟接受
- **替代方案**：只用备用键（Alt+Plus、Alt+方向键等），不支持小键盘。代价：外接键盘用户用不了参考播放器习惯的小键盘布局，且现在 `"5"` 与小键盘 5 的匹配关系不确定。
- **约束**：宿主已在 winit 层拿到 `PhysicalKey`（`main.rs` 的 `KeyboardInput`），把 `Numpad*` 物理键映射为独立输入名；主键盘数字不匹配 `Numpad*`，`Numpad*` 也不匹配主键盘数字。与 H5 同一处改动。
- **理由**：〔核心职责〕绑定语法属于核心；〔通用〕任何插件都可以用小键盘名。
- **核心体积影响**：小。

### R-H5 macOS Option 键识别

- **请求方**：所有含 Alt 的默认绑定（多个插件）
- **初判**：拟接受（按缺陷处理，§4.2 第 1 条）
- **替代方案**：插件改用不含 Alt 的默认键：大量参考播放器默认键无法沿用，各插件各自找替代键还会互相冲突。
- **理由**：这是**现有契约的缺陷**：参考文档 §4.1 允许 `Alt+…` 绑定，而 `from_slint_key` 按 `event.text` 识别，Option+键在 macOS 上打出别的字符或死键，绑定根本不触发。按缺陷修，不需要作者论证。修法：识别时并入 winit `PhysicalKey`（与 H4 同一处）；同时核实 `Ctrl+[` 以 `\u{1b}` 到达被误认成 `Ctrl+Escape` 的问题。
- **核心体积影响**：小。

### R-H6 Theme 改为宿主可设

- **请求方**：preferences H10
- **初判**：拟改写（随 R-S2 收窄）
- **改写建议**：只把颜色 token 改为 `in property`；`radius-*` 保持常量（R-S2 已去掉独立圆角，参考播放器配色也沿用 Vestra 圆角，降低需求）。宿主在每个窗口、面板、菜单实例创建时和配色变化时下发；两套配色表放在宿主。
- **替代方案**：不做配色切换。
- **理由**：〔共享〕Theme 被所有视图共享。
- **核心体积影响**：中（所有视图实例的下发路径，含面板重建与浮动窗口）。

### R-H7 选项容器窗口

- **请求方**：preferences J04
- **初判**：拟改写（随 R-S1）
- **改写建议**：容器只有「分类树 + 页面区 + 关闭」，不做页面内滚动以外的布局能力；`settings.lastPage` 由核心记住。页面宽度固定 440px，作者按此设计。
- **替代方案**：方案 C（菜单入口）。
- **理由**：〔共享〕。
- **核心体积影响**：中（与 R-S1 同一份成本，不重复计）。

### R-H8 画中画帧输出

- **请求方**：window H07
- **初判**：拟接受（第 4 轮，随 W5）
- **替代方案**：见 R-W5（迷你尺寸形态）。
- **约束**：`AVSampleBufferDisplayLayer` / `AVPictureInPictureController` 需要 `objc2-av-kit`、`objc2-core-media` 等绑定，先确认在 Safe Rust 下可用（不行则按 §7 升级）；渲染驱动不能依赖主窗口绘制回调；帧时间戳映射到 `CMTimebase`。在 C-SIZE 之后做，以便浮窗按视频比例确定渲染尺寸。
- **理由**：〔宿主〕。
- **核心体积影响**：大。

### R-H9 生成流音频跟随输出设备

- **请求方**：audio G16
- **初判**：拟改写（分期）
- **替代方案**：降低需求：第一期 G16 只作用于 mpv 播放的媒体，插件生成的流（NES）仍走系统默认设备；菜单与参考文档写明这一限制。
- **改写建议**：第 2 轮只做 A2–A4（mpv 路径）；cpal 路径（`producer_audio.rs`）按设备 id 打开、断开后切回，放到第 3 轮再做。理由是 cpal 设备 id 与 mpv 的 CoreAudio 设备名要先确认能一一对应，不能确认时这项会变成两套设备列表。
- **理由**：〔宿主〕；同一个设置要覆盖所有输出路径才一致，所以最终应做，但不必和 A3 同批。
- **核心体积影响**：小到中。

### R-H10 停用状态持久化与对外停用

- **请求方**：plugins J07
- **初判**：拟接受（与 R-G2、R-G3 合并）
- **约束**：停用集合存在安装库的 catalog（与安装记录同一个事务边界），不放在 `ui-state.json`；开发加载的插件不写入。
- **理由**：〔核心职责〕插件生命周期。
- **核心体积影响**：小到中（计入 G2、G3）。

### R-H11 单击判定

- **请求方**：hotkeys J02
- **初判**：拟接受（与 R-K3 合并）
- **约束**：只在 `Mouse:LeftClick` 有绑定时延迟判定（系统双击间隔、移动 ≤ 4px），与现有 `has_wheel` 的做法相同；无绑定时保持立即聚焦。
- **理由**：〔宿主〕。
- **核心体积影响**：小。

### R-H12 ASS 覆盖策略可设

- **请求方**：subtitles D17
- **初判**：拟接受（与 R-T7 的 `assOverride` 合并）
- **约束**：默认值从 `force` 改为 `keep`（`scale`）是**默认行为变化**（§4.2 第 3 条）：现有内嵌 ASS 轨的显示会变。发布时写进变更说明，并在 GUI 回归里覆盖「内嵌 ASS + 插件样式」场景。
- **理由**：〔宿主〕mpv 全局属性，现在写死在宿主启动参数里。
- **核心体积影响**：极小。

### R-H13 A-B 循环跨文件复位

- **请求方**：marks B23
- **初判**：拟接受（与 R-M2 合并）
- **理由**：〔宿主〕mpv `ab-loop-*` 默认跨文件保留，只有宿主能在来源变化时清除；这是 M2 契约的一部分，不是独立功能。
- **核心体积影响**：极小。

---

## 合并议题

按 §4.2 第 5 条，以下请求指向同一能力，合并设计、一次定稿，不逐条实现：

1. **步进与基础动作**：R-P1 + R-P2（合成 `player.step(unit, direction)`）+ R-P3 + R-P4 + R-H1。基础动作要调用逐帧，P3/P4 是 H1 的组成部分；按 §5 自上而下流程做，playback 作者迁移。
2. **播放统计扩展**：R-P6 + R-P7 + R-P8，统一为 `PlaybackStats` 的透传字段，按需查询，不进 PlayerState。
3. **音频输出设备**：R-A2 + R-A3 + R-A4 + R-H9，一个 `audioOutput` 设计，分两期（mpv 路径 / 生成流路径）。
4. **次字幕与字幕载入**：R-T3 + R-T4（`select` 选项）+ R-T6 + R-T7（`secondaryPosition`）+ R-H12。`select` 的 `role`、`load` 的 `select`、图形轨校验、次字幕位置是同一组状态，一次设计。
5. **画面处理节点**：R-V1–R-V8 一次评审节点清单与链顺序，收窄后为 `hue`、`sharpen`、`denoise`、`deblock`、`deband`、`transform`、`deinterlace`、`reframe`、`frameBlend` 九种（原请求 14 种）。
6. **显示层与渲染**：R-V9 + R-V10 + R-H2 + R-H3 + R-P8，显示层、GPU 渲染选项的句柄形态与所有权在 C-SIZE / C-GPU 之后统一定。
7. **窗口控制**：R-W1–R-W5 + R-H8，一个 `plugin.window` 模块与能力 `window.control`；画中画部分（W5、H8、W1 的 `pictureInPicture`）拆到第 4 轮。
8. **插件与应用信息**：R-W6 + R-G1 + R-G2 + R-G3 + R-H10。插件列表只有一份（`packages.list`，只读不需能力）；`plugin.app` 只剩版本信息。
9. **时间轴与循环**：R-M2 + R-M3 + R-H13（以及 R-M1 的查询），playback 绘制时读 `timeline.marks` 与 `PlayerState.loop` 两个来源。
10. **绑定**：R-K1 + R-K3 + R-H4 + R-H5 + R-H11，全部在 `bindings.rs` 一处；H5 作为缺陷先修。
11. **设置与外观**：R-S1 + R-S2 + R-H6 + R-H7，设置容器与主题下发都要遍历所有视图实例，共用一套「向全部视图下发」机制。
12. **视频区挂载与图片**：R-U1 + R-U3（以及 R-U2 的视频区落点）。背景层、拖放落点、运行时图片都落在视频区与面板组合机制上。

### 建议提升为通用核心能力的候选

按 §4.3 第 4 条第四款（多个插件需要或是通用基础能力），下列能力目前在需求文档里由多个插件**各自**用系统命令实现，或附录 A 只按单个功能提出。建议作为通用能力统一提供；「实现层」一列说明放在哪里最省核心：能在插件运行器（`src/runtime/runner.ts`，每个插件 Node 进程里）实现的，就不进 Rust 宿主，宿主体积不变，但行为统一、停用时由运行器统一清理。

| 候选 | 现在谁各自实现 | 建议形态 | 实现层 | 理由与代价 |
| --- | --- | --- | --- | --- |
| 剪贴板写入（文本、图片） | playback I08（`pbcopy`）、info I05 / I09（`pbcopy`）、capture F03（`osascript` 写图片） | `plugin.clipboard.writeText(text)`、`plugin.clipboard.writeImage(path)`；需求出现后再加读取 | 宿主（`objc2-app-kit` 加 `NSPasteboard` 特性） | 三个插件；`pbcopy` 的编码要靠 `LANG` 环境变量、`osascript` 写图片依赖脚本拼接，各插件写法不同，行为容易不一致。宿主已链接 AppKit，增量很小 |
| 运行时图片 | playback、visualizer | 即 R-U1 | 宿主 | 已列入，拟接受 |
| 设置页挂载 | preferences、hotkeys、plugins、visualizer | 即 R-S1 | 宿主 | 已列入，拟改写后接受 |
| 播放时阻止休眠 | playback B29（`caffeinate`） | 宿主行为：播放中（`ready && !paused`，含生成流）持有电源断言；插件只开关：`plugin.power.preventSleep(mode: "off" \| "playing")`，停用时撤销 | 宿主（启动 `/usr/bin/caffeinate -d -w <宿主 pid>` 子进程即可，无需 `unsafe` 的 IOKit 绑定） | 目前只有一个插件，但「播放时不让显示器睡眠」是所有播放器的基础行为，判断「正在播放」的依据（含模拟器流）在宿主；插件用 `caffeinate -w <插件 pid>` 时插件崩溃重启会留下或丢失断言。优先级低于前三项 |
| 确认对话框 | playlist C18（替换列表）、marks B21（清空书签）、hotkeys J01 / J03（恢复默认、⌘Q、忽略默认）、plugins J07（停用、卸载） | `plugin.dialogs.confirm({ message, detail?, confirm, cancel? }): Promise<boolean>`，系统原生 NSAlert；可选一个复选框（J07「同时删除数据」） | 宿主（已依赖的 `rfd` 有 `MessageDialog`） | 四个插件各自画 Slint 确认窗，按钮顺序、默认键、Esc 行为会不一致；与现有 `dialogs.*` 同一模块，增量小 |
| 在 Finder 中显示 / 打开文件夹 | playlist C23（`open -R`）、capture F09、video E17、plugins J07（`open`） | `plugin.system.reveal(path)`、`plugin.system.openPath(path)` | 运行器（Node `execFile("/usr/bin/open")`） | 四个插件；实现只有一行，但路径校验（不存在时的提示）、不经 shell 的规则可以统一。不进宿主 |
| 外部工具调用（ffprobe、ffmpeg） | playlist、info I05、playback I07、visualizer J11（以及 R-P2、R-T5 的替代方案） | `plugin.tools.run("ffprobe" \| "ffmpeg", args, { timeout, signal, maxOutput })`：统一按 `VESTRA_FFPROBE` / `VESTRA_FFMPEG` / PATH 定位、不经 shell、超时、插件停用时结束进程、找不到时统一的 `unavailable` | 运行器 | 附录 B.4 已规定了统一规则，却要每个插件各自实现一遍；放进运行器后规则只有一份。不进宿主 |
| 插件设置文件原子读写 | 几乎所有插件（`settings.json`、`style.json`、`marks.json` 等，需求文档 45 处） | `plugin.storage.json<T>(name, { defaults, validate? })` → `{ value; save(next) }`：先写临时文件再改名、读坏时用默认值、带版本号 | 运行器 | 参考文档 §3.4 的规则（临时文件 + 改名、读坏用默认）每个插件都在重复写。纯 Node 实现，不进宿主 |
| 输入链接小窗（UI-DLG-02） | playback A02、playlist「添加链接…」 | 不做 API，在 `@vestra/widgets.slint` 里提供一个可复用的输入行组件 | 宿主 UI 库（Slint 组件，不是协议） | 两个插件按同一规格各写一份界面；放进控件库即可统一外观，不增加协议 |

不建议提升的：`sips` 格式转换（只有 capture）、`system_profiler` / `sw_vers` / `sysctl`（只有 info）、钥匙串（只有 subtitle-online）、`fs.watch`（只有 subtitles）。它们都是单个插件的特殊需求，留在插件里。

---

## 初步排期

只列拟接受、拟改写的条目（改写以改写后的形态排期）；拟拒绝的 6 条（R-P5、R-T5、R-V11、R-U4、R-U5、R-K2）不排期，等第 2 步的证据。

**第 2 轮：播放、轨道、处理节点、基础动作移交**

- 基础动作：R-H1、R-P3、R-P4、R-P1 + R-P2（`player.step`）
- 播放信息：R-P6 + R-P7（`PlaybackStats` 扩展，不含 `transfer`）
- 轨道与字幕：R-T1、R-T2、R-T3、R-T4、R-T6、R-T7、R-T8、R-H12
- 处理节点：R-V1–R-V8（收窄后的九种）、R-A1、R-A5
- 音频输出第一期：R-A2 + R-A3 + R-A4（mpv 路径）
- 时间轴：R-M1、R-M2、R-M3、R-H13
- 缺陷修复：R-H5（可提前，与 R-H4 同一处改动一起做）

**第 3 轮：界面、窗口、设置、绑定**

- 绑定：R-K1、R-K3、R-H11、R-H4
- 窗口（不含画中画）：R-W1（不含 `pictureInPicture`）、R-W2、R-W3、R-W4
- 应用与插件管理：R-W6、R-G1、R-G2、R-G3、R-H10
- 设置与外观：R-S1、R-H7、R-S2、R-H6
- 视频区与图片：R-U1、R-U2、R-U3
- 音频输出第二期：R-H9（生成流跟随设备）
- 通用能力候选中被采纳的项（剪贴板、确认对话框、运行器侧的 `tools` / `storage` / `system`）

**第 4 轮：C-SIZE、C-GPU、画中画**

- R-H2（C-SIZE），随后 R-V9（显示层）
- R-H3（C-GPU），随后 R-V10（渲染选项）与 R-P8（`transfer`）
- R-W5 + R-H8（画中画）与 R-W1 的 `pictureInPicture` 字段

统计：共 67 条（附录 A 接口 54 条 + 宿主行为 13 条）；拟拒绝 6、拟改写 30、拟接受 31。

---

## 第 2 步：作者回应

回应人：各插件作者代表（playback、playlist、subtitles、subtitle-online、video、audio、capture、info、window、history、marks、hotkeys、preferences、plugins、visualizer）。依据 §4.3 第 3 条：每条最多反驳一次，反驳必须带证据。替代方案在用户层面可行的，一律接受。

实验环境：MacBook Air（Mac14,2，8 GB 内存，内置 SSD），macOS 26.2，mpv 0.41.0，FFmpeg 9.0.2。实验脚本与产物在会话 scratchpad 的 `neg/` 目录（`gap.py`、`gap2.py`、`tl.lua`、`small.mkv`、`big.mkv` 等），不进仓库。本机没有回环录音设备，所以 R-P5 测的是事件时间，不是录音波形（见该条说明）。

### 结论一览

| 条目 | 初判 | 作者回应 | 要点 |
| --- | --- | --- | --- |
| R-P5 | 拟拒绝 | 接受 | 实测切换间隙低于维护者给的 150 ms 门槛；B26 降级为「快速推进」。附一条宿主改进建议（不是新 API） |
| R-T5 | 拟拒绝 | 接受 | 插件内抽取与宿主现有实现耗时相同（都要扫全文件）；编号对应在样例里成立。附一个字段补充，挂在 R-T6 上 |
| R-V11 | 拟拒绝 | 接受 | `sips` 实测 1080p 转 JPEG 0.05 s、转 BMP 0.03 s |
| R-U4 | 拟拒绝 | 接受 | 按维护者建议的「画在面板内」做不到，但宿主组合不裁切，提示可以画到面板外面。请求把「不裁切」写进参考文档 |
| R-U5 | 拟拒绝 | 接受 | `S` 不绑 |
| R-K2 | 拟拒绝 | 接受 | 竞态只在新装插件时出现，窗口是毫秒级；核心布尔值同样解决不了卸载 hotkeys 后的遗留问题 |
| 拟改写 30 条 | 拟改写 | 全部接受 | 个别条目写明作者侧代价，见下文 |
| R-T6、R-U1 | 拟接受 | 补充 | `SubtitleTrack.streamIndex`；`ImageRef` 的释放规则 |

**本步没有反驳。** 共 36 条（拒绝 6 + 改写 30）全部接受。

### 拟拒绝条目

#### R-P5 `player.preload` 无缝播放：接受

维护者要的证据是在当前宿主上测两段相接 flac 的切换间隙。测量方法与结果如下。

**方法**：用 mpv 0.41.0 复现宿主的推进路径：`keep-open=yes`，加载选项与宿主 `source_backend.rs::load_with_options` 对本地文件的一致（`demuxer=lavf,demuxer-lavf-probe-info=yes,rebase-start-time=yes`）。观察到 `eof-reached` 变为真时立即 `loadfile <下一首> replace`，计时到下一首的 `playback-restart` 事件为止。测试音源是两段 6 秒、440 Hz 正弦波 flac。音量设为 0 并静音，避免发出声音。

| 场景 | 次数 | eof → 下一首开始（ms） |
| --- | --- | --- |
| `ao=null`，同为 44.1 kHz flac | 15 | 中位 3.7，范围 2.4–8.4 |
| `ao=coreaudio`，同为 44.1 kHz flac | 15 | 中位 9.0，范围 4.0–11.3 |
| `ao=coreaudio`，44.1 kHz → mp3 44.1 kHz | 10 | 中位 9.8，范围 4.6–14.5 |
| `ao=coreaudio`，44.1 kHz → 48 kHz flac（音频输出要重建） | 10 | 中位 58.4，范围 56.0–60.4 |

**宿主还会再加一段延迟**（读代码得出，未在 GUI 里测）：

- 自然结束只在状态快照里检测：`source_backend.rs` 第 442 行附近读 `eof-reached`，而这段代码只在 `backend.rs` 第 368 行的 100 ms 周期里执行。所以 `ended` 会晚 0–100 ms 才到 playlist 的 `media.onEnd`。
- 之后是插件 RPC 往返、`File::open` 与 fd 租约，再到 `loadfile`。这几步估计在几毫秒量级。

合计：同格式切换约 5–115 ms（平均约 60 ms），采样率变化时约 60–165 ms。只有「采样率变化，又碰上周期末尾」这种最坏组合会超过 150 ms。按维护者给的门槛，证据不支持本轮引入 preload，所以接受。

**作者侧代价与后续**：

1. B26 降级为「快速推进」。菜单名改为「播放列表 ▶ 快速推进到下一个」，验收标准从「录音波形间隙 < 20 ms」改为「同格式切换 < 150 ms」。需求文档由 playlist 作者改。要说明的是：连续波形上即使断 5–10 ms，也能听到一声咔嗒，所以现场专辑、古典乐的「真正无缝」这次放弃了。以后要再提，会带录音波形证据（需要协调者在有回环设备的机器上测）。
2. playlist 按替代方案 1 做：提前算好下一项，提前 `fs.stat`，`onEnd` 回调里只调一次 `open`。
3. **建议（不是 API 请求，可按 §4.2 第 1 条另议）**：来源阶段变化（`ready → ended`）时立即推送一次 `source.state`，不再等 100 ms 周期。这样能去掉上面 0–100 ms 的大头。改动是在 `backend.rs` 的事件循环里，阶段变化后马上调一次 `snapshot` 并广播。所有用 `onEnd` 的插件（playlist、history、marks）都受益，不改契约。维护者认为不值得做也不影响本条结论。

#### R-T5 `subtitles.cues`：接受

**耗时实测**：在 1 小时 MKV 里封装两条字幕轨（subrip、ass，各 1800 句）和一个字体附件，用维护者给的命令抽取（`-map 0:s:N -f srt pipe:1`），每种跑 3 次：

| 文件 | 大小 | 抽 subrip | 抽 ass |
| --- | --- | --- | --- |
| `small.mkv`（1.5 Mbps，3600 s） | 0.68 GB | 0.15–0.44 s（首次 0.44） | 0.14 s |
| `big.mkv`（7.5 Mbps，4500 s） | 4.25 GB | 2.61–2.66 s | 2.61–2.76 s |

4 GB 文件约 2.6 s，主要是读盘（8 GB 内存的机器缓存不下整个文件，约 1.6 GB/s）。宿主现有的 `native_subtitle.rs::extract` 也是起一个 ffmpeg 扫全文件（`-threads 1`，还更慢一点），所以放进核心**不会更快**。性能不构成进核心的理由。

**编号对应**：`SubtitleTrack.embeddedId` 是 mpv 的分类型轨道号（`subtitle_backend.rs` 第 275 行取的是 `id`），不是容器流号。在上面的样例里（视频、音频、subrip、ass、附件，外加一条 `--sub-file` 外部轨），mpv 给出的 `sub id=1/2` 对应 `ff-index=2/3`，即 `0:s:0/1`；外部轨排在后面（`id=3`、`external=true`）。所以「`embeddedId − 1` = 第 N 条字幕流」在样例里成立，没找到反例。

**作者侧代价**：

- 超时不能固定 15 s。4 GB 文件在 SSD 上要 2.6 s；在 USB 机械盘（约 100–150 MB/s）上估计 30–40 s。subtitles 按文件大小定超时（约 1 s/100 MB，下限 15 s），并保留需求文档里「正在读取字幕…」的提示。
- 网络来源的内嵌字幕不支持按句跳转，命令禁用并写明原因（接受）。
- 内嵌 ASS 轨也能抽成 SRT（实测可以），所以 B11 对 ASS 一样可用。宿主现有的 extract 只支持 subrip（`subtitle_backend.rs` 第 666 行），放进核心还要另外扩展。

补充：「`embeddedId − 1` = 第 N 条字幕流」依赖的是 mpv 没写进文档的排序。宿主自己抽取时用的是 `ff-index`（`subtitle_backend.rs` 第 290 行与第 675 行），`media_tracks.rs` 的注释也写明 ff-index 只在 lavf 下可靠。为了不让插件依赖这个排序，请求在 R-T6 里顺带开放 `streamIndex`，见下文「拟接受条目的补充」。这不是反驳 R-T5。

#### R-V11 `Snapshot.export` JPEG：接受

实测：1920×1080 PNG（336 KB）用 `sips -s format jpeg -s formatOptions 85` 转换 0.05 s（得到 163 KB），转 BMP 0.03 s。比维护者估的 100 ms 还快，用户感觉不到。capture 按替代方案实现，临时 PNG 放在 `dataDirectory/tmp/`，转换完删除。

#### R-U4 面板 `PopupWindow` 越出边界：接受（附一条文档请求）

维护者建议的「画在面板内部」**在几何上做不到**：

- 底栏 `PlaybackBar` 固定 `min-height`、`max-height` 都是 62px（`bar.slint` 第 214–215 行），进度线在 `y = 6px`（第 226 行）。线的上方只有 6px，而 I06 要求提示底边在进度线上方 4px、画在视频区上面。
- I07 的弹框是 160×90 的缩略图加时间文字，至少约 112px 高，比整个底栏还高。
- 「悬停时临时加高底栏」会挤压视频区：底栏槽位是纵向布局里 `vertical-stretch: 0` 的一行（`composition.rs` 第 171 行）。每次悬停整个画面都会跳一下，不可接受。

不过不需要 `PopupWindow`：宿主组合**没有裁切**。`composition.rs` 第 171–175 行生成的底栏外层 `Rectangle` 和 `FocusScope` 都没有 `clip`，而且全屏时底栏本来就以 `y: -self.height` 画在高 0 的外层之外、覆盖在视频区上。也就是说，组件内 `y` 为负的元素会直接画在视频区上面。playback 就按这个方式做 I06、I07：提示框放在 `seek-row` 内，`y` 取负值，并夹在底栏宽度内。

**请求（文档，不是 API）**：在参考文档 §6.3 写明「底栏面板的内容可以画到槽位上方的视频区，宿主不裁切；侧栏在其上层」。这样以后改组合时（比如 R-U3 背景层、R-S1 设置窗口复用 `composition.rs`）不会悄悄加上 `clip`，把 I06、I07 弄坏。如果维护者不想把它写成契约，playback 作者接受风险，并在 playback 的 GUI 回归里加一条悬停截图检查。

#### R-U5 `commands.popup`：接受

`S` 不绑；「使用着色器」开关只保留 Shift+P。video 需求文档相应删除 `S` 键说明。

#### R-K2 `ignoreDefaults`：接受

逐项核对维护者给的代价：

- **竞态窗口**：覆盖层由核心跨启动保存，所以启动时已经存在的命令一直是「被覆盖」的状态，没有窗口。只有在开关打开期间**新装或新启用**插件时，它的默认键在 hotkeys 收到 `onChange` 补写前会短暂生效。这是一次 RPC 往返，毫秒级；用户在这段时间里按到那个新键的场景，我们找不到。
- **hotkeys 停用或卸载**：用替代方案时，写下的空覆盖会留在覆盖层，默认键继续失效，而且没有界面可以恢复（只有重新启用 hotkeys）。但需求文档原方案（核心持久化一个布尔值）同样会留下 `ignoreDefaultBindings: true`，问题一样。所以这不能作为反驳理由。
- **作者侧处理**：hotkeys 在 `ignore-defaults.json` 里记录自己写入的条目。用户在开关打开期间改过的条目从记录里移除，关闭开关时只 `reset` 剩下的条目。J01 列表里「被忽略」的显示由 hotkeys 根据记录得出，不依赖核心区分。

### 拟改写条目

全部接受。下面只写作者侧要做的事、代价或疑问；没写的就是照改写建议做。

**播放器**

- **R-P1 / R-P2 `player.step(unit, direction)`**：接受。去掉 `can.frameStep` 后，playback 在 `sourceId` 变化、进入 `ready` 后查一次 `stats()`，缓存「有没有视频」，用于命令的 `enabled`。生成流（`state.kind === "stream"`）由 playback 自己禁用，免得点了才得到 `unsupported`。按住 F 时丢弃在途重复按键的逻辑由 playback 实现。
- **R-P6 `videoSize` 并入 stats**：接受。window 用同样的「ready 后查一次 `stats()` 缓存」来算 `enabled`，纯音频时菜单仍能预先灰掉，用户看不出差别。
- **R-P7 `PlaybackStats` 扩展**：接受，包括暂不加 `hwdec`。info I05 的「硬件解码」一行暂时固定显示「未使用」，等 C-GPU。
- **R-P8 `transfer`**：接受，第 4 轮。

**轨道与字幕**

- **R-T2 `selectAudioTrack`**：接受。
- **R-T3 次字幕（`select` 的 `role`）**：接受。代价：两个角色共用一个 `trackConfirmation`。subtitles 会把主、次字幕的选择串行化（等上一次确认后再提交下一次），这样确认状态总能对应到正确的菜单项。
- **R-T4 `load` 支持 ASS、标题与选择方式**：接受，包括不加 `"vtt"`。subtitles 与 subtitle-online 各自做 VTT 转 SRT（约 40 行）。两个插件不能共享运行时代码，就把同一份实现分别放进两个包里。
- **R-T7 样式字段**：接受，`box` 合并为一个数字字段。

**画面与渲染**

- **R-V2 只加 `sharpen`（unsharp，两种大小）**：接受。本轮不做 `unsharp` 与 `cas` 的对比截图，所以不就 `cas` 反驳。
- **R-V3 只加 `denoise`（hqdn3d）**：接受，三种降噪预设在 video 插件里定义参数。
- **R-V5 `transform`**：接受。
- **R-V6 `deinterlace` 固定 bwdif**：接受。E10 菜单删掉算法单选。
- **R-V7 `reframe`**：接受。
- **R-V9 `video.view()`**：接受：去掉 `scaleX` / `scaleY`，E13 删四条命令；改为单一拥有者；C-SIZE 之前不开放。
- **R-V10 `video.render()`**：接受。C-GPU 之前，E16、E17 和运动补偿的命令写死禁用原因。

**音频**

- **R-A2 / R-A3 / R-A4 输出设备**：接受，包括去掉 `devicesRevision`。audio 在「输出设备」子菜单所在的工作台窗口打开时每 5 秒轮询一次 `audioDevices()`，窗口关闭时停止。主菜单子菜单在 `sourceId` 或 `audioOutput` 变化时刷新。新插入的设备最多晚 5 秒出现，可以接受。
- **R-H9 生成流跟随设备（分期）**：接受。第一期 G16 菜单下方加一行说明「模拟器声音仍走系统默认设备」。

**窗口与应用**

- **R-W1 `window.state`**：接受，去掉 `onTop`。
- **R-W6 `plugin.app`**：接受（同步只读，不要 `onChange`）。

**界面与输入**

- **R-U2 拖放（无 `onDragOver`）**：接受。代价：长列表里拖放时看不到插入线。playlist 降级为「落在哪一行，就插到那一行之前；落在空白处，就追加到末尾」。核心给的是面板坐标，playlist 还要知道列表滚动位置才能换算到行。办法是在 Slint 里用 `changed viewport-y => { Bridge.scrolled(...) }` 回传滚动位置（侧栏已经在用 `changed` 处理器），不需要核心配合。
- **R-U3 背景层（去掉 `mode`）**：接受。visualizer 用 `phase` 与 `stats().video` 自己区分该画哪种内容。带封面的音频文件里，mpv 会把封面当作视频轨（`stats().video` 非空）。这时背景层不显示，用户看到的是 mpv 渲染的封面。这符合 J11 的预期，记一笔，免得以后被当成缺陷。

**时间轴**

- **R-M1 `player.chapters()`**：接受。marks 在 ready 后查一次并缓存，再写进自己的 timeline 层，供 playback 绘制。

**设置与外观**

- **R-S1 `settings.page`**：接受，440px 宽与需求文档一致。hotkeys J01 的三列表格按 440px 重新排布（命令 220、快捷键 140、状态 60）。
- **R-S2 `appearance`**：接受：去掉 `themes` 列表与 `corners`，固定 `vestra`、`classic` 两个 id。preferences 的预览块自己画。
- **R-H6 Theme 下发**：接受，圆角保持常量。
- **R-H7 选项容器窗口**：接受。

**插件管理**

- **R-G1 `packages.list`**：接受。去掉 `dataDirectory` 与 `self`；`list` 和 `onChange` 不需要能力，info I09 因此不必声明 `plugins.manage`。

### 拟接受条目的补充

- **R-T6 补一个字段 `SubtitleTrack.streamIndex: number | null`**（内嵌轨的容器流号，即 mpv `ff-index`；外部轨或未知时为 `null`）。理由：
  1. 宿主在同一次 `track-list` 读取里已经拿到它（`subtitle_backend.rs` 第 290 行），和 `codec` 一样只需开放，核心增量是一个字段。
  2. 宿主自己抽字幕时用的就是它（第 675 行），并且注明 ff-index 只在 lavf 下可靠。插件拿到同一个值，就能 `-map 0:<streamIndex>`，不用依赖「mpv 轨道号与字幕流同序」这个没写进文档的规则。R-T5 的替代方案因此变得可靠。
  3. 用它的不只 subtitles（B11 跳句、D18 导出内嵌字幕）：capture 导出时选「随快照冻结的字幕」做对照、info I05 显示流信息，都要把轨道对应到容器流。

  如果维护者认为 `ff-index` 的「lavf 才可靠」前提让这个字段语义不稳，可以规定：非 lavf 或网络来源一律为 `null`。这正好和 R-T5「网络来源不支持」的边界一致。
- **R-U1 `ui.image`**：请写明 `ImageRef` 什么时候释放。I07 悬停缩略图每个来源最多 200 张，指针移动时会频繁换引用。希望规则是：引用属于创建它的作用域；同一路径再次调用 `ui.image` 时旧引用失效（宿主丢弃旧的解码结果）；或者宿主按插件做 LRU，上限写进文档（例如每个插件 64 MB 解码内存）。不写清楚的话，长时间悬停可能让宿主内存一直涨。
- **R-H1 基础动作移交**：playback 作者确认按 §5 迁移。需要迁移的有底栏按钮（改用 `host.run`）和 `controller.ts` 里的累加、钳制逻辑；覆盖层旧 id 由核心映射。

### 对「通用核心能力候选」的表态

| 候选 | 会用的插件与用法 | 表态 |
| --- | --- | --- |
| 剪贴板写入 | playback I08（复制文件名、文件夹、路径，现为 `copy.ts` + `system.ts` 调 `pbcopy`）；info I05 / I09（复制属性文本）；capture F03（复制截图，现需 `osascript` 拼脚本） | **支持**。`writeImage(path)` 希望接受 PNG 与 JPEG；失败时返回统一的错误码，好让各插件的 OSD 文案一致 |
| 运行时图片 | 即 R-U1 | 见上条补充 |
| 设置页挂载 | 即 R-S1 | 已接受 |
| 播放时阻止休眠 | playback B29（现为 `sleep.ts`，94 行：有视频用 `caffeinate -d -i`，纯音频只用 `-i`，`-w <插件 pid>`） | **支持，优先级低**。补两点：(1) 核心要区分「有视频帧」与「纯音频」：前者阻止显示器睡眠，后者只阻止系统睡眠。所以 `mode` 不止 `"playing"`，或由核心自己按视频帧判断。(2) 维护者说的「插件崩溃会留下断言」不成立，因为 `-w` 跟的是插件进程，插件崩溃时断言随之结束；问题只是崩溃重启前那段时间里断言丢失。进核心的真正理由是「正在播放」（含生成流）的判断在宿主 |
| 确认对话框 | playlist C18（现为 `confirm.ts` + `confirm.slint` 自画）；marks B21；hotkeys J01 / J03；plugins J07（停用、卸载，带「同时删除数据」复选框） | **支持**。希望加一个可选的 `destructive: true`（NSAlert `hasDestructiveAction`，用于「卸载」「清空书签」），其余照维护者的形态。playlist 会删掉自画的确认窗 |
| 显示 / 打开文件夹 | playlist C23（现为 `finder.ts`，51 行）；capture F09；video E17；plugins J07 | **支持**，运行器实现即可 |
| 外部工具调用 | playlist（`probe.ts`、`media.ts`，ffprobe）；info I05；playback I07（ffmpeg 缩略图）；visualizer J11；**subtitles B11**（R-T5 被拒后改为插件内 ffmpeg 抽取） | **强烈支持**，是用的插件最多的一项。需要：`timeout` 由调用方给（R-T5 实测大文件要几十秒），`maxOutput` 至少 16 MB（字幕 SRT 可达 8 MB 量级），作用域释放时结束进程，找不到可执行文件时统一以 `unavailable` 失败（各插件菜单的「需要 ffmpeg」原因要一致） |
| 设置文件原子读写 | 几乎全部插件；仅 playback `settings.ts`（61 行）与 playlist `store.ts`（146 行）就有两份「临时文件 + 改名 + 读坏用默认值」的实现 | **支持**。希望 `save` 自带去抖（例如 300 ms 合并一次，playlist 拖动排序时连续写入），并且 `validate` 失败时把坏文件改名保留为 `.bad`，方便用户排查 |
| 输入链接小窗组件 | playback A02（现为 `open-url.slint` + `open-url.ts`，123 行）；playlist「添加链接…」 | **支持**，作为 `@vestra/widgets.slint` 组件即可，不需要协议 |

「不建议提升」的四项（`sips`、`system_profiler` 等、钥匙串、`fs.watch`）：capture、info、subtitle-online、subtitles 的作者都同意留在插件里。

### 统计

回应 36 条（拟拒绝 6、拟改写 30）：**接受 36，反驳 0**。拟接受条目补充 3 条（R-T6 加 `streamIndex`、R-U1 释放规则、R-H1 迁移确认）。另有两条不是 API 的请求：R-P5 的「阶段变化立即推送」建议，R-U4 的「底栏不裁切」写进参考文档。

---

## 第 3 步：裁决

裁决人：API 维护者。依据第 1 步回复、第 2 步作者回应（接受 36、反驳 0）与 §4.3 第 4 条（含第四款「通用基础能力」）。本步只定结论、最终形态与轮次，不实现。

相对第 1 步「初步排期」的调整：

- 第 2 轮按体积预算（见下文「核心体积预估」）砍掉了次字幕（R-T3 及其连带字段）、章节与循环（R-M1、R-M2、R-H13）、音频输出设备（R-A2–R-A4）、倍帧节点（R-V8），分别移到第 3a、4a 轮。
- 原第 3 轮过大，拆成 **3a**（输入、窗口、插件管理、音频输出、章节与循环、次字幕、确认框）和 **3b**（界面组合：设置页、主题、运行时图片、拖放、背景层、时间轴标记层）。
- 原第 4 轮拆成 **4a**（C-SIZE、显示层、画中画、倍帧）和 **4b**（C-GPU 与渲染选项）。4b 开工前要先出设计，并确认 Safe Rust 下有可用的渲染封装；做不到时按 §7 升级给用户。
- 音频输出设备的 mpv 路径与生成流路径（R-H9）放到同一轮（3a）一次交付，作者在 R-H9 回应里提到的「第一期说明行」不再需要。

### 3.1 逐条裁决

结论三种：**拒绝+替代**、**改写后接受**、**接受**。「最终形态」是一行签名或行为，细节以第 1 步对应小节的改写建议为准。

| 条目 | 结论 | 最终形态 | 轮次 |
| --- | --- | --- | --- |
| R-P1 逐帧 | 改写后接受 | `player.step(unit: "frame" \| "keyframe", direction: "backward" \| "forward", opts?: { ifCurrent?: PlayerState }): Promise<void>`；不加 `can.*`，不满足条件时 `unsupported` | 2 |
| R-P2 关键帧 | 改写后接受 | 并入 R-P1 的 `unit: "keyframe"`（mpv `seek ±0 relative+keyframes`） | 2 |
| R-P3 第 1 档步长 | 接受 | `player.setStepSeek(options: { seconds: number }): Promise<void>`（0.1–3600，默认 5；调用者停用后恢复） | 2 |
| R-P4 执行核心动作 | 接受 | `plugin.host.run(action: HostAction): Promise<void>`；`HostAction` 为 H1 的 14 个动作 id；禁用时 `unavailable`；需 `commands` | 2 |
| R-P5 无缝预读 | 拒绝+替代 | 不加 `preload`；playlist 用 `onEnd → open(next, { ifCurrent })`，B26 降级为「快速推进」；宿主改为阶段变化立即推送（见 3.2 第 1 条） | 2（宿主行为） |
| R-P6 显示尺寸 | 改写后接受 | 并入 R-P7：`PlaybackStats.video.displayWidth/displayHeight: number \| null`，不进 PlayerState | 2 |
| R-P7 统计扩展 | 改写后接受 | `PlaybackStats.video += { decoder; pixelFormat; colorspace; displayWidth; displayHeight; displayFps; bitrate }`、`audio += { decoder; format; channelLayout; bitrate }`、`avSyncMs`、`cacheSeconds`，全部 `\| null`；不加 `hwdec` | 2 |
| R-P8 传输特性 | 改写后接受 | `PlaybackStats.video.transfer: string \| null`（原样给 mpv `video-params/gamma`） | 4b |
| R-T1 外部音轨 | 接受 | `player.addAudioTrack(path: string, opts?: { select?: boolean; ifCurrent?: PlayerState }): Promise<AudioTrack>` | 2 |
| R-T2 选音轨 | 改写后接受 | `player.selectAudioTrack(id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>` | 2 |
| R-T3 次字幕 | 改写后接受 | `subtitles.select(track \| null, opts?: { role?: "primary" \| "secondary"; ifCurrent? })`；`SubtitlesState.secondary: SubtitleTrack \| null`；共用 `trackConfirmation` | 3a |
| R-T4 字幕载入 | 改写后接受 | `subtitles.load(bytes, opts?: { format?: "srt" \| "ass"; title?: string; select?: "primary" \| "none"; ifCurrent?; signal? })`；第 3a 轮 `select` 增加 `"secondary"`；不加 `"vtt"` | 2（`"secondary"` 3a） |
| R-T5 逐句时间表 | 拒绝+替代 | 不加 `cues`；subtitles 用 `plugin.tools.run("ffmpeg", ["-v","error","-nostdin","-i",src,"-map",`0:${track.streamIndex}`,"-f","srt","pipe:1"], { timeoutMs })`；网络来源禁用 | — |
| R-T6 轨道 codec | 接受（含补充） | `SubtitleTrack.codec: string \| null`、`text: boolean`、`streamIndex: number \| null` | 2 |
| R-T7 样式字段 | 改写后接受 | `SubtitleStyle += { bold?: boolean; shadow?: number /*0–4*/; box?: number \| null /*0–100*/; assOverride?: "keep" \| "force" }`；第 3a 轮加 `secondaryPosition?: "top" \| "aboveMain"` | 2（次字幕位置 3a） |
| R-T8 同步范围 | 接受 | `SubtitleStyle.offsetMs` 范围 −600 000–600 000 | 2 |
| R-V1 色相 | 接受 | `{ kind: "hue"; degrees: number }`（−180–180，运行时） | 2 |
| R-V2 锐化 | 改写后接受 | `{ kind: "sharpen"; amount: number /*0–2*/; size: "small" \| "large" }`（unsharp）；不加 `cas` | 2 |
| R-V3 降噪 | 改写后接受 | `{ kind: "denoise"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }`（hqdn3d，0–20）；不加 `temporalDenoise` | 2 |
| R-V4 去块/消色带 | 接受 | `{ kind: "deblock"; strength: "weak" \| "strong" }`、`{ kind: "deband"; threshold; range }` | 2 |
| R-V5 翻转/旋转 | 改写后接受 | `{ kind: "transform"; rotate: 0 \| 90 \| 180 \| 270; flipH: boolean; flipV: boolean }` | 2 |
| R-V6 反交错 | 改写后接受 | `{ kind: "deinterlace"; when: "interlaced" \| "all"; output: "frame" \| "field" }`（固定 bwdif） | 2 |
| R-V7 裁剪/加边 | 改写后接受 | `{ kind: "reframe"; mode: "crop" \| "pad"; aspect: number /*0.25–4*/; x: number; y: number /*−1–1*/ }` | 2 |
| R-V8 倍帧 | 接受（带性能门槛） | `{ kind: "frameBlend"; fps: number /*24–120*/ }`；与 C-SIZE 的渲染节奏改动同批验证 | 4a |
| R-V9 显示层 | 改写后接受 | `plugin.video.view(): VideoView`，`set(Partial<{ aspect: number \| null; fit: "keep" \| "stretch" \| "fill"; zoom; panX; panY }>)`，单一拥有者（第二个 `conflict`） | 4a |
| R-V10 渲染选项 | 改写后接受 | `plugin.video.render()`，字段同第 1 步；C-GPU 完成才发布，单一拥有者 | 4b |
| R-V11 JPEG 导出 | 拒绝+替代 | 导出 PNG 后 `sips` 转换（实测 0.05 s） | — |
| R-A1 声道 | 接受 | `{ kind: "channels"; mode: "stereo" \| "left" \| "right" \| "mono" \| "swap" }`；多声道由核心先下混 | 2 |
| R-A2 设备列表 | 改写后接受 | `player.audioDevices(): Promise<{ id: string; name: string }[]>` | 3a |
| R-A3 选择设备 | 改写后接受 | `player.setAudioDevice(id: string \| null): Promise<void>`（核心不跨启动保存） | 3a |
| R-A4 输出状态 | 改写后接受 | `PlayerState.audioOutput: { requested: string \| null; active: string \| null }`（无 `devicesRevision`） | 3a |
| R-A5 音频延迟范围 | 接受 | 音频 `delayMs` −10 000–10 000（10 ms 倍数），预算同步放宽 | 2 |
| R-W1 窗口状态 | 改写后接受 | `plugin.window.state: { fullscreen; videoArea: { width; height } }`、`onChange`；4a 增加 `pictureInPicture: { supported; active }`；能力 `window.control` | 3a（PiP 字段 4a） |
| R-W2 视频区尺寸 | 接受 | `window.setVideoArea(size: { width; height }): Promise<{ width; height; clamped: boolean }>` | 3a |
| R-W3 置顶 | 接受 | `window.setAlwaysOnTop(onTop: boolean): Promise<void>` | 3a |
| R-W4 指针隐藏 | 接受 | `window.setCursorAutoHide(mode: "fullscreen" \| "playing"): Promise<void>` | 3a |
| R-W5 画中画 | 接受 | `window.setPictureInPicture(on: boolean): Promise<void>` | 4a |
| R-W6 应用信息 | 改写后接受 | `plugin.app: { readonly version: string; build: string \| null; components: { mpv: string \| null; libav: Record<string, string>; slint: string } }`（同步只读、无需能力） | 3a |
| R-U1 运行时图片 | 接受（含补充） | `plugin.ui.image(path: string): ImageRef`；`PropValue = Json \| ImageRef`；释放规则见 3.2 第 5 条 | 3b |
| R-U2 拖放 | 改写后接受 | `plugin.drops.provide({ extensions: string[]; drop(e: { paths; modifiers }): unknown }): Disposable`；`Panel.onDrop(listener: (e: { paths; at: { x; y }; modifiers }) => unknown)`；能力 `ui.drop` | 3b |
| R-U3 背景层 | 改写后接受 | `panels.mount("backdrop", file)`；`panels.backdrop.state: { effective: boolean } \| null`、`onChange` | 3b |
| R-U4 弹出越界 | 拒绝+替代 | 不加接口；底栏内容可用负 `y` 画到视频区，宿主不裁切写进参考文档 §6.3（见 3.2 第 3 条） | 2（文档） |
| R-U5 弹出主菜单 | 拒绝+替代 | 不加接口；`S` 不绑 | — |
| R-M1 章节 | 改写后接受 | `player.chapters(): Promise<{ sourceId: number; chapters: { title: string \| null; start: number }[] }>` | 3a |
| R-M2 A-B 循环 | 接受 | `player.setLoop(loop: { a: number; b: number; count?: number } \| null, opts?: { ifCurrent? }): Promise<void>`；`PlayerState.loop: { a; b; count: number \| null } \| null`（mpv 给不出剩余次数就不提供 `remaining`） | 3a |
| R-M3 标记层 | 接受 | `plugin.timeline.marks`、`onChange`、`layer(): TimelineLayer`；`layer.set(marks: { time; kind: "chapter" \| "bookmark" \| "marker"; title? }[], opts?)`；能力 `timeline.marks` | 3b |
| R-K1 绑定管理 | 接受 | `plugin.bindings.list() / set(ref, inputs) / reset(ref?) / onChange(listener)`；能力 `bindings.manage` | 3a |
| R-K2 忽略默认键 | 拒绝+替代 | 不加接口；hotkeys 用 K1 写空覆盖并在 `ignore-defaults.json` 记录自己写过的条目 | — |
| R-K3 左键单击 | 接受 | 可绑定输入 `Mouse:LeftClick` | 3a |
| R-S1 设置页 | 改写后接受 | `plugin.settings.page(file, { id; category: SettingsCategory; title; order? }): View & Disposable & { ready }`；`plugin.settings.show(pageId?)`；能力 `ui.settings` | 3b |
| R-S2 外观 | 改写后接受 | `plugin.appearance: { readonly theme: "vestra" \| "classic"; onChange(listener); set(theme): Promise<void> }`；`set` 需 `ui.theme` | 3b |
| R-G1 插件列表 | 改写后接受 | `plugin.packages.list(): Promise<PluginInfo[]>`、`onChange`（不需能力）；`PluginInfo { id; version; origin; state; error; capabilities; directory; previousVersion }` | 3a |
| R-G2 停用启用 | 接受 | `packages.setEnabled(id: string, enabled: boolean): Promise<void>`（需 `plugins.manage`） | 3a |
| R-G3 卸载 | 接受 | `packages.uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>` | 3a |
| R-H1 基础动作移交 | 接受 | 核心注册 14 个动作（含 `rate-normal`）；覆盖层旧 id 一次性映射；按 §5 集中切换 | 2 |
| R-H2 C-SIZE | 接受 | 渲染尺寸跟随视频区（节流重建） | 4a |
| R-H3 C-GPU | 接受（设计门槛） | libmpv GPU 渲染嵌入 Slint；先出设计并确认 Safe Rust 封装，不行则 §7 升级 | 4b |
| R-H4 小键盘键名 | 接受 | `Numpad0`–`Numpad9`、`NumpadPlus`、`NumpadMinus`、`NumpadMultiply`、`NumpadDivide`、`NumpadEnter`、`NumpadDecimal` | 2（与 H5 同一处改动，提前） |
| R-H5 Option 键 | 接受（缺陷） | 识别时并入 winit `PhysicalKey`；修 `Ctrl+[` 误认 | 2 |
| R-H6 Theme 下发 | 改写后接受 | 颜色 token 改 `in property`，圆角保持常量；宿主向全部视图下发 | 3b |
| R-H7 选项窗口 | 改写后接受 | 分类树 + 页面区（440 宽）+ 关闭；记住 `settings.lastPage` | 3b |
| R-H8 画中画帧 | 接受 | sample buffer 路径；先确认 AVKit 绑定在 Safe Rust 下可用 | 4a |
| R-H9 生成流跟随设备 | 改写后接受 | 与 A2–A4 同轮交付（不再分期） | 3a |
| R-H10 停用持久化 | 接受 | 停用集合存安装库 catalog | 3a |
| R-H11 单击判定 | 接受 | 只在 `Mouse:LeftClick` 有绑定时延迟判定 | 3a |
| R-H12 ASS 覆盖 | 接受 | `assOverride` 默认 `keep`（`scale`）；默认行为变化写进变更说明 | 2 |
| R-H13 循环跨文件复位 | 接受 | 来源变化时核心清除 `ab-loop-*` | 3a |

### 3.2 对作者补充的表态

1. **R-P5「阶段变化立即推送」**：接受，第 2 轮做，按缺陷改进处理（不改契约）。`backend.rs` 事件循环在来源阶段变化（含 `ready → ended`）后立即发一次 `source.state`，不等 100 ms 周期；周期推送照旧。验收：TestHost 不变；宿主单测断言阶段变化后的首次广播不依赖周期计时。
2. **R-T5 超时**：同意由调用方给超时；`tools.run` 不设固定上限（见 3.3）。
3. **R-U4 底栏不裁切**：接受写成契约。参考文档 §6.3 增加一句：「底栏面板的内容可以画到槽位上方的视频区（负 `y`），宿主不裁切；侧栏与浮动窗口在其上层。」宿主在 `composition.rs` 的组合测试里加一条断言（底栏外层不带 `clip`），以后改组合时会被测试拦住。
4. **R-T6 `streamIndex`**：接受。取值规则：内嵌轨且宿主判定 `ff-index` 可靠（与 `media_tracks.rs` 现有的 lavf 条件相同）时给出容器流号，否则 `null`；外部轨与网络来源一律 `null`。
5. **R-U1 `ImageRef` 释放**：规则如下。
   - `ImageRef` 属于创建它的作用域，作用域释放后失效（写入失效引用以 `invalid` 失败）。
   - 宿主按（插件、路径、文件大小、修改时间）缓存解码结果，每个插件解码内存上限 64 MB，LRU 淘汰**当前没有被任何属性引用**的条目；被淘汰的引用仍然有效，下次使用时重新解码。
   - 同一路径文件内容变了，重新调用 `ui.image` 得到新引用；旧引用指向的旧解码结果在不再被引用后被淘汰。
   - 校验（格式、16 MB、4096×4096、路径范围）在宿主首次使用该引用时进行，失败作为属性写入错误交给 `onError`。
6. **R-H1 迁移确认**：收到。迁移方式写在第 2 轮任务单 §3.1。
7. **R-K2 hotkeys 的处理方式**：同意作者给出的记录与恢复规则。
8. **R-U2 滚动位置回传**、**R-U3 封面被 mpv 当作视频轨**：同意，后者记入 visualizer README 作为已知行为，不算缺陷。

### 3.3 通用核心能力候选的裁决

| 候选 | 裁决 | 最终签名 | 轮次 |
| --- | --- | --- | --- |
| 剪贴板 | **进宿主** | `plugin.clipboard.readText(): Promise<string \| null>`；`writeText(text: string): Promise<void>`；`writeImage(path: string): Promise<void>`（PNG/JPEG，≤ 16 MB）。能力 `clipboard`。失败码：`invalid`（格式、大小）、`unavailable`（无界面模式）、`failed`（系统拒绝） | 2 |
| 播放时阻止休眠 | **进宿主** | `plugin.power.preventSleep(mode: "off" \| "playing"): Promise<void>`（需 `playback.control`）。任一插件要求 `playing` 即生效，插件停用时撤销它的要求。宿主在「来源就绪且未暂停」（含生成流）时持有断言：有视频帧时阻止显示器睡眠（`caffeinate -d -i`），纯音频只阻止系统睡眠（`-i`），以 `-w <宿主 pid>` 绑定宿主进程。作者指出第 1 步「插件崩溃会留下断言」的说法不对，已更正：进核心的理由是「正在播放」的判断在宿主 | 2 |
| 确认对话框 | **进宿主** | `plugin.dialogs.confirm(options: { message: string; detail?: string; confirm: string; cancel?: string; destructive?: boolean; checkbox?: string }): Promise<{ confirmed: boolean; checked: boolean }>`（需 `ui.dialog`）。用 NSAlert（`objc2-app-kit` 的安全方法：`addButtonWithTitle`、`setShowsSuppressionButton` 作复选框、按钮 `setHasDestructiveAction`） | 3a |
| 外部工具调用 | **进 runtime（SDK 层）** | `plugin.tools.run(tool: "ffmpeg" \| "ffprobe", args: string[], options: { timeoutMs: number; maxOutput?: number /*默认 16 MiB*/; signal?: AbortSignal }): Promise<{ stdout: Uint8Array; stderr: string }>`；`plugin.tools.available(tool): Promise<boolean>`。定位规则 `VESTRA_FFMPEG` / `VESTRA_FFPROBE` / PATH；不经 shell；作用域释放时结束进程；失败码 `unavailable`（找不到）、`timeout`、`failed`（非零退出，`message` 为 stderr 末行）、`invalid`（输出超限） | 2 |
| 设置文件读写 | **进 runtime（SDK 层）** | `plugin.storage.json<T>(name: string, options: { defaults: T; validate?: (value: unknown) => value is T; debounceMs?: number /*默认 300*/ }): JsonStore<T>`；`interface JsonStore<T> extends Disposable { readonly value: T; save(next: T): void; flush(): Promise<void> }`。文件在 `dataDirectory/<name>.json`，临时文件 + 改名；读坏或校验失败时改名为 `<name>.json.bad-<时间戳>` 并用默认值；`dispose` 与插件停用时先 flush | 2 |
| 显示 / 打开文件夹 | **进 runtime（SDK 层）** | `plugin.system.reveal(path: string): Promise<void>`（Finder 中选中）；`plugin.system.openPath(path: string): Promise<void>`（打开文件或文件夹）。路径不存在时 `missing` | 2 |
| 输入链接小窗 | **进宿主 UI 库（不加协议）** | `@vestra/widgets.slint` 新增 `UrlInputRow`（输入框 + 确定 / 取消，规格 UI-DLG-02） | 3b |
| 运行时图片 / 设置页 | 已在 R-U1、R-S1 | — | 3b |
| `sips`、系统信息命令、钥匙串、`fs.watch` | **不做** | 留在各插件 | — |

runtime（SDK 层）指在插件进程里由 SDK 实现（`src/sdk/plugin/`，经 `plugin` 对象提供），不增加宿主协议与 Rust 代码；插件仍只 `import type` SDK。

### 3.4 裁决统计

共 67 条：

| 结论 | 数量 |
| --- | --- |
| 拒绝+替代 | 6（R-P5、R-T5、R-V11、R-U4、R-U5、R-K2） |
| 改写后接受 | 30 |
| 接受 | 31 |

另有通用能力 7 项：进宿主 3（剪贴板、阻止休眠、确认对话框），进 runtime 3（外部工具、设置文件、显示文件夹），进宿主 UI 库 1（输入链接组件）；不做 4 项。

### 3.5 核心体积预估

基线：宿主 Rust 22 770 行（`src/host/src/*.rs`），SDK 与运行器约 4 800 行（`src/sdk/plugin/*.ts`、`src/runtime/`）。预算：每轮宿主 Rust 净增不超过约 3 000 行（约 13%）；超过就拆。估算含单元测试，不含文档。

| 轮 | 新增接口（方法 / 模块） | 扩展的类型与范围 | 宿主行为 | 新能力名 | 宿主 Rust 估算 | SDK / runtime 估算 |
| --- | --- | --- | --- | --- | --- | --- |
| 2 | 7：`player.step`、`setStepSeek`、`host.run`、`addAudioTrack`、`selectAudioTrack`、`clipboard.*`、`power.preventSleep`；另有 runtime 3 个模块 `tools`、`storage`、`system` | `PlaybackStats` 字段、`SubtitleTrack` 3 字段、`SubtitleStyle` 4 字段、`load` 选项、2 个范围放宽、8 种视频节点、1 种音频节点 | 5：H1、H4、H5、H12、阶段立即推送 | `clipboard` | 约 2 600 | 约 1 300 |
| 3a | 15：`window.state/onChange`、`setVideoArea`、`setAlwaysOnTop`、`setCursorAutoHide`、`plugin.app`、`packages.list/onChange/setEnabled/uninstall`、`bindings.*`、`audioDevices`、`setAudioDevice`、`chapters`、`setLoop`、`dialogs.confirm` | `select` 的 `role`、`load` 的 `"secondary"`、`secondaryPosition`、`PlayerState.loop`、`audioOutput`、`Mouse:LeftClick` | 5：H9、H10、H11、H13、次字幕 | `window.control`、`bindings.manage` | 约 2 800 | 约 900 |
| 3b | 7：`ui.image`、`drops.provide`、`Panel.onDrop`、`panels.mount("backdrop")`、`settings.*`、`appearance`、`timeline.*` | `PropValue`、widgets 组件 | 2：H6、H7 | `ui.drop`、`timeline.marks`、`ui.settings`、`ui.theme` | 约 2 900 | 约 800 |
| 4a | 2：`video.view()`、`window.setPictureInPicture` | `frameBlend` 节点、`window.state.pictureInPicture` | 3：H2、H8、渲染节奏 | — | 约 3 000（画中画不确定性大） | 约 300 |
| 4b | 1：`video.render()` | `PlaybackStats.video.transfer` | 1：H3 | — | 未定（设计门槛） | 约 200 |

结论：

- 五批合计宿主净增约 11 000 行（不含 4b），平均每批约 2 800 行，都在预算内；原第 2、3、4 轮各自都会超出，已按上面拆开。
- 接口面：新增方法与模块合计 35 个（宿主 32、runtime 3），新增能力名 7 个（附录 A 原有 6 个全部保留，另加 `clipboard`），没有一个是为单个插件的特殊需求开的。被拒绝和收窄的条目共减少约 15 个接口或类型分支（`preload`、`cues`、JPEG 导出、`commands.popup`、`ignoreDefaults`、`can.*` 两个、`cas`、`temporalDenoise`、`scaleX/Y`、`themes`、`corners`、`devicesRevision`、`AppInfo.plugins`、`onDragOver`、`mode` 等）。
- 第 4b 轮（C-GPU）是唯一可能失控的一批：体积与可行性都取决于 Safe Rust 下的渲染封装。开工前由维护者出设计并给出行数估算，超出预算或需要 `unsafe` 时按 §7 请用户决定，不默认开工。
