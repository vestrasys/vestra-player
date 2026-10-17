# 六探针独立验收产物

日期：2026-10-17。本目录保存冻结后独立验收的产物，不包含下载依赖、构建缓存、生成视频或游戏 ROM。完整结论见[独立审查](../../probe-review.md)与[架构观察](../../probe-design-findings.md)。

- [最终摘要](final-summary.json)：52 项 TS、1 项真实 Rust 后端、67 步 GUI、0 个遗留 owned 临时资源。
- [完整命令日志](final-check.log)：strict、静态边界、格式、Rust build/clippy、Slint 编译及行为测试。
- [源码及 binary 指纹](code-fingerprint.json)：95 个文件，验收前后不变；此处指纹覆盖范围大于 maintainer 的共享代码指纹，不应直接比较两者的 aggregate。
- [窗口观测](report.json)与[实际断言](assertions.json)：真实 Slint callbacks，面板切换不重编译，NES 按键使像素改变。不是物理键鼠自动化。
- [产物校验值](sha256.json)：本目录日志、JSON 与 PNG 的 SHA-256，不包含本说明和校验文件自身。

关键截图：

| 界面                                 | 实际窗口截图                                                             |
| ------------------------------------ | ------------------------------------------------------------------------ |
| 画面参数                             | [video-adjust.png](video-adjust.png)                                     |
| 音频工作台（100 Hz / 1 kHz / 6 kHz） | [audio-workbench.png](audio-workbench.png)                               |
| SRT 合成与原生面板                   | [subtitle-workbench.png](subtitle-workbench.png)                         |
| 截图任务                             | [capture-export.png](capture-export.png)                                 |
| Pi 真实诊断修复完成                  | [plugin-workshop.png](plugin-workshop.png)                               |
| 生成插件实际计数                     | [generated-counter.png](generated-counter.png)                           |
| NES 未按键 / 按键后                  | [nes-released.png](nes-released.png)、[nes-pressed.png](nes-pressed.png) |

从仓库根目录准备与复现：

```sh
./src/scripts/setup.sh
./src/scripts/make-sample.sh
./src/scripts/check.sh --gui
```

需要 Node、Rust、FFmpeg/ffplay 与桌面会话，字幕要求 FFmpeg 支持 libass。环境细节与未验证的平台、设备、生产 BYOK、统一外部来源处理等边界见架构观察，不能从本轮通过外推生产完备性。
