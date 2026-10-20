# 播放列表（vestra.playlist）

功能：右侧栏播放列表（参考播放器样式，见 `doc/product/ui-alignment.md` §5）。31px 页签条（分离/附着、关闭按钮），单行 20px 列表项「序号. 标题」+ 右对齐时长，正在播放蓝色、播放过灰色、选中深色底，失败行尾警示图标；完整路径（及失败原因）为悬停提示，操作反馈用屏幕提示。底部 35px 按钮栏：置顶/上移/下移/置底、ADD（添加文件/添加链接）、DEL、SORT（按名称/按路径/倒序）与已知时长合计。多文件添加（`realpath` 去重，不可读的文件保留待重试）、添加 HTTP(S) 链接（只选中、不播放）；行使用稳定的 `rowId`。多选（Ctrl/Cmd、Shift 点击，Ctrl+A）、拖动排序，移除和移动作用于所选。自然播完后按当时顺序推进一次；循环（不循环/全部/单个）与随机默认关；若播完后来源被别处改动，自动推进放弃。另开的来源（即使路径相同）不接管、不提示。登记为播放序列提供者，`player.next/previous`（底栏、PgUp/PgDn）由本插件执行。

入口：右键菜单顶层「播放列表 ✓ F6」（windows 组 800）；「播放 ▶ 播放列表 ▶ 添加文件… / 添加链接… / 循环 ▶ / 随机」（230–235）。列表右键菜单与列表焦点下的按键：Enter/P 播放、Delete/Backspace 移除所选、Ctrl+↑/↓/Home/End 移动、Ctrl+A 全选、F6 关闭列表；↑/↓（加 Shift 扩选）、Home/End 移动选择；双击播放。链接对话框是独立窗口 `ui/link.slint`，侧栏是 `ui/sidebar.slint`。

时长：添加本地文件后由插件自行调用 ffprobe 读取（`VESTRA_FFPROBE` 指定的可执行文件，否则 PATH 上的 `ffprobe`；参数 `-v error -show_entries format=duration -of json <路径>`，不经 shell）。最多同时 2 个进程，每个 10 秒超时；移除该项或清空列表时中止其进程；结果按 realpath + mtime 缓存。链接行从不读取。某项开始播放后以播放器报告的时长为准。ffprobe 缺失或失败时该项显示 `--:--`，合计只加已知时长并在末尾加「+」（有未播放的链接行时同样加「+」）；不弹屏幕提示、不算错误。测试注入假 runner（`activate(plugin, runner)`），不调用真实 ffprobe。

限制：面板右键菜单只有一层，「排序 ▶」展开为三项。链接由站点插件注册的公共来源解析器打开，本插件不解析。队列只在本次运行内存在；移除/清空和插件停用都不关闭当前媒体。

开发（在 `src/` 下）：`npm run plugin -- check playlist`、`build playlist`、`test playlist`。只用 `import type` 引用 SDK。
