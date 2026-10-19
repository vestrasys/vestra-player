# Vestra Player 实验代码

最新状态：后续真实交互发现“打开文件”触发 Slint 定时器重入崩溃，以及视频区域右键菜单被输入控件吞掉。正在进行[完整交互验收](../doc/interactive-acceptance.md)与修复，尚未通过。下方 52 项测试/67 步窗口结果是上一轮有限场景的历史证据，不包含真实系统对话框，不能理解为全部功能可用。

当前实现仍为实验版本，core API、插件格式和目录结构尚未冻结，与最终成品可能相差很大。代码从 `doc/experimental1` 移入本目录，后续六插件探针也在这里迭代。

```text
host/      Rust Slint 宿主与播放后端
runtime/   TS 插件运行时
sdk/       临时公共 API 和类型
plugins/   各自独立的功能插件
tests/     插件协议测试
scripts/   启动、检查与实验脚本
```

在本目录运行：

```sh
./scripts/run.sh
./scripts/check.sh
# 工具链与依赖齐备时：
./scripts/check.sh --gui
./scripts/playlist-experiment.sh
```

首次获取代码需安装根目录及两个插件各自锁定的依赖（下列命令从仓库根目录执行）：

```sh
./src/scripts/setup.sh
./src/scripts/make-sample.sh
./src/scripts/check.sh
# 已构建宿主后，真实原生窗口组合场景：
PATH="/opt/homebrew/opt/ffmpeg-full/bin:$PATH" node src/tools/probe-gui.ts
```

需要 Node、Rust 工具链和 FFmpeg/ffplay；字幕检查要求 FFmpeg 编译了 libass。GUI 命令需要可显示窗口的桌面会话，会生成测试媒体、自制 CC0 NES ROM、六探针及 Pi 生成插件的窗口截图，产物默认在已忽略的 `.cache/probe-evidence/combined-gui/`。可用 `VESTRA_HOST` 指定宿主，用 `VESTRA_GUI_EVIDENCE` 指定产物目录。操作通过真实 Slint 回调驱动，不等于物理鼠标键盘自动化。

运行条件和用法见 [Slint 实验说明](../doc/experimental1/README.md)，历史结果见 [验证记录](../doc/experimental1/findings.md)。播放列表窗口实验的报告及截图仍输出到 `doc/experimental1/evidence`，生成样本位于本目录已忽略的 `samples/`。

本轮执行规范见 [六插件探针工作流](../doc/plugin-probe-workflow.md)，状态见进度记录；代码所在目录不改变实验性质。

迁移时曾因 Homebrew FFmpeg 缺少 `libx265.215.dylib` 无法运行媒体检查，现已恢复并安装含 libass 的 `ffmpeg-full`。`scripts/env.sh` 会在可用时优先使用该版本，并选择仓库隔离的 Rust 工具链和构建缓存。

本轮六探针已独立验收通过：52 项 TS、1 项真实 Rust 后端、67 步原生 GUI；API 仍是实验基线，不代表生产定稿。进度、复现命令及实验边界见[责任与进度](../doc/probe-progress.md)、[验收矩阵](../doc/probe-acceptance.md)及[独立审查](../doc/probe-review.md)。API 基线和社区迁移流程见[工作流](../doc/plugin-probe-workflow.md)。
