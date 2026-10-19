# SDK v2 进度记录

流程定义见 [sdk-v2-migration.md](sdk-v2-migration.md)。分支：`rebuild/sdk-v2`（自 `rebuild/working` f65f060）。

## P0 基线（2026-10-20，协调者）

静态检查（2026-10-20 f65f060 上执行）：`cargo fmt --check`、`cargo clippy --all-targets -D warnings`（默认特性与 `--no-default-features`）、`cargo test`（110 + 143）、`tsc` 0 错误、prettier、boundaries 全部通过。

插件源码行数（`.ts`/`.slint`/手写 `.mjs`，不含测试、依赖、构建产物）与现有测试：

| 插件 | 源码行数 | 现有测试 | 基线结果 |
| --- | --- | --- | --- |
| audio | 296 | `node --test tests/` | 10 通过 |
| bilibili | 196 | `node --test tests/` | 10 通过 |
| capture | 526 | `node behavior.test.mjs` | 通过（脚本式断言） |
| nes | 274 | `node --test tests/` | 5 通过 |
| playback | 813 | `node --test tests/` | 6 通过 |
| playlist | 906 | `node --test tests/` | 14 通过 |
| subtitles | 324 | `node --test tests/` | 6 通过 |
| video | 127 | `node --test tests/` | 5 通过 |
| workshop | 289 | `test-engine.mjs`、`test-workflow.mjs`、`test-trial-recovery.mjs` | 3 个脚本退出 0（需先 `npm install --no-save`；`package-lock.json` 与 `package.json` 不同步，`npm ci` 失败） |
| youtube | 289 | `node --test test.mjs` | 10 通过 |

注意：workshop 测试运行时会在插件目录生成 `evidence/`，属测试输出，不应提交。

## 阶段状态

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| P0 准备 | 完成 | 本节 |
| P1 SDK 设计与实现 | 进行中 | 维护者 agent 已分派 |
| P2 审查 | 未开始 | |
| P3 插件迁移 | 未开始 | |
| P4 收尾 | 未开始 | |
| P5 GUI 测试 | 未开始 | |
| P6 提交 | 未开始 | |

## 迁移阻塞与裁决

（无）
