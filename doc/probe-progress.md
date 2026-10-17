# 六插件探针进度与责任记录

本文件随实验更新，不是验收报告。工程标准见 [工程规范](../src/ENGINEERING.md)，最终通过条件见 [验收矩阵](probe-acceptance.md)。本轮六探针实现与独立验收已完成，代码在 src/。执行[社区模拟与 API 变更规则](plugin-probe-workflow.md#3-api-契约与变更请求)，目标补充见该工作流第 8 节。

恢复检查已完成：登记契约与消费者基线、补齐 API-003 至 API-007 决策、重建宿主并完成真实消费者回归。最终独立检查退出 0：52 项 TS、1 项 Rust 后端、67 步原生 GUI，退出后 0 个遗留 owned 临时资源。完整报告见[独立审查](probe-review.md)，结论见[架构观察](probe-design-findings.md)，产物见[证据目录](probe-evidence/2026-10-17/README.md)。

## 责任与工作区

| 角色           | Agent           | 写入范围                                                    | 状态                                                               |
| -------------- | --------------- | ----------------------------------------------------------- | ------------------------------------------------------------------ |
| 主协调         | root            | 标准、验收场景、进度、集成辅助工具                          | 已启动                                                             |
| API maintainer | api_maintainer  | sdk/runtime/host、共享配置和 harness、API 与变更记录        | probe-0.3 实现冻结，基线与指纹已核对，独立验收通过                 |
| 画面调节       | probe_video     | plugins/video-adjust、tests/probes/video-adjust             | 8 项测试通过（含 Rust 真实像素），最终独立验收通过                 |
| 音频工作台     | probe_audio     | plugins/audio-workbench、tests/probes/audio-workbench       | 6 项测试通过（含真实音频指标），最终独立验收通过                   |
| 字幕工作台     | probe_subtitles | plugins/subtitle-workbench、tests/probes/subtitle-workbench | API-007 迁移经原作者复核，3 项真实测试通过，最终独立验收通过       |
| 截图与片段导出 | probe_export    | plugins/capture-export、tests/probes/capture-export         | 4 项测试、真实产物及 GUI 场景均经独立验收通过                      |
| NES            | probe_nes       | plugins/nes、tests/probes/nes                               | 4 项测试含真实 Rust/Node、原生 GUI 输入均经独立验收通过            |
| AI 插件工坊    | probe_workshop  | plugins/plugin-workshop、tests/probes/plugin-workshop       | 3 项真实测试通过，Pi 循环/修复/安装/回退及生成包执行均独立验收通过 |
| 独立验收       | acceptance      | probe-review.md、tests/acceptance                           | 已推动缺陷修复，冻结后完整独立检查及全矩阵验收通过                 |

## 环境

- Node 25.9.0。
- FFmpeg 原 8.1 因 Homebrew x265 ABI 缺失无法运行；已重装为 9.0.2，本轮真实媒体检查已重新执行。
- 普通 FFmpeg 9.0.2 缺少 libass，另已安装 ffmpeg-full 9.0.2；脚本通过 PATH 优先使用 full 版本以验证真实字幕合成。
- Rust 1.92.0 隔离安装到仓库已忽略的 `.cache/toolchains`，不修改用户默认工具链；历史 /tmp 工具链不可再依赖。
- 所有构建缓存和生成素材保持忽略，历史 experimental1 报告不修改成新测试结果。

## 变更与风险

- 旧实验 SDK 带专名 Gaussian 字段，不作为六探针的最终公共契约模板；由 maintainer 演进 owner-scoped 资源能力。
- 模型调用使用 mock，不能把供应商互通或真实 BYOK 标为已验证。
- API 请求细节分别记录在各插件 API_REQUESTS.md，maintainer 的公共契约和决策记录为实施依据。
- 最终验收者已逐项核对全部矩阵，协调者复核报告、95 项实现/测试文件及 binary 哈希一致；未凭开发者自报完成。
- 跨插件检查由 tools/check-plugin-boundaries.ts 使用 TypeScript AST 执行；静态检查不宣称能阻止可信 Node 代码的所有运行时通信。
- GUI 通用场景驱动已加入 host/scenario.rs，可加载 manifest、触发真实 Slint action/菜单回调、等待状态与截图；它不是产品插件的隐藏接口。
