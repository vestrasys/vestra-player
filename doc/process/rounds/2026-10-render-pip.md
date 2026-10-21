# GPU 渲染、画中画与 Windows 缺口

分支：`rebuild/render-pip`（自 `main` 6a85fb6）。依据：[社区工作流](../community-workflow.md) §1.5（跨平台规则）、§4.3、§5。原型和实测见[第 4a 轮任务单](2026-10-features-r4a.md) §7，原型分支 `rebuild/prototypes`。

## 1. 用户裁决（2026-10-21）

- **GPU 渲染：锁 OpenGL**，因为要跨平台。libmpv 用 OpenGL 渲染，Slint 锁定 femtovg 渲染器，软件渲染保留作后备，用环境变量或启动失败时自动退回。方案 B（CALayer）只能在 macOS 上用，不采用。
- **画中画：做**。macOS 用系统画中画（AVKit），按离线复现得出的根因修：显示图层必须和视频区一样大，放在窗口里正常的位置。离线探针是 [`assets/pip-probe.swift`](assets/pip-probe.swift)（`swiftc -O pip-probe.swift -o probe && ./probe normal|small`）：标准设置正常，160×90 的小图层能复现白边。Windows 和 Linux 先返回 `unsupported`，以后由宿主自己做无边框置顶小窗。
- **Windows 缺口**：截图导出、生成流（`openStream`）、确认框按钮文字。这些写在[可移植性任务单](2026-10-portability.md) §4.2，本机只能验证编译通过。

## 2. 分工

| 线 | 负责 | 内容 |
| --- | --- | --- |
| A | 维护者甲 | 先把 `rebuild/prototypes` 的 render 子包合并进来（和可移植性轮有冲突，由维护者解决；layer 子包不合并）。然后正式实现 GPU 渲染：<br>· 渲染相关的系统代码放进平台模块，`unsafe` 只留在 `src/host/unsafe/render/`；<br>· 软件渲染后备；<br>· 硬件解码（hwdec）开启，同时更新 `stats` 和信息面板里的「硬件解码」行；<br>· `video.render()`（R-V10：HDR 色调映射、着色器、运动补偿）和 `PlaybackStats.video.transfer`；<br>· 处理原型遗留：换来源时闪一帧、帧日志与 stats、NES 生成流、浮动侧栏。<br>完成后接着做 Windows 缺口，第二个提交 |
| B | 维护者乙 | 画中画：从 `rebuild/prototypes` 取 pip 子包，按根因修好。核心 API 是 `window.setPictureInPicture(on)` 和 `WindowState.pictureInPicture`；画中画小窗里的按钮要回传到播放；非 macOS 返回 `unsupported` |
| C | 作者 | 在 A、B 交付之后：video 插件接上 E16、E17、E18 运动补偿；window 插件让 H07 用上画中画，迷你尺寸作为非 macOS 时的退路 |
| D | 协调者 | 真机验收，然后合并 |

## 3. 记录

### 3.1 A 线交付一：GPU 渲染（维护者甲，2026-10-21，提交「Render: …」）

**合入原型**：从 `rebuild/prototypes` 只取 render 子包的改动（`Handle::latest()`、`on_frame()`、轮转时跳过窗口服务器还在用的 IOSurface）。layer、pip 子包没有合入。原型在 `main.rs` 的 `layer-video` 接入整段丢弃；`gpu-render` 的接入按可移植性轮的平台模块重写（见下）。

**结构**

| 位置 | 内容 |
| --- | --- |
| `src/host/unsafe/render/`（`vestra-gpu-render`） | 唯一含 `unsafe` 的地方。新增：发布门（`Renderer::start(mpv, size, publish)`，门关着时照常渲染但不发布）、按刷新率限速并 `report_swap`（`set_display_fps`，运动补偿用）、`reshow()`（宿主换掉视频区图像后，窗口不再显示此前的帧，下一帧重新设图像）、`shown()`/`size()`（帧日志）、`attach(…, failed)`（窗口渲染器不是 OpenGL 时回调）。`on_frame` 与重画列表分开，不会被「最近 4 个窗口」挤掉。621 行（测试 72 行），`unsafe` 块 18 个（测试另 1 个），每块都有 SAFETY 注释；2 个测试 |
| `platform/macos.rs` | `GPU_RENDER = true`、`gpu_start`、`GpuRenderer`/`GpuHandle`（即 crate 的类型） |
| `platform/mod.rs` 的 `no_gpu` | Windows、Linux：`GPU_RENDER = false`，`gpu_start` 返回错误；类型没有值，方法永远不会执行 |
| `renderer.rs`（新） | `Render`：渲染方式、退回请求、生成流标志、刷新率；`wanted()`、`hwdec()`；2 个测试 |
| `video_render.rs`（新） | `video.render()` 的校验、mpv 选项映射、单一拥有者、失败恢复、着色器编译失败识别；4 个测试 |
| `backend.rs` | 渲染线程先试 GPU（`GpuRun`），失败或窗口要求退回时转软件渲染；`video.render.*`；stats 的 `backend`、`framesShown`、`hwdec`、`transfer` |
| `main.rs` | Slint 选 `renderer_name("femtovg")`（选不上就退回 Slint 默认渲染器 + 软件渲染）；`attach_gpu` 不再有 cfg；显示刷新率；帧日志合并两种计数 |
| `Cargo.toml` | 删掉 `gpu-render` feature，`vestra-gpu-render` 是 macOS 的普通依赖；`pip` feature 留给维护者乙 |

宿主主包仍是 `forbid(unsafe_code)`，`grep -rnw unsafe src/host/src` 为空。

**行为**

- 默认 GPU 渲染（macOS）。软件渲染后备：`--headless`；`VESTRA_SOFTWARE_RENDER=1`（`0` 或空值不算）；Windows、Linux；Slint 选不上 femtovg；mpv 的离屏 OpenGL 上下文建不起来；窗口的渲染通知不是 `NativeOpenGL` 或建不了 GL 对象（这时渲染线程丢掉 GPU 渲染器、改软件渲染，`hwdec` 改回 `no`）。每种退回都在标准错误输出写一行 `render: …`。
- 硬件解码：GPU 渲染时 `hwdec=auto-safe`（`VESTRA_HWDEC` 覆盖），解不了的格式 mpv 自己退回软件解码；`stats().video.hwdec` 是 `hwdec-current` 原值（`"no"` = 软件解码）。软件渲染照旧 `hwdec=no`。
- `video.render()`（R-V10，签名按协商第 3 步裁决：`plugin.video.render(): RenderOptions`，`set(Partial<{ toneMapping; targetPeak; peakDetect; shaders; interpolation }>)`，单一拥有者，`FilterStatus` 四态）。软件渲染下 `set` 返回 `unsupported`。映射见参考文档 §8.2。实测（crate 测试环境，mpv 0.41 + macOS OpenGL 4.1）：七个 `tone-mapping` 值、`target-peak`、`hdr-compute-peak`、`gamut-mapping-mode`、`display-fps-override` 都被接受；mpv 日志 `Disabling HDR peak computation (… compute shaders=0, SSBO=0)`，所以 `peakDetect` 在 macOS 上不起作用（参考文档写明）；坏着色器在第一次渲染时报 `libmpv_render` 级别 `error` 的 `ERROR: 0:39: …`，宿主据此把拥有者的状态推为 `failed`（`video.render.status`）。`off` 没有 mpv 的对应值，映射为 `tone-mapping=clip` + `gamut-mapping-mode=clip` + 不做峰值检测（与需求里的「发灰」不完全一样，已写进参考文档；如作者要「完全不处理」，需要另议，例如把 `target-trc` 设为 PQ，但那会影响 SDR 片源）。
- `PlaybackStats.video.transfer` = `video-params/gamma` 原值。
- 原型遗留：
  - 换来源闪一帧：GPU 渲染器也走发布门（方向准备期间不发布）；来源进入 `empty` 时清掉视频区并 `reshow()`，下一个来源从自己的第一帧开始，不会先露出上一个来源的最后一帧。
  - 帧日志与 stats：`framesShown` = 软件计数 + GPU 发布数；`VESTRA_FRAME_LOG=1` 的「produced / shown」在 GPU 下是发布数 / 窗口拷贝数，尺寸是 GPU 帧尺寸；`backend` 如实。
  - NES 生成流在改尺寸时被覆盖：后端每轮先写「生成流在用视频区」标志，窗口的 GPU 回调在标志为真时不设图像；生成流的每帧和停止后的清屏帧都会 `reshow()`，之后的 GPU 帧照常显示。
  - 浮动侧栏不显示视频：浮动侧栏窗口（`FloatingPanel`）本来就没有视频区，软件渲染下也不显示视频，这是设计如此，不是 GPU 的缺口。分离侧栏会重建主窗口，新主窗口照常 `attach_gpu`，第一次重画就拷最新一帧。需要 GUI 确认主窗口在侧栏浮动期间一直有画面（下面 R5）。
- C-SIZE 的来源封顶只在软件渲染下有；GPU 按视频区物理像素渲染，放大由 mpv 完成。

**给作者的接入说明（C 线，video 插件）**

- E16「HDR 色调映射」：`const render = plugin.video.render();`（manifest 已有 `processing.video`）。单选 → `render.set({ toneMapping })`；「目标亮度」→ `targetPeak`（100–400）；「动态峰值检测」→ `peakDetect`（macOS 上不起作用，菜单可以照常保留）。非 HDR 判断：`(await plugin.player.stats()).video?.transfer` 是 `"pq"` 或 `"hlg"` 才是 HDR。持久化照需求 `render.*`，激活时把保存的值一次 `set` 进去。
- E17「像素着色器」：`render.set({ shaders: [...] })`，必须是绝对路径、文件存在、最多 16 个；总开关关时 `set({ shaders: [] })`（列表由插件自己保留）。编译失败是稍后推来的 `failed`（`code: "failed"`，`message` 以 `shader compile failed: ` 开头，mpv 不报文件名），按需求显示「着色器载入失败」。「打开着色器文件夹」用 `plugin.system.openPath`（不要再用 `spawn("open")`）。
- E18「运动补偿」：`render.set({ interpolation: true/false })`。
- 禁用条件：`render.status` 为 `failed` 且 `error.code === "unsupported"` 时（软件渲染、Windows），三组命令禁用，原因可写「需要 GPU 渲染（当前为软件渲染）」；把原来的常量 `GPU = "等待 GPU 渲染原型结论"` 换掉。`conflict` 表示别的插件占着渲染选项。示例见 `doc/plugins/examples.md` §34。
- info 插件（属作者）：「硬件解码」一行可以改为读 `stats().video.hwdec`（`"no"` 显示「未使用」，其它显示解码器名），`backend` 字段已经如实区分两种渲染。这一行不属于 A 线可写范围，本轮没有改。

**给维护者乙**：GPU 渲染默认开启后，`show_frame` 只在生成流时被调用，画中画应改用 `GpuHandle::latest()`（最新一帧的序号和 IOSurface）与 `on_frame()`（渲染线程每发布一帧回调一次），可以直接 `CVPixelBufferCreateWithIOSurface`，免拷贝。平台模块 macOS 下 `GpuHandle` 就是 crate 的 `Handle`，这两个方法可以直接调用；宿主里通过 `self.backend.render.gpu()` 取得。

**给协调者的 GUI 实测要点**（release：`cd src/host && CARGO_TARGET_DIR=/tmp/vestra-review-target/render cargo build --locked --release`）

片源：

```sh
ffmpeg -f lavfi -i testsrc2=size=3840x2160:rate=30 -t 30 -c:v libx264 -pix_fmt yuv420p -preset veryfast /tmp/t2160.mp4
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 30 -c:v libx264 -pix_fmt yuv420p /tmp/t1080.mp4
```

CPU 测量（播放器在前台播放时，另开终端；`PID` 是 `vestra-host` 进程，去掉第一个样本取平均）：

```sh
PID=$(pgrep -x vestra-host | head -1)
top -l 21 -s 1 -pid $PID -stats cpu | awk '/^%CPU/{getline v; n++; if(n>1){s+=v; if(v>m)m=v}} END{printf "mean=%.0f%% max=%.0f%%\n", s/(n-1), m}'
```

| # | 场景 | 期望 / 记录 |
| --- | --- | --- |
| R1 | 默认启动，打开 4K 片源；窗口、全屏各量 20 秒 CPU；再用 `VESTRA_SOFTWARE_RENDER=1` 启动重复 | 原型参考：GPU 窗口 19%、全屏 14%；软件 81%、152%。信息面板 / `stats()` 的 `backend` 分别为「GPU 渲染（OpenGL）」「软件渲染」，`hwdec` 为 `videotoolbox` / `no` |
| R2 | 画面方向、颜色、清晰度；OSD、信息块、右键菜单、全屏浮动底栏、字幕都在画面之上 | 与软件渲染一致 |
| R3 | 换来源：播放中打开另一个文件，再打开第一个；关闭文件后再打开；打开一个带旋转标记的竖拍视频 | 每次都出画面；不先闪上一个来源的最后一帧；竖拍视频不闪未旋转的一帧 |
| R4 | 改尺寸：连续拖窗口角 10 秒；进出全屏 | 画面跟随，无撕裂、无错位 |
| R5 | 界面重建与浮动侧栏：播放中停用 / 启用一个挂界面的插件；把播放列表侧栏分离为浮动窗口再合回 | 播放不中断，主窗口一直有画面（浮动窗口本身没有视频区） |
| R6 | 生成流：打开 NES，播放中拖窗口改尺寸、进出全屏；停止 NES 后打开普通视频 | NES 画面不被黑屏或旧视频盖住；之后普通视频正常显示 |
| R7 | `VESTRA_FRAME_LOG=1` 启动播放 1080p | 每秒一行 `produced ~30 shown ~30`，`target` 是视频区物理尺寸 |
| R8 | 渲染选项（插件接入前可以跳过，或用 examples §34 的写法临时做一个插件）：切 `toneMapping`、加一个反色着色器、加一个语法错误的着色器、打开 `interpolation` | 切换时播放不中断；反色生效；坏着色器状态变为 `failed` 并带编译器错误；运动补偿时画面不卡顿，`VESTRA_FRAME_LOG=1` 的帧数接近显示刷新率 |
| R9 | 截图：`--snapshot` 或含效果截图 | 记录截图里是否有视频（GPU 纹理是否进 `take_snapshot`，原型未验证；插件的截图走离线 FFmpeg，不受影响） |

**检查**（macOS，均实际执行）：`src/`：`npm run check`、`format:check`、`boundaries` 通过；`test:sdk` 102 通过（新增 `tests/sdk/render.test.ts` 5 项）；`plugins:build` 通过；19 个插件测试共 488 通过、0 失败（workshop 的 `vendor/sdk` 已重同步）；`npm run portability`：`portability: clean`（Windows 目标下宿主编译通过，GPU 部分走 `no_gpu`）。`src/host/`：`cargo fmt --check` 通过；`cargo clippy --locked --all-targets -- -D warnings` 通过（另跑 `--features pip` 与 `-p vestra-gpu-render` 也通过）；`cargo test --locked` 240 通过；`cargo test -p vestra-gpu-render` 2 通过；`grep -rnw unsafe src/host/src` 为空。未执行：GUI（按约定由协调者做）、HDR 片源实测（本机没有 HDR10 片源）。

### 3.2 A 线交付二：Windows 缺口（维护者甲，2026-10-21，提交「Windows gaps: …」）

按[可移植性任务单](2026-10-portability.md) §4.2 的三项。本机只能交叉编译检查，Windows 上没有运行过。

| 缺口 | 做法 | 位置 |
| --- | --- | --- |
| 截图与片段导出 | 离线工作进程照旧把持有的源文件作为 FFmpeg 的标准输入（租约、前后校验不变），只是输入名由平台模块给：Unix `/dev/fd/0`，Windows `fd:`（FFmpeg 6.1 起的 `fd` 协议，可以对文件句柄 seek；`pipe:` 不行）。`encode`、MP4 校验、内嵌字幕提取去掉 `cfg(unix)`，删掉 Windows 的「不可用」桩；`ffprobe` 按 `EXE_SUFFIX` 找同目录的 `ffprobe.exe` | `platform::HELD_FILE_INPUT`；`media_worker.rs`、`native_subtitle.rs` |
| `openStream` | 传输放进平台模块：Unix 仍是 `/tmp` 私有目录下的 Unix 套接字（`av_unix`）；Windows 是回环 TCP（`av_tcp`，只用 std），因为 Node 在 Windows 上不支持 Unix 套接字路径。本机任何进程都能连回环端口，所以播放器生成 32 字节随机令牌（`getrandom`，锁文件里已有的 0.3.4），SDK 连上后先发令牌，令牌不对的连接直接丢弃。`producer.prepare` 的答复多一个可选字段 `token`，`socket` 为 `tcp://127.0.0.1:<端口>`；SDK 按地址形态连接（`connectTo`），不按 `process.platform` 判断。顺带：Unix 上接受的连接改回阻塞模式（macOS 会继承监听端的非阻塞，原来的读超时形同虚设，读循环会空转） | `platform/mod.rs` 的 `av_unix`、`av_tcp`、`av_listen`；`producer.rs` 去掉全部 `cfg(unix)`；`sdk/producer.ts` |
| 确认框按钮文字 | rfd 打开 `common-controls-v6`（`TaskDialogIndirect`，可以自定义按钮），`build.rs` 在 MSVC 目标上用链接参数 `/MANIFEST:EMBED /MANIFESTINPUT:windows/vestra.manifest` 嵌入 Common Controls v6 的应用清单（没有清单时打开这个特性的程序启动不了）。不引入新 crate。两个按钮文字相同时，取消按钮末尾加一个不换行空格，答复才不会混淆（`confirm::button_labels` / `confirmed_by`，有单元测试）。复选框仍是第二个「是 / 否」对话框（rfd 不提供任务对话框的复选框） | `Cargo.toml`、`build.rs`、`windows/vestra.manifest`、`platform/windows.rs`、`confirm.rs` |

另外：Windows 目标下宿主的警告从 3 条减到 1 条（`no_gpu` 改成无效果的普通类型，不再产生「不可达代码」；`media_worker` 的取消测试不再限于 Unix）。剩下的 1 条是 `source_backend.rs` 的 `playback` 字段（`fd://` 租约，§4.2 表里的另一项，不在本轮范围）。

**SDK / TestHost / 文档**：`PreparedProducer.token?`、`connectTo`；`TestHost.producerTransport = "tcp"`（假生成端校验令牌，`producer.admitted`）；新增 `tests/sdk/windows-gaps.test.ts`（2 项：TCP 加令牌的 `openStream` 全流程、`connectTo` 只把 `tcp://127.0.0.1:` 当回环地址）；宿主新增测试：回环传输只接受带令牌的连接、按钮文字区分。参考文档 §5.2（传输）、§7（Windows 任务对话框）、§10（Windows 导出与 FFmpeg 6.1）、§14、§16.7、§17「渲染轮：Windows 缺口」。

**给协调者**：没有 Windows 机器，以下只能等有 Windows 环境时验证：(1) 链接后的 `vestra-host.exe` 能启动（清单已嵌入：`mt.exe -inputresource:vestra-host.exe;#1 -out:m.xml` 能导出含 `Microsoft.Windows.Common-Controls` 的清单）；(2) 插件确认框显示自定义按钮；(3) 截图导出（`VESTRA_FFMPEG` 指向 6.1+ 的 `ffmpeg.exe`）；(4) NES 生成流出画面和声音。macOS 上这次提交的行为不变（生成流的读循环不再空转除外），回归可以只看 R6（NES）和一次插件截图导出。

**检查**（macOS，均实际执行）：`src/`：`npm run check`、`format:check`、`boundaries` 通过；`test:sdk` 104 通过；`plugins:build` 通过；19 个插件测试共 488 通过、0 失败（workshop 的 `vendor/sdk` 已重同步；前任两次整套运行中各有一个计时类测试偶发失败，playback 一次、subtitles「上传中取消」一次，单独重跑都通过；接手后重新整套运行一次全部通过）；`npm run portability`：`OK: the host compiles for Windows`，`portability: clean`。`src/host/`：`cargo fmt --check` 通过；`cargo clippy --locked --all-targets -- -D warnings` 通过（另跑 `--features pip`）；`cargo test --locked` 243 通过；`grep -rnw unsafe src/host/src` 为空。未执行：Windows 链接与运行（没有 MSVC 链接器和 Windows 环境）。
