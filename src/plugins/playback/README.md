# 基础播放（vestra.playback）

功能：主窗口底部控制栏（`ui/bar.slint`，PotPlayer 样式两行 62px）与基础动作之外的播放命令：打开、跳转第 2–4 档、关键帧、跳转到时间（独立小窗）、固定倍率与自定义速度、跳到开头等。播放/暂停、停止、第 1 档跳转（←/→）、上一帧/下一帧、上一个/下一个、减速/加速/正常、音量 ±、静音是**核心动作**（参考文档 §5.5），由播放器注册；底栏按钮用 `plugin.host.run` 执行它们，与快捷键同一行为和提示。插件停用后这些键照常可用。本插件自己的操作结果和失败显示为屏幕提示（`plugin.osd.show`）。

控制栏：

- 进度条行 22px：2px 进度线（已播放黄色）+ 8px thumb；最右端静音图标 13×10 + 固定 70px 音量条。
- 按钮行 40px：播放/暂停（`play-pause`）、停止（`stop`）、后退（`seek-backward`，右键 `previous`）、前进（`seek-forward`，右键 `next`）、打开，五个 41×40 图标按钮；静音图标为 `mute`；音量条要连续值，仍用 `player.setOutput`（规则与核心音量 ± 相同）。按钮可用状态从 `player.state` 推出（Slint `Path` 自绘）；时间文字点击切换已播放/剩余；右侧 23×22 播放列表开关（`plugin.panels.sidebar`，与谁挂载侧栏无关），窗口窄于 320px 时隐藏。

菜单与按键（按 doc/process/rounds/2026-10-plugin-ui.md §3 分配表）：

- 顶层：`打开文件…`（Ctrl+O、F3）、`打开 → 打开链接…`（Ctrl+U，110）、`重开当前/最后文件`（Ctrl+Y，121）、`复制到剪贴板 → 文件名 / 文件夹路径 / 完整路径`（125–127）。
- `播放 → 跳转 →` 上一关键帧 / 下一关键帧（Ctrl+Shift+←/→，206–207，`frame` 组，按住重复）、后退/前进 N 秒第 2–4 档（`backward-2…4` / `forward-2…4`，Ctrl+←/→、Shift+←/→、Ctrl+Alt+←/→，212–217，`step` 组）、跳到开头（Backspace，219，`points` 组）、时间跨度设置…（223，`jump-settings` 组）；`播放 →` 跳转到…（G，218）；`速度 →` 0.5、1.5、2 倍（单选）/ 自定义…；播放时禁止屏保（240，勾选）、进度条悬停显示时间（242，勾选）、进度条悬停显示缩略图（243，勾选），三者在 `playback-options` 组。
- 核心注册（不在本插件）：关闭（F4）、播放/暂停（Space）、上一个/下一个、上一帧/下一帧（D/F）、后退/前进第 1 档（←/→）、正常/减速/加速（Z/X/C）、音量 ±（↑/↓、滚轮）、静音（M）。

功能（doc/product/feature-requirements.md「播放控制」A02、A10、B06、B08、B09、B29、I06、I07、I08，以及 marks 章 B22 的绘制侧；B07 由核心的上一帧/下一帧承担）：

- 打开链接（`open-url.ts` + `ui/open-url.slint`）：输入框与「打开」「取消」是宿主 UI 库的 `UrlInputRow`（UI-DLG-02，与播放列表「添加链接…」同一组件；打开期间 `enabled: false`、确定按钮为「正在打开…」）；http/https/rtsp/rtmp/rtmps/mms/ftp、`file://` 与绝对路径；打开时可取剪贴板首行（`plugin.clipboard.readText`，读不到当作没有），否则用本次运行上次输入的文本。`player.open` 无条件替换当前来源。
- 重开当前/最后文件（`reopen.ts`）：有媒体来源时按原暂停状态重新载入并跳回原位置；没有来源时打开 `last.json`（`plugin.storage.json`）记下的最后一个媒体来源（任何插件打开的、第一次就绪时记录；插件流不记）。
- 播放时禁止屏保（`main.ts`）：开关原样交给 `plugin.power.preventSleep("playing" | "off")`，由宿主在播放时持有断言（有视频 `-d -i`、纯音频 `-i`），暂停、播完、关闭、停用时释放。启动 caffeinate 失败只写在播放器日志里，插件不提示（参考文档 §16.2）。
- 进度条悬停显示时间：`ui/bar.slint` 用 `duration` 在 Slint 内算出指针处的时间，提示框用负 `y` 画在底栏上方的视频区（参考文档 §6.3，宿主不裁切），不经插件往返。
- 进度条悬停显示缩略图（I07，`thumbs.ts`，默认关）：底栏在悬停位置变化时报 `hover-changed(fraction)`（Slint 里 50 ms 冷却），离开时 `hover-ended()`。时间按时长的 0.5%（至少 2 秒）分槽；命中缓存立即显示，未命中等指针停 80 ms 后 `tools.run("ffmpeg", ["-ss", t, "-i", 路径, "-frames:v", "1", "-vf", "scale=160:90:…", …])`（3 秒超时），同一时间只有一个进程，跑完时指针已移走则只入缓存、接着做最新目标。图片 `dataDirectory/thumbs/<realpath + mtime 的哈希>/<槽>.jpg`，用 `plugin.ui.image` 显示；每来源至多 200 张（最久未用的先删，正在显示的不删），换来源时删目录并中止其 ffmpeg，启动时清空 `thumbs/`。只对本地绝对路径且 `stats().video` 非空的来源（与关键帧共用一次 `stats()`）；单张失败不提示，连续 5 次失败后本来源不再尝试。找不到 ffmpeg 时菜单灰、原因「需要 ffmpeg」（启动时查找，缺失期间每次换来源再查；运行时报 `unavailable` 也会变灰）。
- 进度条标记（B22 绘制侧，`bar.ts` + `ui/bar.slint`）：读 `plugin.timeline.marks`（各插件的层，已按时间排序），换算成时长比例传给底栏；1×6 竖线跨在进度线上，章节 `Theme.text-dim`、书签 `Theme.accent`、`marker`（如 marks 的 A 点）`Theme.playing`。`PlayerState.loop` 画成进度线下方 2px 的 `Theme.playing` 色块与 A、B 两条 1×8 竖线（不管谁设的循环）。悬停在标记 ±3px 内时，时间提示改为「标题  01:23」（标记的时间；无标题只显示时间），在 Slint 里用固定 13 步的二分查找找最近的标记（因此至多传 8192 个标记），不经插件往返。标记不接收点击。
- 跳转秒数（`time-jump.ts` + `ui/time-jump.slint`）：四行秒数（0.1–3600，最多一位小数），每行「默认值」；非法行标红、窗口不关。第 1 档用 `player.setStepSeek` 交给核心（插件停用后核心回到 5 秒），第 2–4 档命令放在子作用域里，改值后按新标题重新注册。
- 关键帧（`keyframes.ts`）：`player.step("keyframe", …)`，以当前状态为条件；每个来源第一次就绪时查一次 `stats()`，缓存有没有视频（纯音频禁用，原因「没有视频」；插件流和不可跳转的来源禁用）；一次只走一步，在途时的重复按键丢弃。提示「定位: 下一关键帧 00:12 / 01:40」、「已是最后一个关键帧」、「定位: 开始处」。
- 复制到剪贴板（`copy.ts`）：`plugin.clipboard.writeText`（能力 `clipboard`）。
- 设置 `settings.json`：`openUrl.pasteFromClipboard`、`preventSleep`、`hoverTime`（默认都为 `true`），`hoverThumbnail`（默认 `false`），`seekSteps`（默认 `[5, 30, 60, 300]`，长度不对或有值越界时整组用默认值）；用 `plugin.storage.json` 读写（不是对象的文件改名为 `.bad-<时间戳>`，单个坏字段回到默认值）。测试用 `TestHost` 的核心动作（`coreCommands`、`coreAction(id)`）及 `clipboard`、`power`、`coreRuns`、`stepSeek`、`steps`。

结构：`main.ts` 组装；`controller.ts` 本插件的播放动作（拖动、第 2–4 档跳转、倍率、音量条、跳到开头）、本地草稿与提示文字；`commands.ts` 菜单命令表；`bar.ts` 底栏属性与回调（核心动作走 `host.run`）；`keyframes.ts`、`time-jump.ts` 见上；`prompt.ts` + `ui/prompt.slint` 跳转到/自定义速度小窗；`format.ts` 时间格式。

限制：

- 拖动是一个手势（`player.scrub()`）：期间任何外部来源变化都会结束手势，之后的拖动点被忽略直到下一次按下；取消只丢弃未发送的点，已被宿主接受的跳转不撤销。
- 跳转到/自定义速度小窗属于打开时的来源：换了来源窗口自动隐藏，迟到的确认被拒绝；同一来源内被暂停、跳转不影响，确认作用于当前状态。
- 插件停用时当前播放、已提交的速度/音量/暂停保持不变。

开发（在 `src/` 下）：`npm run plugin -- check|build|test playback`。
