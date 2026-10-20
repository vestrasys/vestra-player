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
| P3 插件迁移 | 进行中 | 见下文 P3 记录 |
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

## P3 记录

| 插件 | 状态 | 源码行数 前→后 | 测试 | 协调者复核 |
| --- | --- | --- | --- | --- |
| video | 完成（P1 参考迁移） | 127→约 120（格式化后；字节 −48%） | 6/6 | 通过 |
| bilibili | 完成 | 196→177 | 13/13 | 通过：测试 13/13，6.4 grep 为空 |
| youtube | 完成 | 289→250 | 13/13 | 通过：测试 13/13，6.4 grep 为空 |
| audio | 完成 | 296→212 | 12/12 | 通过：阻塞经 SDK 修复后 12/12，6.4 grep 为空 |
| subtitles | 完成 | 324→343（旧代码多为单行长语句，协议代码已全删） | 13/13 | 通过：SDK 补充后 13/13，6.4 grep 为空 |
| playback | 完成 | 813→567 | 15/15 | 通过：SDK 修复后 15/15，6.4 grep 为空 |
| playlist | 完成 | 906→约 590 | 19/19 | 通过：SDK 补充后 19/19，6.4 grep 为空 |
| capture | 完成 | 526→737（旧 main.ts 为超长行，字节数实际减少；拆为 4 模块） | 11/11 | 通过：SDK 修复后 11/11，6.4 grep 为空 |
| nes | 完成 | 274→133 | 11/11 | 通过：SDK 修复后 11/11，6.4 grep 为空 |
| workshop | 进行中 | 289→ | | |

## 迁移阻塞与裁决

- **bilibili：无原因的载入失败兜底文案**。旧版显示“媒体加载失败。”，v2 中 `PlayerState.error` 在后端无原因时由 SDK 填入“无法打开此媒体”。裁决：接受，兜底文案由 SDK 统一，不改 SDK。
- **youtube：超时语义**（非阻塞，迁移者主动说明）。迁移指南建议把 `AbortSignal.timeout` 合入 signal，但那样超时会关闭媒体，而旧行为是超时只停止等待。迁移者改用计时器与 `open`/`ready` 竞速以保持旧语义。裁决：接受；P4 时由维护者在迁移指南中补充说明“超时是否关闭媒体”两种写法。

（无）
- **audio：被取代值的失败卡住新值**（真实 SDK 缺陷）。`filters.ts` 的 `#apply()` 在旧值（已被新 `set()` 取代）失败时仍进入 failed，新音轨的中性设置不提交。旧 `controller.ts` 用 epoch 忽略过期失败。裁决：SDK 缺陷，退回维护者修复（被取代或属于旧来源代次的值失败时不进入 failed，继续提交最新值），并授权维护者把音频测试中对应 todo 用例改为普通用例。
  - 已修复（维护者）：`filters.ts` 只在“当前期望值 + 当前来源”失败时进入 failed；同代次被取代与来源代次变化后迟到的失败两种情况均修复并补 SDK 测试（`test:sdk` 39/39）。其它最后写入生效的资源经检查无同类问题。迁移指南补充网络媒体超时两种写法。
- **subtitles：`subtitles.load` 无法中途取消**（旧版在分块之间检查取消、不提交）；**内嵌轨编号只能解析未文档化的 id 格式**。裁决：SDK 向后兼容补充 `load` 的 `signal` 选项与内嵌轨编号字段，授权维护者在字幕插件中做对应最小改动。迁移者担心的上传 id 冲突经协调者核实不存在（宿主每次加载分配新 owner，释放时清除 `high_water`）。
  - 已补充（维护者）：`subtitles.load` 支持 `signal`（分块之间与提交前检查，提交发出后不中止）；`SubtitleTrack.embeddedId`；字幕插件恢复“上传中取消不提交”并改用 `embeddedId`。`test:sdk` 40/40。
- **playlist：解析中被替换无法与打开失败区分**。旧版凭 ticket 识别被替换，显示“已切换到列表外来源”且不标失败；v2 `player.open()` 以普通错误失败。裁决：SDK 补充 `open` 被替换时以 `code: "replaced"` 拒绝（与 `media.ready()` 一致），授权维护者在播放列表中做对应最小改动。该插件报告的原有 bug（自动推进 stale 后行状态停在“正在打开”）本轮不修，记入遗留问题。
  - 已补充（维护者）：`player.open()`/`openStream` 在已被接受后被替换时以 `code: "replaced"` 拒绝；播放列表恢复旧行为；`sdk-v2.md` 补 `open` 错误 code 列表。`test:sdk` 42/42。
- **nes：`openStream` 在选中来源请求未答复时中止，撤销不干净**（真实 SDK 缺陷：来源最终被选中且指向已释放的 producer）；**首帧预填充超时与激活超时共用 code**，插件只能匹配英文消息。裁决：退回维护者修复并提供可区分的超时信息，授权其在 NES 插件中做对应最小改动。宿主拒绝选中来源时原文案中的 JSON 无法还原：接受为已知文案差异。
- **playback：编辑框中用户输入被覆盖**（SDK 设计缺陷）。`ViewBase.dispatch` 每收到回调就清空已发送缓存并重发全部期望属性，包括打开编辑框时一次性 `set` 的初始值，于是编辑期间任何其它底栏回调都会把用户输入的文字改回初始值。裁决：退回维护者修复（排在 NES 修复之后）。其它细微差异（失败文案不再是 JSON、音量请求不再有 34 ms 间隔、某音量请求失败后后续值仍发送）接受。遗留：PB-K2（打开媒体的 Cmd/Ctrl+O 快捷键，宿主只路由 Space/←/→）。
- **capture：同 playback 的属性覆盖缺陷**（`section`、`start-time`、`end-time` 等 in-out 属性在用户修改后被任意回调重发覆盖；插件做了部分缓解，标签页切换和“Use current”组合仍会被重置）。另报告 SDK 风险：窗口 `ui.set` 持续失败时，在 `onError` 中写状态会触发不断重试。裁决：与 playback 一并退回维护者修复。文案差异（不再显示 intent 版本、快照 id、任务 id）接受。遗留：`MediaJob` 无数值进度、无编码器预检查询。
  - nes 已修复（维护者）：`openStream` 中止时先等选中请求答复、只关闭这一代来源、再释放 producer；超时错误带 `stage: "prefill" | "activation"`；NES 改按 `stage` 映射文案。`test:sdk` 44/44。
  - 属性覆盖已修复（维护者）：回调不再触发重发；`bind` 值只在变化时发送；`set()` 同值也发送一次（显式“拉回模型值”）；写入失败不循环（绑定值待变化、`set` 值退避重试、连续失败只报一次）。playback/capture 删除变通代码、恢复旧语义并补测试。`test:sdk` 46/46。已知边角：取整后恰等于上次发送值时滑块停在用户拖到的位置（旧扁平 API 下同样存在），subtitles 的字号/延迟滑块属此情况。
