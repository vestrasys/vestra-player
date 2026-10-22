# 当前状态

更新：2026-10-20。

## 界面整改进度

[界面整改](product/ui-spec.md)分两步：

- **第一步（核心，已完成 2026-10-20，分支 `rebuild/core-ui`）**：输入绑定表、新菜单模型、主窗口外壳、主题与控件库、屏幕提示与信息层、播放序列、面板状态与分离、界面状态持久化、窗口标题、面板右键菜单；新增播放信息插件（Tab）。旧接口已删除，插件已全部改到新写法、交互未变。记录见 [本轮记录](process/rounds/2026-10-core-ui.md)。
- **第二步（各插件交互改版，已完成 2026-10-20，分支 `rebuild/plugin-ui`）**：10 个插件与播放信息插件按整改文档改版；核心按插件请求补了图标按钮、命令注销、插件数据目录。记录见[本轮记录](process/rounds/2026-10-plugin-ui.md)。

## 插件

10 个产品插件全部使用插件 API（SDK v2，见 [doc/plugins/](plugins/README.md)）。2026-10-20 在同一播放器进程中加载全部插件，用真实鼠标键盘做了全量 GUI 测试，12 项全部通过。这是开发者实测，尚无独立验收者结论。

| 插件 | 目录 | 已实测 | 未实测 / 已知缺口 |
| --- | --- | --- | --- |
| 基础播放 | `src/plugins/playback` | 打开、暂停/播放、拖动跳转、倍速、静音、音量 | Cmd/Ctrl+O 快捷键未实现（画面只路由 Space/←/→） |
| 播放列表 | `src/plugins/playlist` | 添加、选播、上/下一项、播完提示、切到列表外来源提示 | 网络条目未单独复测；自动推进被判过期后，该行状态停在「正在打开」 |
| 画面调节 | `src/plugins/video` | 亮度/对比度/饱和度/Gamma/高斯模糊作用于画面，复位，调节不打断播放 | — |
| 音频工作台 | `src/plugins/audio` | 均衡器、预设、复位，播放不中断 | 「响度规范化」勾选在深色背景下对比度低 |
| 字幕 | `src/plugins/subtitles` | 外部 SRT 载入、中英混排显示、轨道选择、样式与撤销 | 轨道名显示宿主临时名 `vestra-subtitle-*.vtt` 而非原文件名；插件允许 8 MiB SRT，宿主上传上限 2 MiB |
| 截图与导出 | `src/plugins/capture` | 保存当前画面，PNG 与源帧逐像素一致 | 连拍、MP4 未复测；无数值进度；连拍页「Sample start」保留打开窗口时的时间 |
| NES | `src/plugins/nes` | 打开 ROM 出画面和声音，停止后清屏 | 键盘输入、存读档按范围暂缓 |
| 插件工坊（实验性） | `src/plugins/workshop` | 生成 → 宿主校验 → 安装 → 生成的计数器可用（本地 mock 模型，无付费调用） | 升级、回退未在 GUI 实测 |
| YouTube | 插件仓库 `plugins/youtube` | 公开视频播放 | 下载未在 GUI 复测 |
| Bilibili | 插件仓库 `plugins/bilibili` | 公开视频播放 | — |

另有示例插件 `src/examples/bookmark`（[入门教程](plugins/tutorial.md)用），已在播放器中实测。

## 偶发问题（未稳定复现）

2026-10-20 宿主清理后的 GUI 冒烟测试中各出现一次，用同样步骤在清理前后的宿主上重试都没有复现：

- 播放 NES 并停止后再打开普通媒体，字幕工作台一直显示该来源「loading · 0 条字幕轨」，刷新无效，无法载入字幕；之后再打开另一个媒体恢复正常。
- 已应用画面亮度后载入外部 SRT、再跳转，亮度效果消失（画面调节窗口仍显示原值）。

## 待处理的 API 请求

按[社区工作流](process/community-workflow.md)的自下而上流程处理。

| 条目 | 来源 | 建议优先级 |
| --- | --- | --- |
| 窗口打开时转发根组件全部回调，取消「回调须在首次 `show()` 前绑定」的限制 | 维护者评估 | 中 |
| 宿主广播字幕状态事件，去掉字幕插件 1.5 秒轮询 | 维护者评估 | 中 |
| 字幕上传上限统一（插件 8 MiB / 宿主 2 MiB） | 字幕插件 | 中 |
| 构建工具对未打包的 npm 依赖导入报错（NES 曾因此安装后无法启动） | GUI 测试 | 中 |
| 以插件为夹具的真实宿主自动化集成测试（目前只有单元测试和手动 GUI 测试） | 维护者评估 | 中 |
| `commands.unregister`（运行期间移除单个命令） | 迁移 | 低，等有插件需要 |
| 导出任务数值进度、编码器预检 | 截图插件 | 待需求 |
| 打开媒体快捷键 Cmd/Ctrl+O | 基础播放 | 待需求 |
| 插件停用/管理 API（插件管理器）；目前没有对外停用入口，GUI 测试无法测「停用只撤销自己的效果」 | GUI 测试 | 待需求 |

## 检查命令

在 `src/` 下：`npm run check`、`npm run format:check`、`npm run boundaries`、`npm run test:sdk`、`npm run plugins:build`、`npm run plugin -- test <插件>`。
在 `src/host/` 下：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
