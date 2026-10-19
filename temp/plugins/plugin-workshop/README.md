# AI 插件工坊探针（实验）

这是普通 TS 插件，原生界面使用 Slint。目标消费者基线为 **probe-0.2**（见[公共基线登记](../../sdk/baselines/README.md)），依赖 `plugins.validate/install/remove`、`ui.patch`、`onAction` 和 `dialogs.openFile`。未要求新增或破坏性 API；不因其他插件引入可选能力而迁移。全部代码和生成包均为实验，最终 API 可能差异很大。

## 有限需求

本轮只模拟“生成一个原生计数器插件，按钮及菜单按指定增量计数”。默认 +1，可生成 +2 替换版，或打开 JSON：

```json
{ "prompt": "生成一个加三的原生计数器", "increment": 3 }
```

`prompt` 是模拟用户指令，`increment` 为 1–10 整数。确定性 mock 不具备理解任意自然语言的能力；这不是可泛化的真实模型产品。工坊不导入生成包实现，不读取其他探针，也不调用其动作。测试由独立宿主测试客户端操作生成包来证明真实行为。

## 真实 Pi 与 mock 的边界

- 精确锁定 `@earendil-works/pi-agent-core@1.3.0`、`@earendil-works/pi-ai@1.3.0`、`typebox@1.3.27`，三个直接依赖均为 MIT。包与 lock 在本插件，Pi 不是 Rust core/SDK 的依赖。
- 使用实际 Pi `Agent` 的工具执行、transcript、事件、循环和 `abort()`；不使用 coding-agent CLI/TUI，也不重新手写 agent 循环。
- `ModelFactory` 返回 `{ model, streamFn }`，唯一被 mock 的部分是模型流。以后可替换为选定模型和 Pi `Models.streamSimple` 适配器，生成工具和公共安装流程继续使用。
- mock 模型输出实际 tool calls。Pi 执行 `write_package` 写四个文件 → `validate_package` 调公共宿主编译 → 编译器返回真实 TS2322 → 下一轮 mock 读取这个诊断才输出修复 → 再校验 → `install_package`。
- 没有诊断就不走“修复成功”分支。不是复制磁盘模板、伪造编译结果，或把一个成功标记当成已安装。生成包只有 SDK type import，独立 Node worker 执行 `increment` 并更新自己的 Slint global。
- 每轮最多 12 次 mock 模型调用。正常修复链有 6 轮模型调用、5 次工具调用。无 API key、零网络模型请求、无付费调用。供应商协议、BYOK、模型质量没有验证。

实际 API 以已安装包的 `dist/agent.d.ts`、`dist/types.d.ts` 和源码为准。参考 [Pi Agent 官方说明](https://github.com/earendil-works/pi/blob/main/packages/agent/README.md) 与 [Agent 循环源码](https://github.com/earendil-works/pi/blob/main/packages/agent/src/agent-loop.ts)。本插件只使用公开导出。

## 安装事务、取消与文件寿命

候选写入工坊自己的 `.cache/workshop-*/candidate-*`，与当前可用包隔离。写工具只允许 index.ts、ui.slint、manifest.json、package.json 四个文件，限制大小；这是防止误操作的约束，不是权限沙箱。

只有当前 revision 真实校验通过后才能试载。公共 `install` 启动独立候选 worker，ready 后替换旧 worker；激活失败保留旧 route、状态和计数。工坊保留前一个成功包，可以显式回退；回退重新激活旧包，计数从 0 开始，这与安装失败时旧进程继续运行不同。

生成是后台任务，不阻塞 worker 的 action 队列。`cancel` 调用 Pi 的 abort 并等待结束；慢模型响应可立刻取消。真实编译 API 暂无取消参数，取消期间等待在途校验返回，但不会继续安装。已进入 install 事务时不宣称可以取消，界面说明等待提交结果后可回退。

失败/取消候选清理源文件，只留 `run.json` 证据。成功包与上一个可回退包保留源码；更旧包释放源码。工坊 `remove-generated` 动作使用公共 remove，释放当前生成 worker 与保留包源码，证据继续保留。

**关闭/卸载工坊不会自动卸载已安装生成插件**。安装产物是独立插件，仍可继续计数，因此其源文件继续保留。当前会话工坊记录自己的 current/previous；跨宿主重启的包目录持久化、管理所有权恢复和垃圾回收不是本轮验证范围。不要把 `.cache` 当成生产插件数据库。证据保留是本实验的明确策略，磁盘文件不等于仍有活动 worker/订阅/任务。

## 运行和复现

仓库 `src` 的 Node 开发依赖以及 Rust binary 须先按统一检查脚本准备；Node 至少 22.19（作者实际 Node 25.9）。先安装本插件的局部依赖：

```sh
npm ci --prefix src/plugins/plugin-workshop
```

从仓库根目录运行真实宿主测试：

```sh
PATH=/opt/homebrew/opt/ffmpeg-full/bin:$PATH \
VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1" \
node --test src/tests/probes/plugin-workshop/workshop.test.ts
```

启动真实窗口（视频不是工坊前置条件）：

```sh
SLINT_BACKEND=winit-software \
.cache/probe-target/debug/vestra-slint-experimental1 \
  --plugin src/plugins/plugin-workshop/manifest.json
```

右键“工具 → 插件工坊 → 生成计数器插件”，或选择工坊面板按“生成 +1/+2”。生成成功后，右键“插件界面”选择生成计数器；“工具 → 生成插件”也有计数菜单。Slint 界面首次引入新组件可能重建窗口，工坊不另造 UI 编排 API。

## 作者证据与覆盖

对应 W1/W2/W3/I1 的 `workshop.test.ts` 已在真实 Node worker + Rust binary 运行 **3/3 通过**（2026-10-17；最终作者复跑 27.63 秒，并发真实编译负载；后续共享修改仍需集成回归）：

1. 严格需求对象/未知字段/数值校验。
2. 真实 Pi 诊断修复及生成包计数；坏 TS、坏 Slint、合法但 activate 抛错；取消慢模型；+2 替换；回退 +1；卸载工坊后计数器继续工作。
3. 原生文件选择 API 的测试 FIFO 读取需求 JSON，生成 +3 并真实执行；公共 remove 后无生成 worker，回退错误可恢复。

每次运行写 `.cache/probe-evidence/plugin-workshop/` 的运行报告及 summary，各报告含真实 transcript、tool events、编译诊断、结果；生成候选中的 run.json 记录原始证据。测试断言真实生成包 UI 属性与 route 保留，未以作者状态字符串替代实际行为。

真实窗口组合、最终新 binary 回归、六插件顺序/生命周期和独立总验收由协调/验收角色另行记录；本 README 不代表六探针已全部通过。未验证外部模型、任意用户需求、安全沙箱、Windows/Linux 打包或重启后的插件持久化。
