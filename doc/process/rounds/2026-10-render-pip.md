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

（各阶段完成后补充）

### 3.1 B 线交付：画中画（维护者乙，2026-10-21）

**结论**：`window.setPictureInPicture(on)` 与 `WindowState.pictureInPicture` 已实现（macOS 14+），按根因重做了显示图层的位置。静态检查全部通过；没有操作 Vestra GUI，小窗是否铺满要协调者实测（§3.1.5）。

#### 3.1.1 离线探针结果（先验证，再动手）

探针扩展了 9 个 variant（[`assets/pip-probe.swift`](assets/pip-probe.swift) 开头有说明），每个在本机跑一次，只看探针自己打印的日志：

| variant | 做法 | renderSize | 说明 |
| --- | --- | --- | --- |
| normal | 图层铺满 content view | 640×360 | 基线 |
| small | 160×90 子图层 | 160×90 | 白边的情况 |
| covered | 同 normal，上面盖一个不透明 view | 640×360 | **被遮挡但尺寸正确：正常** |
| area | 图层 = 视频区（去掉侧栏 120、底栏 48） | 520×292 | renderSize 是图层里视频画面的矩形（16:9 按比例） |
| gl | 模拟 Vestra：翻转的 layer-backed view，NSOpenGLContext 画不透明红色，图层插到 view 图层的第 0 个子图层 | 520×292 | 尺寸对，但日志显示 GL 画面是 `_NSOpenGLViewBackingLayer` 的 contents，**子图层总是画在 contents 上面**：放进 winit 的 view 会盖住 Slint 画的屏幕提示、信息块、浮动底栏 |
| glresize | 同 gl，启动前窗口改到 960×540 | 840×472 | 跟随改尺寸 |
| hidden | `isHidden = true` | 无 | AVKit 不报 renderSize，不能用 |
| clear | `opacity = 0` | 640×360 | 不采用（小窗里是否可见无法从日志判断） |
| child | 图层在一个无边框**子窗口**里，子窗口排在主窗口**下面**，被主窗口完全挡住 | 640×360 | 启动正常（子窗口 `occlusionState` 不含 visible 也行） |
| childgl | 正式方案：GL 主窗口 + 子窗口框在视频区的屏幕矩形上；画中画打开后主窗口改尺寸，子窗口跟着改 | 520×292 | 启动正常 |
| grow / bigframe | 画中画打开后改窗口尺寸 / 1920×1080 帧 + display immediately（Vestra 的喂帧方式） | 640×360 | AVKit 不移动、不缩放源图层；打开期间改尺寸不会再报 renderSize |

另外：用 `stopPictureInPicture` 主动关闭时，AVKit 也会先调 `restoreUserInterface…`（宿主据此区分「返回」与「关闭」时要排除自己发起的关闭，见下）。

#### 3.1.2 做法

- **显示图层**：`AVSampleBufferDisplayLayer` 是一个无边框子窗口内容 view 的子图层，铺满子窗口。子窗口用 `addChildWindow:ordered:NSWindowBelow` 挂在主窗口**下面**，每个 tick 把它的 frame 设成视频区的屏幕矩形（Slint 新输出 `video-x`、`video-y` 加原有的 `video-width/height`，换算到 Cocoa 屏幕坐标）。所以图层与视频区同大同位、随窗口 / 全屏 / 侧栏变化，又被主窗口不透明的视频区挡住，不影响任何叠加层。子窗口不接收鼠标、无阴影、不进窗口菜单、`FullScreenAuxiliary | IgnoresCycle`。主窗口拖动时子窗口由系统带着走。
- **控制器**在子窗口第一次上屏之后才创建（AVKit 按源图层几何布局自己的视图）。主窗口重建（挂载 / 卸载界面）只换父窗口，图层和控制器不变，**画中画不中断**（原型的第 7 条限制解决）。
- **帧（CPU 路径）**：`show_frame` 拿到的软件渲染 RGB24 拷进 `CVPixelBufferPool` 里的 IOSurface BGRA 缓冲（按尺寸建池，不再每帧新建），包成 display-immediately 的 `CMSampleBuffer` 入队。只在画中画启动中 / 打开期间喂帧；暂停时用最后一帧（`Image::to_rgb8`）起步。
- **时间基**：图层渲染器自带的时间基按播放位置、倍速（暂停为 0）设置，偏差超过 0.5 s 才重设；可跳转的来源报告 `[0, duration]`，小窗有进度条和跳转按钮，直播 / 不可跳转报告无限范围（只有播放 / 暂停）。
- **按钮回传**：`setPlaying` → 与当前状态不同才执行核心动作 `play-pause`；`skipByInterval` → `seek-forward` / `seek-backward`，步长用系统给的间隔（`run_core_action_with` 新增步长参数，其它调用不变）。
- **进入 / 退出同步**：控制器委托报告 `didStart`、`didStop`、`failedToStart`、`restoreUserInterface`。`setPictureInPicture(true)` 在 `didStart` 后 resolve，`false` 在 `didStop` 后 resolve；5 秒没完成 `failed`。小窗关闭按钮（没有 restore、也不是宿主发起）→ 宿主暂停；「返回」按钮不暂停。新来源没有视频、来源关闭、打开它的插件停用 → 宿主关闭小窗。全屏中调用 `true` → 先退出全屏，退出完成后再启动。画中画期间主窗口视频区显示灰字「正在以画中画播放」（`host.slint` 新属性 `picture-in-picture`）。
- **平台**：平台无关的规则在 `src/host/src/pip.rs`（`PictureInPicture<S: Surface>`，6 个单测用假 Surface 覆盖：检查顺序、等帧 / 等放置 / 全屏、按钮、关闭按钮暂停与返回不暂停、失败 / 超时 / 视频消失、插件停用）。系统实现：`platform/macos.rs` 的 `PipWindow`（包 `vestra-pip`）；Windows 与 Linux 共用 `platform/pip_unsupported.rs`（`PipWindow` 是空枚举，`new()` 返回 `unsupported`，`pip_supported()` 为 `false`）。以后做 Windows / Linux 的宿主自建无边框置顶小窗时，只需在那里实现 `Surface`。
- **unsafe**：只在 `src/host/unsafe/pip/`（`vestra-pip` 0.1.0）。`src/lib.rs` 703 行（`use_small_heuristics = "Max"` 格式，与 render 子包一致；含 3 个测试），`unsafe` 块 25 个（测试里 2 个），`unsafe impl` 3 个（`define_class!` 里 NSObjectProtocol、两个 AVKit 委托协议），`#[unsafe(...)]` 属性 10 处；每个块、每个 `unsafe impl` 都有 `// SAFETY:`（脚本检查无遗漏）。另有 `examples/check.rs`（105 行，2 个 `unsafe` 块）：crate 自己的独立检查程序，见下。宿主主包 `forbid(unsafe_code)` 不变，`grep -rnw unsafe src/host/src` 为空。
- **feature**：原型的 `pip` feature 删除，`vestra-pip` 成为 macOS 的普通依赖（默认构建就带画中画）。`VESTRA_PIP` / `VESTRA_PIP_OVERLAY` 诊断开关随原型删除。

**crate 自检（不是 Vestra GUI）**：`cargo run -p vestra-pip --example check` 打开一个被红色 view 盖住的 640×360 窗口，把 `Pip` 放在左上 520×312 的「视频区」，喂 1920×1080 RGB，启动、打开期间改窗口尺寸、主动停止。本机结果：`Started renderSize (520, 292)`、`Stopped { restored: true }`，与探针 childgl 一致。

#### 3.1.3 与 A 线（GPU 渲染）的衔接点

- 开 `gpu-render`（或 A 线正式化后的 GPU 路径）时 `show_frame` 不再被调用，画中画收不到帧，`setPictureInPicture(true)` 会在 5 秒后 `failed`（「画中画没有启动」）。
- 零拷贝接口已预留：`vestra_pip::Pip::enqueue_surface(&IOSurfaceRef)`（`CVPixelBufferCreateWithIOSurface` 包住，不拷贝；测试覆盖），宿主侧 `platform::PipWindow::push_surface(&vestra_pip::IOSurfaceRef)`（暂时 `allow(dead_code)`）。
- 合并时要做的：(1) 在 GPU 渲染线程「出了一帧」回到主线程的地方（原型是 `Handle::on_frame` → `invoke_from_event_loop`），当 `self.pip.feeding()` 时把那块 IOSurface 交给画中画：给 `pip::Surface` 加一个 `push_surface` 方法（macOS 调 `PipWindow::push_surface`，`pip_unsupported.rs` 里 `match *self {}`），在 `PictureInPicture` 里像 `push` 一样置 `fed`；(2) IOSurface 必须是 BGRA，并且在 AVKit 显示期间不能被渲染线程改写：轮转时要跳过 `IOSurfaceIsInUse` 的那块（原型 layer-video 已这样做），否则小窗可能撕裂；(3) 暂停时起步用的「最后一帧」改为最新一块 IOSurface（`Handle::latest()`）；(4) 删掉 `push_surface` 上的 `allow(dead_code)`。
- 渲染尺寸：现在小窗的分辨率就是渲染尺寸（C-SIZE 跟随主窗口视频区）。小窗被用户拉得比主窗口视频区大时会发糊；GPU 路径下可以让渲染尺寸取「视频区」与「小窗」的较大者。AVKit 打开期间改小窗尺寸不再回调 renderSize（探针 grow），要另找来源（暂不做）。

#### 3.1.4 给作者的接入说明（window 插件 H07）

- 能力：已有的 `window.control`。
- 可用条件：`plugin.window.state.pictureInPicture.supported` 为 `true`（macOS 14+）且有视频（`player.state.videoSize` 非空）。`supported` 为 `false`（Windows、Linux、无界面）时保留迷你尺寸（`setVideoArea` + `setAlwaysOnTop`）作为退路，菜单文字和快捷键不变。
- 开关：`await plugin.window.setPictureInPicture(!plugin.window.state.pictureInPicture.active)`。resolve 时 `state` 已更新；全屏中调用由核心先退出全屏，插件不用自己退。提示建议：成功「画中画: 开」/「画中画: 关」；失败「无法进入画中画：<message>」。上一次没返回时再按会 `busy`，插件忽略即可（或自己在 await 期间禁用命令）。
- 勾选状态：用 `state.pictureInPicture.active`，并 `plugin.window.onChange` 跟随。用户在小窗里点「返回」或关闭、来源没有视频、来源关闭时，`active` 会自己变成 `false`，插件据此提示「画中画: 关」（没有视频时可以提示「画中画: 关（没有视频）」）。
- 不需要插件做的：小窗按钮回传、关闭按钮暂停、主窗口灰字、插件停用时退出画中画，都由核心处理。
- 测试：`TestHost` 的 `host.mainWindow.pictureInPicture`（`{ supported, active }`），`setPictureInPicture` 立即切换并退出全屏；`host.setMainWindow({ pictureInPicture: { supported: true, active: false } })` 模拟用户在小窗里关闭；`supported: false` 模拟 Windows。直接 `deepEqual` 整个 `window.state` 的现有测试会多一个字段（SDK 的 `round3a` 测试已更新；各插件测试未受影响）。

#### 3.1.5 给协调者的 GUI 实测要点

构建：`cd src/host && cargo build --locked --release`（默认就带画中画；不需要 feature 或环境变量）。片源用 1080p 与 4:3 / 竖屏各一个。window 插件接入前，可以用任意有 `window.control` 的测试插件调 `plugin.window.setPictureInPicture(true)`。

| # | 场景 | 期望 / 记录 |
| --- | --- | --- |
| P1 | 播放 1080p，调用 `setPictureInPicture(true)` | 1 秒内出现系统小窗；**画面按比例铺满小窗，没有白色区域**（这是本轮要验证的根因）；调用 resolve，`state.pictureInPicture.active` 为 `true`；主窗口视频区变成灰字「正在以画中画播放」，主窗口里看不到任何额外的窗口或画面；标题栏正常 |
| P2 | 拖小窗边角放大、缩小；拖到其它角落 | 画面一直铺满、比例正确（放大后可能变糊，记录程度） |
| P3 | 小窗里点暂停 / 播放、前进 / 后退 15 秒、拖进度条（如有） | 宿主跟着暂停 / 继续 / 跳转；在宿主里按 Space 暂停时，小窗按钮跟着变 |
| P4 | 小窗「返回」按钮；再打开，点小窗关闭按钮 | 返回：小窗关闭、播放继续、主窗口恢复画面；关闭：小窗关闭并**暂停**。两种情况 `active` 都变 `false` |
| P5 | 画中画打开时：改主窗口尺寸、开关侧栏、最小化主窗口、切到别的应用 / 别的空间 | 小窗照常播放（主窗口最小化时也是）；恢复主窗口后画面正常；关闭小窗时回到主窗口的动画落在视频区上 |
| P6 | 全屏中调用 `setPictureInPicture(true)`；画中画打开时进入全屏再退出 | 先退出全屏再出现小窗；进出全屏不崩溃，关闭小窗后画面正常 |
| P7 | 画中画打开时挂载 / 卸载一个界面插件（重建主窗口） | 小窗**不关闭**、继续播放；新主窗口显示灰字 |
| P8 | 画中画打开时换来源（视频 → 视频；视频 → 纯音频）；关闭来源 | 视频 → 视频：小窗保持并显示新画面，比例跟着变；纯音频或关闭：小窗自动关闭，`active` 变 `false` |
| P9 | 停用调用它的插件 | 小窗关闭，不暂停 |
| P10 | 置顶（`setAlwaysOnTop(true)`）后再开画中画 | 无异常；主窗口下面没有露出黑色子窗口 |
| P11 | CPU：P1 状态下与关闭画中画时各采样 20 秒 | 记录每帧拷贝的额外开销（已改为缓冲池） |

如果 P1 仍有白边或只显示一角：记录小窗尺寸和画面占的区域，并在终端运行 `cargo run -p vestra-pip --example check` 对照（独立小程序，同样的子窗口方案）。

#### 3.1.6 检查

全部在本 worktree 执行，都通过：`src/` 下 `npm run check`、`format:check`、`boundaries`、`test:sdk`（100 个，含新的 `tests/sdk/pip.test.ts` 3 个）、`plugins:build`、19 个插件各自 `npm run plugin -- test <名字>`（全部 0 失败；workshop 的 `vendor/sdk` 已按 `vendor-sdk.test.mjs` 开头的命令重新同步）、`npm run portability`（clean：宿主能按 Windows 目标编译，`PipEvent` 在 Windows 上未构造的警告已消除）；`src/host/` 下 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`（240 个，含 `pip::` 6 个），另加 `--features gpu-render` 的 clippy、`cargo clippy -p vestra-pip --all-targets -D warnings`、`cargo test -p vestra-pip`（3 个）；`grep -rnw unsafe src/host/src` 为空。注意磁盘：本机可用空间一度只有约 0.5 GB，`npm run portability` 用了主仓库 `src/host/target/portability` 的已有缓存（临时软链接，检查后删除）。

### 3.2 A 线交付一：GPU 渲染（维护者甲，2026-10-21，提交「Render: …」）

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

### 3.3 A 线交付二：Windows 缺口（维护者甲，2026-10-21，提交「Windows gaps: …」）

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

### 3.4 A 线交付三：合并画中画，GPU 帧送进画中画（接手维护者甲，2026-10-21，提交「Merge rebuild/render-pip …」「Render: feed picture-in-picture from the GPU path」）

前任维护者甲做完了 §3.2、§3.3 的代码和记录，没有提交 §3.3。接手后先审查：§3.3 的改动按原样保留，只把 `platform/windows.rs` 里放错位置的文档注释挪回 `default_ffmpeg`；重跑全部检查后提交。然后合并 `rebuild/render-pip`（维护者乙的 11b467b），再按 §3.1.3 接上 GPU 帧。

**合并**：冲突在 `Cargo.toml`（两个 unsafe 子包都是 macOS 的普通依赖，`gpu-render`、`pip` 两个原型 feature 都不再有）、`platform/mod.rs` 的总表、参考文档 §17（两条都保留）、本单 §3（乙的 §3.1 编号不变，因为探针源码引用了它；甲的两次交付顺延为 §3.2、§3.3）。`main.rs` 自动合并：原型的 `#[cfg(feature = "pip")]` 整段被乙的正式实现替换，`show_frame` 里软件帧照常送画中画。合并提交单独做了全部检查（见下）。

**§3.1.3 的四项**

| 项 | 做法 |
| --- | --- |
| (1) GPU 帧回调送画中画 | `pip::Surface` 加关联类型 `Frame` 和 `push_surface(&Self::Frame)`；`PictureInPicture::push_surface(sequence, frame)` 像 `push` 一样置 `fed`，同一序号只送一次（关闭、重新打开时清零）。宿主第一次 tick 见到 GPU 渲染器时注册一次 `GpuHandle::on_frame`：渲染线程每发布一帧，只在画中画要帧（`pip.feeding()`，每个 tick 写进一个 `AtomicBool`）时 `invoke_from_event_loop` 一次（合并未处理的唤醒），UI 线程取 `GpuHandle::latest()` 送进去。生成流（NES）在用视频区时不送 GPU 帧，它的像素帧照旧经 `show_frame` 送 |
| (2) BGRA、不改写在用的 IOSurface | 渲染子包的环本来就是 `'BGRA'` IOSurface，并且轮转时跳过 `IOSurfaceIsInUse` 的那块（§3.2 合入原型时就有）。环有 3 块；全部在用时仍会写下一块（宁可偶发撕裂，也不让画面停住），GUI 实测看 G3 |
| (3) 暂停时起步用最新一块 | `pip_tick` 里 `needs_frame()` 时：GPU 渲染送 `latest()`，软件渲染照旧用 `last_image` |
| (4) 去掉 `allow(dead_code)` | `PipWindow::push_surface` 这个固有方法删掉，改为 `Surface` 的实现，调用 `vestra_pip::Pip::enqueue_surface`（`CVPixelBufferCreateWithIOSurface`，零拷贝） |

**签名调整**（插件 API 不变，只有宿主内部）：`pip::Surface::{type Frame, push_surface}`；平台模块新增 `GpuFrame`（macOS 是 `vestra_gpu_render::IOSurfaceRef`，与 `vestra_pip::IOSurfaceRef` 是同一个 `objc2-io-surface` 0.3.2 类型；其它系统是空枚举），`no_gpu` 的 `GpuHandle` 补上 `latest()`（总是 `None`）与 `on_frame()`（不做事）；`pip_unsupported.rs` 的 `push_surface` 是 `match *self {}`。宿主主包仍是 `forbid(unsafe_code)`，unsafe 子包这次没有改动。新增单测 `pip::gpu_frames_feed_the_window_once_each_and_only_while_it_exists`。参考文档 §6.6 与 §17（画中画条目）补了一句：GPU 渲染时小窗显示 GPU 帧，分辨率是主窗口视频区的渲染尺寸。

**给作者**：接入说明不变，video 插件 E16/E17/E18 见 §3.2「给作者的接入说明」，window 插件 H07 见 §3.1.4。补充一点：E18 运动补偿打开后 GPU 渲染器按显示刷新率出帧，画中画也跟着收到同样多的帧。

**给协调者的 GUI 实测要点**（默认构建就是 GPU 渲染加画中画：`cd src/host && cargo build --locked --release`；R1–R9 见 §3.2，P1–P11 见 §3.1.5，CPU 测量命令见 §3.2）。原来 P1–P11 是按软件渲染写的，现在默认是 GPU 渲染，P 系列请在默认启动下做一遍，下面几项另外看：

| # | 场景 | 期望 / 记录 |
| --- | --- | --- |
| G1 | 默认启动（GPU），播放 1080p，打开画中画 | 1 秒内出小窗，画面铺满、颜色和方向正确（GPU 帧是 BGRA，如果红蓝互换要记录）；`stats().backend` 仍是「GPU 渲染（OpenGL）」 |
| G2 | 暂停状态下打开画中画 | 小窗起步就有当前画面（取最新一帧），不是黑的；然后在小窗里点播放，画面动起来 |
| G3 | 画中画打开时播放 4K、开 `interpolation`（运动补偿）、拖小窗放大 | 无撕裂、无闪回旧帧（渲染环只有 3 块，全部在用时会写下一块；如有撕裂请记录，必要时加大环） |
| G4 | 画中画打开时换来源（视频 → 视频）、改主窗口尺寸、进出全屏 | 小窗换到新画面，比例正确；改尺寸后小窗分辨率跟着视频区变 |
| G5 | 画中画打开时打开 NES，再停 NES 打开普通视频 | NES 画面进小窗（走软件帧），没有 GPU 旧帧插进来；之后普通视频正常 |
| G6 | `VESTRA_SOFTWARE_RENDER=1` 启动重复 P1、P3 | 与 §3.1.5 一致（软件帧路径） |
| G7 | CPU：4K 播放，GPU 渲染下画中画关 / 开各量 20 秒；软件渲染下再量一次画中画开 | 用 §3.2 的 `top` 命令。期望 GPU 下画中画开、关差别很小（零拷贝，只多一次唤醒和入队）；软件渲染下画中画开会多出 RGB→BGRA 拷贝的开销。三组数字都记下来 |

**检查**（macOS，本机实际执行，三个提交各跑一遍，结果相同，下面是最后一个提交的数字）：`src/`：`npm run check`、`format:check`、`boundaries` 通过；`test:sdk` 107 通过；`plugins:build` 通过；19 个插件测试共 488 通过、0 失败（workshop 的 `vendor/sdk` 与 `src/sdk` 逐字节一致，合并后无须重同步）；`npm run portability`：`OK: the host compiles for Windows`，`portability: clean`。`src/host/`：`cargo fmt --check` 通过；`cargo clippy --locked --all-targets -- -D warnings` 通过（另跑 `-p vestra-pip -p vestra-gpu-render` 也通过）；`cargo test --locked` 250 通过（Windows 缺口提交 243，合并提交 249）；`grep -rnw unsafe src/host/src` 为空。未执行：GUI（协调者做）、Windows 链接与运行。
