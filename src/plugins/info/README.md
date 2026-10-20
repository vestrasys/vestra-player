# 播放信息（vestra.info）

右键菜单顶层「播放信息」（勾选项，`windows` 组 order 810）或 Tab 开关视频左上角的只读信息层（默认关），显示期间每秒刷新一次。内容（整改文档 §9.2）：

```
文件：movie.mp4
时间：00:05 / 01:40
视频：h264 1920×1080
帧率：24.00 fps
音频：aac 48000 Hz 2 声道
后端：libmpv · 软件渲染（会话 3）
已显示帧：120
丢帧：1
```

未打开媒体时文件名为「未打开媒体」，其它未知项为「-」。文件名取路径最后一段；网络地址取路径最后一段，没有则取主机名。

简要信息（I03，无菜单项）：按 Shift+Tab 在同一信息块显示两行，3 秒后消失，显示期间再按重新计时，时间每秒刷新：

```
movie.mp4
01:23 / 45:00 · H.264 1920×1080 · AAC 2 声道
```

编码用简短名称（H.264、HEVC、AV1、AAC 等，不用 mpv 的长描述）；每行最多 44 列（中文算 2 列，按最窄视频区 318px 估算），过长时末尾用「…」省略。未知的项整项省略；直播（时长未知）写「01:23 / 直播」；未打开媒体只显示「未打开媒体」。完整信息开着时两块同时显示，简要信息在上，中间空一行；到时只移除简要信息。

播放属性（I05）：右键菜单顶层「播放属性…」（`windows` 组 order 811）或 Ctrl+F1 打开工具窗口 `ui/properties.slint`（460×420，最小 400×320），已打开时再按只带到前面。三个页签，上次的页签记在 `info.json`（`plugin.storage`）：

- 播放信息：`player.stats()` 的扩展字段（解码器、输入格式、源 → 显示尺寸、帧率、码率、采样格式、声道布局、音画同步偏差、缓冲时长），窗口打开期间每秒刷新。「硬件解码」固定「未使用」（播放器固定软件解码）。
- 文件信息：本地文件用 `plugin.tools.run("ffprobe", ["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json", 路径])`（超时 10 秒），每个来源读一次，「刷新」重读，换来源时中止旧的读取；列出来源（路径、大小、修改时间）、容器、逐条流、章节数、元数据、内嵌资源（附件文件名与 MIME、封面）。缺少 ffprobe 显示「无法读取文件信息（未找到 ffprobe）」。网络来源不调用 ffprobe，只列播放器报告的信息。
- 系统信息：Vestra 版本（`plugin.app.version` 与构建哈希，如「0.1.0（构建 289b26b）」）、插件 API 版本、mpv 版本（`plugin.app.components.mpv`）、FFmpeg 版本（`components.libav`：libmpv 报告的 `ffmpeg-version`，有其它 `libav*` 键时依次列出），核心不知道的显示「未知」；macOS 版本、机型、CPU（`sw_vers`、`sysctl`，`sysinfo.ts`）、内存总量与可用量（Node `os`）、显示器（`system_profiler SPDisplaysDataType -json`）。

底部「复制到剪贴板」（`plugin.clipboard.writeText`，提示「已复制播放信息」，失败时状态行「复制失败」）、「保存为…」（默认名 `<文件名> 信息.txt`，临时文件加改名）、「关闭」。复制和保存的都是当前页签的纯文本：分组标题一行，`键：值` 一行一条，分组之间空行；每个值最多 512 字符。

窗口用「关闭」按钮或标题栏关闭（`Window.onClose`）时，「播放信息」的每秒 `stats()` 轮询和进行中的 ffprobe / 系统信息读取都停止；再次打开时重新开始。

关于 Vestra（I09）：右键菜单顶层「关于 Vestra…」（`windows` 组 order 880）或 F1 打开工具窗口 `ui/about.slint`（固定 360×420），已打开时再按只带到前面。内容：logo（`ui/logo.png`，由 `src/assets/brand/logo.png` 缩成 192×192，编译期 `@image-url`）、「Vestra」、版本行「版本 0.1.0（构建 289b26b）」；组件版本：插件 API、mpv、FFmpeg、Slint（`plugin.app`）、Node（插件进程的 `process.versions.node`），拿不到的显示「未知」；已加载插件列表（id、版本、来源、状态，按 id 排序，行高 18、显示 8 行、超出滚动）。底部「复制信息」（全部文字放进剪贴板，提示「已复制版本信息」，失败时状态行「复制失败」）、「关闭」。

已知限制：插件列表要用核心的 `packages.list` / `onChange`（第 3a 轮批次 2，尚未交付），现在列表区只显示「未知（播放器尚未提供插件列表）」，复制的文字里也是这一行。格式化（`pluginRows`）已写好并有测试，接上后只需在 `about.ts` 里读列表并订阅变化。

用到的接口：`plugin.osd.panel()`（能力 `ui.osd`）、`plugin.player.stats()`（`playback.read`）、命令 `bindings`；I05、I09 另用 `windows.create`（`ui.window`）、`plugin.app`、`dialogs.saveFile`（`ui.dialog`）、`clipboard.writeText`（`clipboard`）、`tools.run`、`storage.json`。信息层的底色、字体与位置由宿主决定。
