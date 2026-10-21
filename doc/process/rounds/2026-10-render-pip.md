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
