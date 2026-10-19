# 截图与片段导出探针

本版本是架构实验，使用公共契约 probe-0.2 的兼容子集（包含 API-004/005），在当前 probe-0.3 宿主回归；产品插件调用不变，仅测试中的字幕 provider fixture 按 API-007 迁移。版本见[基线登记](../../sdk/baselines/README.md)。最终 API、界面与产品可能相差很大。它是普通独立 TS 插件，不导入其他插件、宿主实现，也不自行调用 FFmpeg。

## 使用场景与策略

在宿主加载本目录 manifest 后，从“视频 → 截图”或“工具 → 导出”菜单，或原生 Slint 面板执行：

- **原始截图**：选择 PNG 输出，读取当前文件来源和时间，忽略画面处理及字幕。
- **处理后截图**：使用开始时冻结的处理链；可包含或排除字幕，仍保留其他画面处理。
- **连续截图**：从当前时间起按间隔采样，最多 100 张，输出 `名称-001.png` 等文件。它是离线有限时间轴采样任务，不要求用户继续播放。可中途取消，已经成功发布的 PNG 保留，当前未完成文件由宿主清理。超过来源时长时报告错误，并保留已完成文件。
- **MP4 片段**：从当前时间起导出指定时长，遇到文件末尾截短。开始后切换当前播放来源和修改处理参数，不改变已经启动的导出。输出包含来源音频及开始时音频处理参数。

保存对话框关闭后才冻结文件路径、播放位置、会话标识、处理链及组参数。准备期间发现会话改变则拒绝，避免混用两个来源。任务运行在宿主；TS 只轮询句柄更新进度。原始截图不申请处理快照，其余任务通过 owner 隔离的 `effects.snapshot`/`media.start({processingSnapshot})` 使用不可变句柄，finally 释放。

每次只运行一组操作，新增操作会明确失败。取消命令不会被长时间 await 的 action 队列阻塞。停用插件由宿主清理其任务、子进程与快照；插件 disposer 还会撤销订阅并尽力取消活动任务。

菜单、控件调用同一命令体系；多个菜单入口不要求多个 UI。原生 Slint 负责布局，没有按钮/布局 DSL。

## 自动验证

先构建真实 Rust 宿主，之后从仓库根目录运行：

```sh
export VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1"
node --test src/tests/probes/capture-export/*.test.ts
"$VESTRA_HOST" --check-ui src/plugins/capture-export/manifest.json
"$VESTRA_HOST" --ui-scenario src/tests/probes/capture-export/ui-scenario.json
src/node_modules/.bin/tsc --noEmit -p src/tsconfig.json
```

FFmpeg 必须具备 libx264、AAC、PNG、eq、gblur、scale 和 libass subtitles。本机默认 Homebrew 精简版 FFmpeg 没有 subtitles，真实测试使用 `PATH=/opt/homebrew/opt/ffmpeg-full/bin:$PATH`。这不是静默跳过字幕的替代测试。

设置 `VESTRA_CAPTURE_EVIDENCE` 可保留媒体和 `report.json`，每次在该目录下创建独立子目录，重跑不会覆盖先前证据；未设置时用独立临时目录并自动清理。

### 覆盖与已观察结果

2026-10-17 开发者验证：4 项自动测试通过，严格类型检查通过。真实 Rust headless 宿主、真实 Node worker 和 FFmpeg 执行，不用 mock 媒体算法：

- **E1**：有效 PNG 签名并独立解码；原始黑色像素均值 0，处理后均值 73；真实 SRT 字幕使逐通道平均差达到约 2.56；排除字幕与无字幕处理图逐字节像素一致。
- **E2**：连续 3 张有效 PNG；真实 8 秒 MP4，1280×720、带音轨；任务运行中切换为红色视频并删除旧处理参数后，导出仍为原灰色处理结果 `[73,73,73]`。观察到 0–1 中间进度，再取消长导出；真实状态为 cancelled，未发布输出且无 `.vestra-*` 半成品。
- 有限连续截图可取消；正在执行任务时停用插件，公开诊断中 jobs/snapshots 均回到 0，无半成品。
- 无媒体、无能力、无效参数、已有文件不可覆盖均有断言；失败后插件仍可继续工作。

真实处理证据由测试生成在 `.cache/probe-evidence/capture-export/vestra-capture-*/`。缓存不是提交材料，命令可以重建证据。

**G1 的独立 GUI 场景**实际创建 Slint 窗口、加载本插件、调用全局回调和已声明菜单回调、改变字幕及截图数量，并截取面板；输出在 `.cache/probe-evidence/capture-export-gui/`。场景测试不是物理鼠标/键盘操作，也未自动操作系统保存对话框。GUI 隐藏/重建的组合验证由全局验收场景覆盖；这里不把 UI 编译通过冒充该项完整验收。

## 明确限制与架构发现

- 仅支持有文件路径的可 seek 来源，不把 NES 等外部实时来源伪造成文件；未来外部帧截图需要新的公共采样能力。
- “原始/处理后”是当前 FFmpeg 实验后端的采样阶段，不包含 Slint 面板/窗口边框，不宣称覆盖未来 HDR/GPU/显示色彩管理。
- 用户在应用保存对话框中确认覆盖已有文件后，宿主仍拒绝覆盖，避免任务竞态破坏文件。
- 任务总进度按输出时长/组数量估算，取消期间可能短暂保留前次进度；最终状态以宿主回读为准。
- 字幕与画面参数可由任何 owner 注册，本插件只引用公共处理快照；没有对字幕工作台或画面调节插件的依赖。
- 原先考虑由插件读取 `effects.list()` 重建 FFmpeg 链，维护者拒绝，以避免复制宿主规则并遗漏 legacy/GPU 路径。快照句柄使一组任务绑定统一处理状态；API 请求与理由见 [API_REQUESTS.md](API_REQUESTS.md)。

开发者验证不代替独立验收 agent 的最终结论。
