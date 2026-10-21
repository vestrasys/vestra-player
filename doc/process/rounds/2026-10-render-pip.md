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
