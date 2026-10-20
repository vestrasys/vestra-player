# 基础播放（vestra.playback）

功能：主窗口底部控制栏（`ui/bar.slint`，PotPlayer 样式两行 62px）与菜单命令：打开/关闭来源、载入失败后重新打开、播放/暂停（播完后重播）、上一个/下一个（`player.previous/next`）、进度条拖动跳转、±5/30/60/300 秒、跳转到时间（独立小窗）、倍速（正常/±0.1/固定倍率单选/自定义小窗）、音量 0–200%（±5）与静音。操作结果和失败都显示为屏幕提示（`plugin.osd.show`）。

控制栏：

- 进度条行 22px：2px 进度线（已播放黄色）+ 8px thumb；最右端静音图标 13×10 + 固定 70px 音量条。
- 按钮行 40px：播放/暂停、停止、后退 5 秒（右键：上一个）、前进 5 秒（右键：下一个）、打开，五个 41×40 图标按钮（Slint `Path` 自绘）；时间文字点击切换已播放/剩余；右侧 23×22 播放列表开关（`plugin.panels.sidebar`，与谁挂载侧栏无关），窗口窄于 320px 时隐藏。

菜单与按键（按 doc/process/rounds/2026-10-plugin-ui.md §3 分配表）：

- 顶层：`打开文件…`（Ctrl+O、F3）、`打开 → 打开链接…`（Ctrl+U，110）、`关闭`（F4）、`重开当前/最后文件`（Ctrl+Y，121）、`复制到剪贴板 → 文件名 / 文件夹路径 / 完整路径`（125–127）。
- `播放 →` 播放/暂停（Space）、上一个（PageUp、鼠标 X1）、下一个（PageDown、鼠标 X2）、`跳转 →` 后退/前进 5 秒（←/→，按住重复）、30 秒（Ctrl+←/→）、60 秒（Shift+←/→）、300 秒（Ctrl+Alt+←/→）、跳到开头（Backspace，219，`points` 组）、跳转到…（G）、`速度 →` 正常（Z）/ 减速 0.1（X）/ 加速 0.1（C）/ 0.5、1.5、2 倍（单选）/ 自定义…；播放时禁止屏保（240，勾选）、进度条悬停显示时间（242，勾选），二者在 `playback-options` 组。
- `声音 →` 音量 +（↑、滚轮上，重复）/ 音量 −（↓、滚轮下，重复）/ 静音（M，勾选）。

本轮功能（doc/product/feature-requirements.md「播放控制」A02、A10、B09、B29、I06、I08）：

- 打开链接（`open-url.ts` + `ui/open-url.slint`）：http/https/rtsp/rtmp/rtmps/mms/ftp、`file://` 与绝对路径；打开时可取剪贴板首行（`/usr/bin/pbpaste`，1 秒超时），否则用本次运行上次输入的文本。`player.open` 无条件替换当前来源。
- 重开当前/最后文件（`reopen.ts`）：有媒体来源时按原暂停状态重新载入并跳回原位置；没有来源时打开 `last.json` 记下的最后一个媒体来源（任何插件打开的、第一次就绪时记录；插件流不记）。
- 播放时禁止屏保（`sleep.ts`）：`phase: ready` 且未暂停时运行 `/usr/bin/caffeinate`（有视频或插件流 `-d -i`，纯音频 `-i`，都带 `-w <插件 pid>`），暂停、播完、关闭、出错、换来源、关开关、停用时结束。
- 进度条悬停显示时间：`ui/bar.slint` 用 `duration` 在 Slint 内算出指针处的时间，提示框画在底栏上方的视频区（不裁切），不经插件往返。
- 复制到剪贴板（`copy.ts`）：`/usr/bin/pbcopy`（`LANG=en_US.UTF-8`，2 秒超时）。
- 设置 `settings.json`：`openUrl.pasteFromClipboard`、`preventSleep`、`hoverTime`，默认都为 `true`。系统命令集中在 `system.ts`，测试替换为假实现（`tests/fakes.ts`）。

结构：`main.ts` 组装；`controller.ts` 播放动作、本地草稿与提示文字；`commands.ts` 菜单命令表；`bar.ts` 底栏属性与回调；`prompt.ts` + `ui/prompt.slint` 跳转到/自定义速度小窗；`format.ts` 时间格式。

限制：

- 拖动是一个手势（`player.scrub()`）：期间任何外部来源变化都会结束手势，之后的拖动点被忽略直到下一次按下；取消只丢弃未发送的点，已被宿主接受的跳转不撤销。
- 跳转到/自定义速度小窗属于打开时的来源：换了来源窗口自动隐藏，迟到的确认被拒绝；同一来源内被暂停、跳转不影响，确认作用于当前状态。
- 插件停用时当前播放、已提交的速度/音量/暂停保持不变。

开发（在 `src/` 下）：`npm run plugin -- check|build|test playback`。
