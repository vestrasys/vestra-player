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
| P1 SDK 设计与实现 | 完成 | 见下文 P1 记录 |
| P2 审查 | 通过（附裁决） | 见下文 P2 记录 |
| P3 插件迁移 | 进行中 | 第 1 批：audio、bilibili、youtube、subtitles |
| P4 收尾 | 未开始 | |
| P5 GUI 测试 | 未开始 | |
| P6 提交 | 未开始 | |

## P1 记录（维护者）

交付：`src/sdk/plugin/`（句柄式 SDK v2 与测试用 `TestHost`/`startPlugin`）、`src/tests/sdk/`（36 项）、`src/scripts/plugin.mjs`（公共构建工具：`npm run plugin -- check|build|test <name>`、`plugins:build`、`test:sdk`）、`runtime/runner.ts`（默认导出走 v2、命名导出 `activate` 走旧 API；v2 去掉客户端 10 秒超时）、文档 `sdk-v2.md`、`sdk-v2-usage.md`、`sdk-v2-migration-guide.md`、画面调节参考迁移（6.4 grep 为空，测试 6/6）。宿主未改。

协调者复核：tsc 0 错误、prettier、boundaries 通过；`test:sdk` 36/36；`plugin test video` 6/6；video `main.ts` 无协议字段。

## P2 审查（协调者）

结论：通过。usage 示例短、用领域名词、无协议细节，句柄随作用域释放，`bind`/命令选项函数提供“期望状态”。核实了更新时机：SDK 在每次调用插件处理函数后（异步处理函数在结束后）重新求值；同一处理函数中多个 `await` 之间设置的中间状态要到结束才显示，需要中途显示时调用 `plugin.update()`（已告知迁移者）。

裁决：

1. **工坊生成代码与 `vendor/sdk`**（维护者报告会卡住 P4 删除旧 API）：`vendor/` 中第三方 npm 依赖本轮不动；`vendor/sdk` 是本仓库 SDK 的副本，不属于第三方依赖，由工坊迁移者随迁移更新为 v2 类型，生成的计数器插件代码（`engine.mjs` 的 `counterSource`）同步改为 v2 写法。
2. **维护者建议的宿主改动**（`commands.unregister`、处理/字幕状态事件广播、`source.operation` 区分 unknown 与 unknownExpired、`ui.open` 转发全部回调）：本轮迁移不需要，暂缓，P4 结束后再评估。相关限制（释放命令只能禁用；窗口回调须在首次 `show()` 前绑定）在迁移中接受。
3. **`media.close()` 条件放宽为“仍是同一来源”**：接受，属于用户可见行为的轻微改善（暂停后关闭不再失败），不视为违反等价迁移。

其它已知问题（不在本轮修）：subtitles 允许 8 MiB SRT 而宿主上传上限 2 MiB；workshop `package-lock.json` 不同步（工坊迁移者修复）。

## 迁移阻塞与裁决

（无）
