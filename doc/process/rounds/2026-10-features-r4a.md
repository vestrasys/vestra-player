# 第 4a 轮：渲染尺寸跟随视频区（C-SIZE）、显示层、倍帧与渲染节奏 + 插件接入

分支：`rebuild/features-r4a`（自第 3b 轮合并到 `main` 之后的提交新建，由协调者创建）。依据：[协商记录](2026-10-features-negotiation.md)「第 3 步：裁决」（下称「裁决」）的 4a 排期与 3.5「核心体积预估」，[功能需求](../../product/feature-requirements.md)（下称「需求文档」）E11、E13、E18、H07，[社区工作流](../community-workflow.md) §3、§4.3、§7，[第 3b 轮任务单](2026-10-features-r3b.md)（下称「r3b」）的格式与规则。

本轮先做了可行性调研（§1）。裁决把画中画（R-W5、R-H8、`window.state.pictureInPicture`）排在 4a，前提是「先确认 AVKit 绑定在 Safe Rust 下可用」。调研结论是**不可用**，所以画中画不进本轮，按工作流 §7 升级给用户（§7.1）。4b 的 C-GPU 同样不可行，一并请用户裁决（§7.2）。

## 1. 前置调研（2026-10-21，维护者）

环境：Apple M2（8 核），内置 Retina 屏 2560×1664，macOS 26.2（Darwin 25.2）；Homebrew mpv / libmpv 0.41.0（FFmpeg 9.0.2，libplacebo 7.360）；宿主依赖 `rsmpv =0.2.1`（`render` 特性）、`slint =1.16.1`、`objc2 =0.6.5`、`objc2-app-kit =0.3.2`。原型都在 scratchpad 里（`…/scratchpad/r4/`），不进仓库。

### 1.1 C-SIZE：软件渲染跟随视频区尺寸——**可行**

现状：`backend.rs` 渲染线程固定按 960×540 RGB24 调 `render_software`，UI 线程每 16 ms 的定时器 `take()` 最新一帧，`clone_from_slice` 复制进 `SharedPixelBuffer` 后设给 `video-frame`，`host.slint` 再用 `image-fit: contain` 缩放。

依据：

1. **不用重建渲染上下文。** rsmpv 的 `RenderContext::render_software(width, height, format, stride, &mut [u8])` 每次调用自己带尺寸（安全函数，宿主已经在用）。所以改尺寸只是换一块缓冲，不重建 `mpv_render_context`。原型 `sizebench` 模拟窗口拖动，每一帧都换尺寸（1600×900 → 2107×1173，共 40 种）：1080p 来源下没有丢帧，CPU 与固定 1920×1080 时相同（54% 对 55%）。
2. **显示层属性在软件渲染下有效。** 原型 `viewprobe` 把 16:9 来源渲染到 800×800 的目标，统计全黑的行和列：

   | mpv 属性 | 全黑行 / 列 | 含义 |
   | --- | --- | --- |
   | 默认（`keepaspect=yes`） | 350 / 0 | 上下黑边 |
   | `keepaspect=no` | 0 / 0 | 拉伸填满 |
   | `panscan=1` | 0 / 0 | 填满并裁切 |
   | `video-aspect-override=1.3333` | 200 / 0 | 4:3 |
   | `video-zoom=1`（×2） | 0 / 0 | 放大 |
   | `video-zoom=-1`（×0.5） | 575 / 400 | 缩小，四周黑边 |

   所以只要渲染目标与视频区比例一致，R-V9 的结果就正确，这也是需求文档 E11 指出的唯一前提。
3. **CPU 成本**（`/usr/bin/time` 统计整个进程，含约 1 s 启动，24 fps，`hwdec=no`，无音频输出；单位是一个核的百分比）：

   | 渲染目标 | 1080p 来源 | 2160p 来源 |
   | --- | --- | --- |
   | 64×36（≈ 只算解码） | 31% | 55% |
   | 960×540（现状） | 40% | 72% |
   | 1920×1080 | 50–55% | 89–92% |
   | 2880×1620（Retina 全屏级别） | 95–99% | 109–118% |
   | 3840×2160 | 117% | 121–129% |

   RGB24 与 `rgb0` 成本相同：`rgb0` 还要逐像素补 alpha，4K 时每帧多 2.4 ms。所以继续用 RGB24 + `Image::from_rgb8`。每一种尺寸下渲染线程都稳定输出 24 fps、无丢帧。注意 `render_software` 默认阻塞到帧的目标显示时间（rsmpv 不暴露 `BLOCK_FOR_TARGET_TIME`），单帧耗时测不出真实开销，表里用的是进程 CPU。
4. **HiDPI**：Slint `window.scale_factor()`（宿主已在用，安全）乘以逻辑视频区（`host.slint` 的 `video-width` / `video-height`）就是物理像素。本机默认窗口 780×671 逻辑点，视频区约 1560×1000 物理像素，全屏约 2560×1500。
5. **把帧交给 UI 不必复制**：`slint::SharedPixelBuffer<Rgb8Pixel>` 是 `Send` 的（`SharedVector<T>: Send` 当 `T: Send + Sync`，见 i-slint-core `sharedvector.rs`）。渲染线程可以直接往它的字节里渲染，UI 线程只调 `Image::from_rgb8(buffer)`，省掉现在每帧一次的 `clone_from_slice`（4K RGB24 每帧 25 MB）。`slint::invoke_from_event_loop` 是安全函数，可以在帧到达时唤醒 UI，不再靠 16 ms 轮询。

方案（细节见 §4.1）：按物理视频区确定目标尺寸，再用来源分辨率封顶（不在 CPU 上渲染比来源更多的像素，剩下的放大交给 Slint 的纹理采样）；尺寸变化时节流；暂停时尺寸变了要主动重画一帧。

风险：

- **UI 线程纹理上传没有实测**（子 agent 不操作 GUI）。femtovg 每一帧上传一张 RGB8 图：全屏 1080p 是 6 MB，4K 是 25 MB。要在 GUI 测试里看 UI 线程是否跟得上 60 fps（§6.2）。
- 来源分辨率封顶之后，全屏放大由 Slint 双线性采样完成，比 mpv 的缩放器略软。可以接受：比现在的 960×540 放大清楚得多。需求文档的「放大后画质」是对 E13 zoom 而言，zoom > 1 时上限按 zoom 放宽（§4.1）。
- 4K 来源在 Retina 全屏下进程 CPU 超过一个核（约 120–130%，M2 上实时）。这是软件渲染固有的成本，C-GPU（§7.2）之前没有办法；比现在多约 50 个百分点。
- 拖动期间旧帧比例与新视频区不一致，`contain` 会出现一两帧细黑边，不变形。可以接受。

### 1.2 画中画（R-W5、R-H8）：Safe Rust 下**不可行**

依据（`objc2-av-kit` / `objc2-av-foundation` / `objc2-core-media` 都取 crates.io 最新的 0.3.2，与宿主现有的 objc2 0.6.5 / `objc2-*` 0.3.2 同一代；用 `cargo fetch` 取源码检查，并在 `#![forbid(unsafe_code)]` 原型 `pipprobe` 里编译）：

1. **AVKit 全部方法都是 `unsafe fn`**：`objc2-av-kit` 0.3.2 生成的绑定共 119 个 `pub unsafe fn`、0 个安全 `pub fn`。包括 `AVPictureInPictureController::isPictureInPictureSupported`、`initWithContentSource`、`startPictureInPicture`、`stopPictureInPicture`，以及 sample buffer 路径的 `AVPictureInPictureControllerContentSource::initWithSampleBufferDisplayLayer_playbackDelegate`、`invalidatePlaybackState`。
2. **AVFoundation 的 `AVSampleBufferDisplayLayer` 同样全部是 `unsafe fn`**（`new`、`enqueueSampleBuffer`、`flush`、`setControlTimebase`、`isReadyForDisplay`……）。整个 crate 有 3357 个 `unsafe fn`、6 个安全函数（都是 `AVMakeRectWithAspectRatioInsideRect` 这类几何辅助）。objc2 的规则是没有人工审计过的方法一律标 `unsafe`；AppKit 的 `NSWindow` / `NSEvent` 等宿主在用的方法已经审计、是安全的，AVKit / AVFoundation 还没有。
3. **帧的构造没有安全 API**：`CMSampleBuffer::create_ready_with_image_buffer` 是 `unsafe fn`（参数是 `NonNull<CMSampleTimingInfo>` 和输出指针 `NonNull<*mut CMSampleBuffer>`）；`CVPixelBufferCreate` / `CVPixelBufferCreateWithBytes` 在 `extern "C-unwind"` 块里，调用需要 `unsafe`；`CVPixelBufferGetBaseAddress` 本身安全，但只返回 `*mut c_void`，写像素必须 `unsafe`；`CMTimebase` 的创建（`create_with_master_clock`）与 `set_rate` / `set_time` 都是 `unsafe fn`，`CMTimebaseCreateWithSourceClock` 还没有生成（源码里是 `// TODO`）。
4. **委托类**：`AVPictureInPictureSampleBufferPlaybackDelegate` 要用 `define_class!` 实现。实例化时 `msg_send![super(this), init]` 要求 `unsafe`（E0133）。
5. `pipprobe` 实测：调用 `isPictureInPictureSupported`、`AVSampleBufferDisplayLayer::new`、`isReadyForDisplay`、`CVPixelBufferCreate` 与委托的 `init` 都报 `error[E0133]: call to unsafe function … requires unsafe block`。

结论：sample buffer 画中画链路的每一步（判断是否支持、建图层、建控制器、建帧、入队、时间基）都要 `unsafe`，不存在只用安全封装的写法。这正是裁决 R-H8「不行则按 §7 升级」的情况。选项见 §7.1。

附带发现（与本轮实现无关，但影响「禁止 `unsafe`」的检查方式）：`define_class!` 宏里写的 `unsafe impl NSObjectProtocol for X {}` 在 `#![forbid(unsafe_code)]` 下**能编译通过**（lint 不报宏展开出来的代码；原型 probe 3 实测）。所以 `forbid(unsafe_code)` 拦不住所有 `unsafe` 关键字。建议 §6.1 的静态检查加一条 `grep -rn '\bunsafe\b' src/host/src`（结果应为空，`#![forbid(unsafe_code)]` 行除外）；宿主目前没有 `define_class!` / `msg_send!`，现在这条检查的结果为空。

### 1.3 C-GPU（R-H3，4b）：Safe Rust 下**不可行**

依据：

1. **libmpv 只有两种渲染 API**：mpv 0.41 的 `render.h` 只定义了 `MPV_RENDER_API_TYPE_OPENGL` 和 `MPV_RENDER_API_TYPE_SW`，没有 Metal / Vulkan 的嵌入接口（Vulkan / Metal 只在 mpv 自己的窗口里用：`gpu-context=macvk`）。所以「等价的 Metal 路径」不存在，只能走 OpenGL。
2. **rsmpv 的 OpenGL 构造函数是 `unsafe fn`，而且是有意的**：`RenderContext::new_opengl` / `new_opengl_drm` 是 `pub unsafe fn`。文档说明：「GL 上下文必须在创建、每次调用以及 drop 时都是当前上下文」，这是类型系统表达不了的动态条件，违反就是未定义行为。上游没有把它改成安全函数的打算，这不是缺一层封装的问题。
3. **Slint 接入外部 GL 纹理是 `unsafe fn`**：`BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture` 与已弃用的 `Image::from_borrowed_gl_2d_rgba_texture` 都是 `pub unsafe fn`（i-slint-core 1.16.1 `graphics/image.rs` 第 997、871 行），理由是无效的纹理 id 会让驱动出现未定义行为。`set_rendering_notifier` 本身安全，但它给的 `GraphicsAPI::NativeOpenGL { get_proc_address }` 只返回 `*const c_void`，建 FBO / 纹理要把它转成函数指针（例如 `glow::Context::from_loader_function`），同样是 `unsafe`。
4. Slint 有安全的 `wgpu` 纹理导入（`unstable-wgpu-27/28` 特性，`Image::try_from(wgpu::Texture)`），但 mpv 渲不进 wgpu 纹理。要在 macOS 上把 GL 纹理和 wgpu（Metal）纹理打通，得走 IOSurface 互操作，同样是 `unsafe`。
5. 软件路径的替代办法：本机 FFmpeg 有 `tonemap`，但没有 `zscale`（`tonemap` 前后的线性化要靠它）和 `libplacebo` 滤镜。`minterpolate` 有，但需求文档 E18 已经因为 1080p 远达不到实时而排除了它。GLSL 着色器（E17）只能在 GPU 渲染器里用。所以 E16、E17、E18 运动补偿在软件路径上都没有办法。

结论：C-GPU 至少需要 `rsmpv::new_opengl`、GL 调用和 Slint 借用纹理三处 `unsafe`，属于工作流 §7「放宽 Safe Rust」与「既定技术路线满足不了关键场景」，不默认开工。选项见 §7.2。`PlaybackStats.video.transfer`（R-P8）只服务于 E16，随 4b 一起等裁决。

## 2. 范围

只做裁决里标为「4a」、且 §1 结论为可行的条目；不提前做 4b，不新增裁决之外的接口。签名做不到时，维护者在本文 §8.2 记录原因与调整后的签名。

| 类别 | 条目 | 新能力 | 需求 |
| --- | --- | --- | --- |
| 渲染尺寸 | R-H2 C-SIZE：渲染尺寸跟随视频区（节流） | — | E11、E13 的前提；所有画面的清晰度 |
| 渲染节奏 | 裁决 3.5「渲染节奏」：帧到达即交给 UI，不靠 16 ms 轮询；与 R-V8 同批验证 | — | E18 倍帧 48/60 fps |
| 显示层 | R-V9 `plugin.video.view()`（单一拥有者） | 现有 `processing.video` | video E11、E13 |
| 处理节点 | R-V8 `{ kind: "frameBlend"; fps }`（带性能门槛） | 现有 `processing.video` | video E18 倍帧 |

对照裁决 3.5 的 4a 行：接口 2 个中只做 `video.view()`；扩展 2 项中只做 `frameBlend` 节点；宿主行为 3 项中只做 H2 和渲染节奏。

**不在本轮**：

- 画中画（R-W5 `window.setPictureInPicture`、R-H8、`window.state.pictureInPicture`）：§1.2 不可行，等用户裁决（§7.1）。在此之前**不发布** `pictureInPicture` 字段，即使是恒为 `{ supported: false }` 的版本也不发布（与 R-V10「不提前发布恒为 `unsupported` 的接口」同一规则）。window 插件保持 3a 的「迷你尺寸」形态。
- C-GPU（R-H3）、`video.render()`（R-V10）、`transfer`（R-P8）：4b，§1.3 不可行，等用户裁决（§7.2）。

## 3. 阶段与顺序

| 阶段 | 执行 | 内容 | 完成条件 |
| --- | --- | --- | --- |
| P0 | 协调者 | 建分支；记录基线（§8.1）；把 §7 的两个问题转给用户（不阻塞 P1） | 基线写进 §8.1 |
| P1 | 维护者 | §4 全部核心项（都是兼容扩展；C-SIZE 改变的是画面清晰度，不改契约） | 静态检查全过；参考文档、TestHost 同步；协调者审查通过 |
| P2 | 插件作者 | §5 分派 | `check/build/test` 全过 |
| P3 | 协调者 | §6 验收，提交推送，合并到 `main` | — |

P1 分两批，作者按依赖开工：

| 批次 | 内容 | 解锁 | 宿主 Rust 估算 |
| --- | --- | --- | --- |
| 批次 1：渲染 | §4.1 C-SIZE、§4.2 渲染节奏、§4.4 `frameBlend` | video E18 倍帧 | 约 750 |
| 批次 2：显示层 | §4.3 `video.view()` | video E11、E13 | 约 450 |

§4.3 依赖 §4.1（裁决 R-V9：C-SIZE 之前不开放显示层）。§4.4 可以和 §4.1 并行写，但验收必须在 §4.2 之后（裁决 R-V8：与渲染节奏同批验证）。

## 4. 维护者清单（P1）

每一项都要同时完成：宿主实现（`src/host/`）、SDK 类型与实现（`src/sdk/`）、TestHost 模拟（`src/sdk/plugin/testing.ts`）、参考文档（`doc/plugins/reference.md`）、单元测试。第一方 Rust 不得用 `unsafe`（包括宏参数里的 `unsafe impl`，见 §1.2 附带发现）。

### 4.0 体积预算

宿主 Rust（`src/host/src/*.rs`，含测试）净增**约 1 200 行**，上限 3 000 行（裁决 3.5 的每轮预算）。基线为 r3b 结束时的 30 966 行（P0 复测为准）。裁决给 4a 的 3 000 行里约一半是画中画；画中画移出后本轮明显变小，不另外补入别的条目。

| 项 | 宿主改动点 | 宿主 Rust 估算 | SDK / runtime 估算 |
| --- | --- | --- | --- |
| §4.1 C-SIZE | `backend.rs`：渲染目标共享状态、按目标尺寸分配 `SharedPixelBuffer`、尺寸变化时主动重画；`main.rs`：由视频区与缩放因子算目标、节流、封顶；新模块 `render_target.rs`（纯函数：目标计算、封顶、取偶、节流判定，便于单测） | 约 450 | — |
| §4.2 渲染节奏 | `backend.rs` 帧到达后 `invoke_from_event_loop`（合并唤醒，原子标志）；`main.rs` 把取帧从 16 ms 定时器移到唤醒回调；产生 / 上屏计数（诊断日志） | 约 150 | — |
| §4.3 `video.view()` | 新模块 `video_view.rs`（所有者、字段校验、mpv 属性映射、恢复默认、状态四态）；`plugins.rs` 停用清理；协议方法 `video.view.set` | 约 450 | 约 200（含 TestHost） |
| §4.4 `frameBlend` | `effects.rs` 节点（校验、`framerate=fps=N`、整图放在最后、全局最多一个）；`offline_descriptor.rs` 支持该节点 | 约 150 | 约 30（类型） |
| 合计 | | **约 1 200** | **约 230** |

超出处理：批次 1 实测超过 1 200 行时，维护者在 §8.2 写明原因；全轮超过 3 000 行时，把 §4.3 拆到下一轮（E11、E13 继续置灰），§4.1、§4.2、§4.4 不拆。不为了压行数削减测试。

### 4.1 C-SIZE：渲染尺寸跟随视频区（R-H2）

- **目标尺寸**：物理视频区 `(video-width, video-height) × window.scale_factor()`，四舍五入后取偶数，最小 2×2。视频区为 0（最小化、无外壳）时保持上一个目标。
- **来源封顶**：设物理视频区为 A，来源显示尺寸为 S（`video-out-params/dw`、`dh`），显示层缩放为 `zoom`（§4.3，默认 1）。缩放系数 `k = min(1, max(S.w / A.w, S.h / A.h) × max(1, zoom))`，目标为 `A × k`。比例始终等于视频区比例，所以 `keepaspect`、`panscan`、`aspect` 的结果正确。总像素另有绝对上限 3840×2160（8.3 MP），超过时按比例缩小。来源尺寸未知时用 1920×1080 封顶。
  - 理由：§1.1 的表里，渲染像素超过来源以后 CPU 继续上涨，画面却不会更清楚。例如 1080p 来源在 Retina 全屏（约 2560×1500）下：渲 1920×1080 约 55%，渲物理分辨率约 99%。
  - 生成流（NES 等 `producer`）不受影响：它们的帧仍按自己的尺寸交给 UI。
- **节流**：拖动窗口时至多每 50 ms 应用一次新目标，停止变化后 50 ms 内一定应用最终值；全屏切换、`ScaleFactorChanged`（窗口拖到另一块屏幕）立即应用。尺寸变化不重建 `RenderContext`（§1.1 第 1 条），只重新分配缓冲。
- **暂停时**：目标变化后渲染线程主动调一次 `render_software`（mpv 会重画当前帧），不等 `update()`，否则暂停中拖动窗口，画面会停在旧比例上。
- **交给 UI**：渲染线程直接渲进 `SharedPixelBuffer<Rgb8Pixel>`（RGB24，stride = 宽 × 3），UI 线程 `Image::from_rgb8(buffer)`，不再 `clone_from_slice`。`Frame` 改为携带缓冲本身，`sequence` 保留。
- `host.slint` 视频图像保持 `image-fit: contain`（拖动期间旧帧比例不同时不会变形）。
- 方向处理（`source_geometry.rs` 的发布闸门）、截图与离线导出（离线 ffmpeg）、`--frame-evidence-dir` 诊断都不受影响；诊断按实际帧尺寸写 PPM。
- 统计：`PlaybackStats.backend` 仍为「libmpv · 软件渲染」；不新增字段。
- 单测：目标计算（HiDPI、封顶、取偶、来源未知、zoom > 1 放宽、绝对上限）；节流判定（连续变化、最终值、全屏立即）；暂停时尺寸变化会重画（用渲染线程的假时钟或计数）。

### 4.2 渲染节奏（裁决 3.5，与 R-V8 同批验证）

- 现状：UI 线程用 16 ms（62.5 Hz）的 `slint::Timer` 轮询帧槽。48 / 60 fps 输出时，两者节拍不同，帧会被跳过或重复。
- 改为：渲染线程放入新帧后，如果「已请求唤醒」标志为假，就置真并 `slint::invoke_from_event_loop(…)`；回调里清标志、取帧、设 `video-frame`。同一时间至多一个唤醒在途，帧槽仍只保留最新一帧（UI 慢的时候丢旧帧，不排队）。16 ms 定时器保留其它工作，不再取视频帧。
- 诊断：宿主内部计数「渲染产生的帧」和「设到 UI 的帧」，环境变量 `VESTRA_FRAME_LOG=1` 时每秒往 stderr 打一行（不进公开 API，GUI 验收用，§6.2）。
- 主窗口最小化或被遮挡时，渲染线程照常按 mpv 节奏渲染（与现在相同）。

### 4.3 显示层 `plugin.video.view()`（R-V9，现有能力 `processing.video`）

```ts
plugin.video.view(): VideoView;   // 需 processing.video；第一个调用的插件拥有显示层，别的插件再调用以 conflict 失败；同一插件第二次调用抛错
interface VideoView extends Disposable {
  set(value: Partial<ViewSettings>): void;      // 合并写入，最后写入生效；失败交 onError 与 status
  readonly status: FilterStatus;                // 与滤镜相同的四态
  onStatus(listener: (s: FilterStatus) => void): Disposable;
}
interface ViewSettings {
  aspect: number | null;              // null = 按来源；0.25–4 → video-aspect-override（null 写 -1）
  fit: "keep" | "stretch" | "fill";   // keep: keepaspect=yes, panscan=0；stretch: keepaspect=no, panscan=0；fill: keepaspect=yes, panscan=1
  zoom: number;                       // 0.25–4 → video-zoom = log2(zoom)
  panX: number; panY: number;         // −1–1 → video-pan-x / video-pan-y（单位为视频宽 / 高）
}
```

- 签名与裁决 R-V9 一致（没有 `scaleX` / `scaleY`）。`conflict` 的语义与 `provideSequence` 相同：拥有者 `dispose` 或停用后，别的插件才能取得。
- 默认值：`{ aspect: null, fit: "keep", zoom: 1, panX: 0, panY: 0 }`。拥有者 `dispose` 或停用时，核心把五个属性写回默认值。
- 换来源时核心**保留**当前值（mpv 的这些属性跨 `loadfile` 保持）。E13「换片源时复位」由插件自己在 `sourceId` 变化时 `set({ zoom: 1, panX: 0, panY: 0 })`；参考文档写明。
- 状态：没有就绪媒体时为 `waiting`（值保留，就绪后应用）；写 mpv 属性期间 `applying`；成功 `applied`；mpv 拒绝时 `failed`。范围外的值同步以 `invalid` 失败，不改变已应用的值。
- `zoom` 同时进入 §4.1 的封顶计算（放大时渲染更多像素，避免放大后发糊）。
- 显示层效果不进截图和导出（截图走离线 ffmpeg，本来就不经过 vo 属性）；单测断言离线描述里没有这些字段。
- 协议：新方法 `video.view.set { values }`（宿主按调用插件判断所有权），返回 `{ state }`；所有权在第一次 `set` 或 SDK 激活 `view()` 时取得，由维护者定，写进 §8.2。
- TestHost：`host.videoView`（拥有者与当前值）；`host.setVideoViewOwner(pluginId)` 模拟别的插件先占用（让 `view()` 得到 `conflict`）。

### 4.4 `frameBlend` 节点（R-V8，现有能力 `processing.video`）

```ts
{ kind: "frameBlend"; fps: number }   // 24–120 的整数；FFmpeg framerate=fps=N；非运行时字段（改值时重建本阶段）
```

- 全部插件合计最多 1 个 `frameBlend`，第二个 `invalid`（帧率只能改一次）。核心把它放在整条视频图的**最后**（所有插件的链之后），保证时间域滤镜（降噪、反交错）在原帧率上工作。参考文档 §8 的节点表加一行，并写明这条位置规则。
- 「含效果」截图与导出用同一条链：导出视频的帧率随之改变（裁决 R-V8 的验收项）；`offline_descriptor.rs` 要支持该节点，单测断言导出描述里有 `framerate=fps=N`。
- 实测（§1.1 原型，`vf=lavfi=[framerate=fps=N]`，1920×1080 目标）：1080p 来源 24 → 48 fps，CPU 53% → 90%；24 → 60 fps，110%。4K 来源 24 → 48 fps，93% → 162%；24 → 60 fps，200%。渲染线程实际输出 48.0 / 60.0 fps，`frame-drop-count` 为 0。UI 侧要在 §4.2 之后用 GUI 实测。
- `PlaybackStats.video.fps` 读 `estimated-vf-fps`，所以打开倍帧后显示 48 / 60，与需求文档 E18 的验收方式一致；维护者在单测或 headless 运行里确认该属性反映滤镜之后的帧率。

### 4.5 文档

- 参考文档：§5.4 补一句「渲染尺寸跟随视频区，按来源分辨率封顶」（不改字段）；§8 节点表加 `frameBlend`、位置规则与「最多一个」；新增小节「显示层 `plugin.video.view()`」（字段、映射、所有权、默认值、换来源保留、不进截图）；§14 TestHost 新增项；§17 本轮变更说明（C-SIZE 改变的是画面清晰度与 CPU 占用，不改契约；画中画与 C-GPU 等用户裁决）。
- 示例：`video.view()` 宽高比切换、`frameBlend` 开关各一个短例（≤ 30 行）。

## 5. 插件接入分派（P2）

作者规则同 r3b §4：只写自己的插件目录；按需求文档对应小节实现；缺核心能力时按工作流 §4.1 写请求，同时继续不依赖它的部分；降级项按裁决实现，并修改需求文档对应小节。

### 5.1 分派表

| 作者 | 插件 | 新功能（编号 → 依赖） | 迁移与清理 | 降级与文档 | 工作量 |
| --- | --- | --- | --- | --- | --- |
| A | video | E18 倍帧（`frameBlend`，目标 = min(原帧率 × 2, 60)，原帧率读 `stats().video.fps`，≥ 50 时不加节点并显示「当前帧率已足够」；5 秒内丢帧 > 5% 时提示一次）；E11 宽高比与显示模式（`video.view()` 的 `aspect`、`fit`；「比例 ▶」菜单、Ctrl+F6、「自定义比例…」小窗）；E13 帧位缩放平移（`zoom`、`panX`、`panY`；「帧位 ▶」菜单与小键盘 / 备用键；换来源时复位）；「画面」页宽高比、显示、缩放三行与「效果」页倍帧复选取消置灰 | 删除 `main.ts` 里倍帧的 `LATER` 禁用项；README 去掉「放大的是 960×540 缓冲」的已知限制 | 运动补偿、HDR、着色器继续禁用，原因不变（「需要 GPU 渲染，当前版本不支持」），等 §7.2；E18「两者互斥」的逻辑在运动补偿可用前不触发 | 中高（倍帧在批次 1 之后；E11、E13 在批次 2 之后） |
| — | window | 无（H07 保持 3a「迷你尺寸」形态） | — | 需求文档 H07 补一句「画中画等用户裁决（第 4a 轮 §7.1）」，由协调者修改 | — |

只有 1 个作者。别的插件没有本轮条目；C-SIZE 让所有画面变清楚，§6.2 用回归场景覆盖。

### 5.2 准入

- 批次 1 审查通过后：A 开工 E18 倍帧，同时做 E11、E13 不依赖核心的部分（菜单、自定义比例小窗与校验、小键盘绑定、持久化、TestHost 测试）。
- 批次 2 审查通过后：A 接上 `video.view()`。

### 5.3 需求文档按裁决改写（作者 A 改 video 章节）

| 章节 | 需求文档现写法 | 按裁决与本轮改为 |
| --- | --- | --- |
| E11 核心依赖 | `VideoView` 含 `scaleX`、`scaleY`；「多插件，最后写入生效」 | 去掉 `scaleX`、`scaleY`；单一拥有者，第二个插件 `conflict` |
| E13 核心依赖 | 引用 `scaleX`、`scaleY`；「C-SIZE 完成之前放大的是 960×540 缓冲」 | 去掉两字段；删掉那句已知限制（C-SIZE 已完成，放大时渲染尺寸随 `zoom` 放宽） |
| E18 | 「运动补偿第 4b 轮」 | 「运动补偿等用户裁决（第 4a 轮 §7.2）」 |

## 6. 验收

### 6.1 静态检查与测试

工作流 §6.1 的全部命令；各插件测试数不减少；菜单快照只多出本轮新增项（video 的「比例 ▶」「帧位 ▶」，倍帧由禁用变为可用）；新增 `grep -rnw unsafe src/host/src`，除 `#![forbid(unsafe_code)]` 行外结果为空（§1.2 附带发现）。

### 6.2 真实 GUI 测试（协调者）

工作流 §6.2 的全部必测场景，加上本轮：

| 范围 | 场景 | 证据 |
| --- | --- | --- |
| C-SIZE 清晰度 | 1080p 测试图（细线、小字）在默认窗口与全屏下截图，与本轮开始前的截图对比：细线不再糊成 960×540 的放大效果 | 前后截图 |
| C-SIZE 拖动 | 播放 1080p 时连续拖动窗口角 10 秒：画面不卡住、不变形，最多一两帧细黑边；暂停时拖动，画面跟随新比例重画 | 录屏或连续截图；`VESTRA_FRAME_LOG` 输出 |
| C-SIZE HiDPI | 窗口在 Retina 屏与外接 1x 屏之间拖动（有外接屏时）：画面清晰度随之变化，没有错位 | 截图 |
| C-SIZE CPU | 1080p 与 4K 来源各播 1 分钟（默认窗口、全屏），活动监视器记录宿主 CPU，与 §1.1 的表对照（允许 ±20 个百分点）；不丢帧（`stats().framesDropped` 不增长） | CPU 读数、stats |
| 渲染节奏 / 倍帧 | 24 fps 平移片段打开倍帧：`stats().video.fps` 为 48；`VESTRA_FRAME_LOG` 显示「上屏」≥ 产生帧的 95%；画面观感更平滑；60 fps 片源打开倍帧时链中没有节点；4K 来源打开倍帧，丢帧提示只出现一次 | 日志、stats、截图 |
| 倍帧导出 | 打开倍帧后「含效果」导出 5 秒片段：`ffprobe` 显示 48 fps | `ffprobe` 输出 |
| 显示层 / E11 | 16:9 片源选 4:3，左右黑边；窗口拖成接近 1:1 后选「不保持宽高比」，没有黑边；「填满窗口」没有黑边且有裁切；自定义 21:9 进入切换序列；含效果截图的尺寸比例不受影响；停用 video 后恢复按来源显示 | 截图 |
| 显示层 / E13 | Numpad9 / Alt+Plus 放大到 200% 后画面清楚（不是放大的低分辨率缓冲）；平移、复位、居中都有 OSD；按住放大到 400% 停止；换来源后复位；截图仍是未缩放画面 | 截图 |
| 显示层所有权 | 临时探针插件在 video 之前调用 `view()`：video 的「比例 ▶」各项失败并显示原因；停用探针后 video 可以取得 | 截图、日志 |
| 回归 | NES 出画面并正常清屏；背景层（visualizer）在无视频时显示；侧栏浮动 / 附着切换后画面正常；字幕位置随视频区正确 | 截图 |

缺陷归属同工作流 §6.2。

## 7. 需要用户裁决

两个问题彼此独立。在用户回答之前，只暂停依赖它的工作（工作流 §7）：本轮的 §4、§5 不受影响，照常进行。

### 7.1 画中画（H07 / R-W5 / R-H8）

> **用户裁决（2026-10-21）**：先做原型再决定。原型放在独立子包 `src/host/unsafe/pip/`（只有它允许 `unsafe`，宿主主包保持 `#![forbid(unsafe_code)]`，静态检查要求 `unsafe` 只出现在 `src/host/unsafe/` 下，每个 `unsafe` 块写 SAFETY 注释），对外只给安全接口，上限 500 行。原型报回 `unsafe` 块个数与行数，由用户决定是否正式做；否则保留迷你尺寸。

**问题**：macOS 系统画中画只能经 AVKit 的 `AVPictureInPictureController` + `AVSampleBufferDisplayLayer` 实现，而目前可用的 Rust 绑定（objc2 0.3.2 系列）把这条链路上的每个方法、以及 `CMSampleBuffer` / `CVPixelBuffer` / `CMTimebase` 的构造都标成了 `unsafe`（§1.2）。在「第一方 Rust 禁止 `unsafe`」之下做不出来。

| 选项 | 做法 | 代价 |
| --- | --- | --- |
| A. 不做系统画中画 | H07 永久采用 3a 已上线的「迷你尺寸」（缩小视频区 + 置顶）；删掉 `pictureInPicture` 字段与 W5 | 核心零成本。失去系统画中画（盖在全屏应用之上、跟随切换空间、系统管理位置）。用户已经有一个可用的替代 |
| B. 允许一个隔离的 `unsafe` 模块 | 宿主新增一个模块（或 workspace 内单独的 crate，例如 `vestra-pip`），只有它 `#![allow(unsafe_code)]`，其余模块保持 `forbid`。每个 `unsafe` 块写 `// SAFETY:`，单测覆盖构帧与时间基；对外只暴露安全的 `Pip::start/stop/enqueue(frame, pts)` | 改变已定原则「第一方 Rust 禁止 `unsafe`」，开了先例。估算宿主约 1 200–1 600 行（其中 `unsafe` 模块约 500 行），每帧 RGB→BGRA 拷贝一次；需要人工审查内存与线程安全（委托回调在主线程、帧在渲染线程）。可以作为第 4c 轮单独交付 |
| C. 进程边界（不写 Rust `unsafe`） | 用 Swift 写一个小的辅助进程（无 Dock 图标），由宿主启动；帧经共享内存或管道传过去（960×540 BGRA 30 fps 约 60 MB/s），播放 / 暂停 / 返回经 stdin/stdout JSON 往返 | 第一方 Rust 仍然没有 `unsafe`，但引入第二种原生语言、Swift 工具链与额外的签名 / 打包步骤（工作流 §7「既定技术路线」）。跨进程同步时间戳更难，音画同步验收（±40 ms）有风险。估算 Swift 约 500 行 + 宿主约 600 行 |
| D. 等上游 | 向 objc2 提交审计，把 AVKit / AVFoundation 里画中画用到的方法标为安全；`CMSampleBuffer` / `CVPixelBuffer` 构造需要上游增加手写的安全封装 | 时间不确定（CF 输出指针类函数的安全封装目前没有排期），期间 H07 保持迷你尺寸，等于先按 A 执行 |

维护者推荐：**A**，同时把 D 作为长期跟踪项。理由：迷你尺寸已经覆盖了「小窗看片」的主要用途；B 和 C 都只为一个功能改变原则或技术路线，代价明显大于收益。

需要用户回答的一句话：**画中画选 A（只保留迷你尺寸）、B（允许隔离的 `unsafe` 模块）、C（Swift 辅助进程）还是 D（等上游，期间按 A）？**

### 7.2 C-GPU（4b：R-H3、R-V10、R-P8；需求 E16、E17、E18 运动补偿）

> **用户裁决（2026-10-21，修订）**：GPU 渲染重要（主流 macOS 播放器与 PotPlayer 都支持；还影响 4K 的 CPU 占用与硬件解码零拷贝），改为先做原型：放在独立子包 `src/host/unsafe/render/`，隔离规则同 §7.1，上限 500 行，验证 libmpv OpenGL 渲染到纹理并由 Slint 显示。原型报回：`unsafe` 块个数与行数、4K 播放 CPU 对比软件渲染、能否满足播放连续性（换来源、重建界面不中断）。正式切换时保留软件渲染作为后备。E16/E17/E18 运动补偿视原型结论恢复。

**问题**：GPU 渲染只能走 libmpv 的 OpenGL 渲染接口（mpv 0.41 没有 Metal / Vulkan 嵌入接口）。rsmpv 的 `new_opengl` 有意设计为 `unsafe fn`，Slint 接入外部 GL 纹理（`BorrowedOpenGLTextureBuilder`）也是 `unsafe fn`，GL 调用本身同样需要 `unsafe`（§1.3）。

| 选项 | 做法 | 代价 |
| --- | --- | --- |
| A. 放弃 E16 / E17 / E18 运动补偿 | 保持软件渲染；`video.render()`、`transfer` 不做；video 插件把这三项从菜单移除，或保留禁用并写永久原因 | 核心零成本，Safe Rust 原则不变。失去 HDR 色调映射（HDR 片源在 SDR 屏上发灰）、自定义 GLSL 着色器、运动补偿插帧；硬件解码（`hwdec`）与更高质量的缩放也一并没有。倍帧（4a）仍可用 |
| B. 允许一个隔离的 `unsafe` 渲染模块 | 一个模块 / crate 持有 `unsafe`：在 Slint 的渲染通知（femtovg = OpenGL）里建 FBO 与纹理，调 `rsmpv::new_opengl` / `render_opengl`，用 `BorrowedOpenGLTextureBuilder` 交给视频区；渲染要在 UI 线程（Slint 的 GL 上下文）上完成，或者建共享上下文放到单独线程（更多 `unsafe`） | 改变已定原则。估算宿主 2 000–3 000 行（其中 `unsafe` 约 400–600 行），单独一轮。技术风险：依赖 Slint 在 macOS 上继续用 OpenGL 渲染器（femtovg-wgpu、Skia 已经存在，默认渲染器一换就要重写）；macOS 已弃用 OpenGL（仍可用，最高 4.1）；多窗口（浮动侧栏、画中画）不能共享借用纹理。好处：`hwdec=videotoolbox`、gpu-next、E16/E17/E18 全部可做，4K CPU 大幅下降 |
| C. mpv 原生子视图（`wid`） | 把 mpv 的 `wid` 设为视频区的 `NSView` 指针（整数转换，Rust 侧不需要 `unsafe`），用 `vo=gpu-next`、`gpu-context=macvk` 让 mpv 自己画 | mpv 0.41 手册里的 `--wid` 只写了 X11、win32、Android，macOS 下 libmpv 是否支持**未验证**，需要协调者做 GUI 原型。即使能用：Slint 画的屏幕提示、信息块、背景层、全屏浮动底栏都画不到原生子视图上面，要把这些叠加层改成单独的子窗口，等于重做视频区组合（r3b 背景层、拖放命中都要改）。风险最高 |
| D. 等上游 | 等 mpv 提供 Vulkan / Metal 渲染 API，并有 Slint 安全的 wgpu 纹理导入（Slint 已有 `unstable-wgpu-28`）与 mpv→wgpu 的安全互操作；或者等 rsmpv / Slint 提供安全的 GL 纹理共享 | 时间不确定（rsmpv 与 Slint 都把 `unsafe` 写成了有意的设计，不是缺封装），期间等于 A |

维护者推荐：**A**（结论记为「暂不做」，D 长期跟踪）。如果用户认为 HDR 播放是关键场景，推荐 **B**，并先做一个不超过 500 行的 `unsafe` 原型轮，确认 Slint femtovg 渲染通知里的帧时序与多窗口限制之后，再决定是否投入整轮。

需要用户回答的一句话：**C-GPU 选 A（放弃 HDR / 着色器 / 运动补偿）、B（允许隔离的 `unsafe` 渲染模块，先做原型）、C（试 mpv 原生子视图）还是 D（等上游，期间按 A）？**

## 8. 记录

### 8.1 基线

（P0 填写：静态检查、各插件测试数；宿主 Rust `src/host/src/*.rs` 行数——撰写本任务单时为 30 966；SDK 与运行器行数）

### 8.2 维护者交付与签名调整

（P1 每批交付时填写：实现清单、签名调整与实现取舍、检查、宿主 Rust 实测净增、给协调者的 GUI 实测要点。至少要记录：封顶规则的实际取值、节流参数、`video.view` 所有权在何时取得、`frameBlend` 在离线导出中的实测帧率、`estimated-vf-fps` 是否反映倍帧）

### 8.3 用户裁决

（协调者填写 §7.1、§7.2 的回答与日期；按回答更新需求文档 H07、E16–E18 与 4b 排期）

### 8.4 作者交付

（作者完成时填写）

### 8.5 验收结果

（P3 填写）
