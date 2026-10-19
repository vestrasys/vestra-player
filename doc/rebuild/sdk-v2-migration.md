# SDK v2：自上而下的插件 API 重构流程（2026-10-20）

本文是本轮所有 agent 的共同任务说明。每个 agent 开工前必须完整读完本文，再读自己角色对应的小节。本文优先于 `plugin-probe-workflow.md` 中与之冲突的流程规定（例如插件作者提交 API 请求、维护者按请求自下而上扩展）；那些规定在本轮结束后恢复适用。

## 1. 为什么做这件事

当前 `src/sdk` 只是一层很薄的 RPC 绑定：`Host` 由 59 个“发请求、收结果”的函数和几个全局事件流组成。宿主为保证正确性引入的协议字段（`revision`、`expectedGeneration`、`stamp`、`intentVersion`、`eventSequence`、`operationId`、`unknown`/`unknownExpired` 等）被原样交给每个插件处理。后果：

- 每个插件都在重写同样的轮子：合并连续更新（`Latest`、`LatestValue`、`LatestProjection` 三个版本）、提交失败后查询结果并重试（10 个插件各一份）、过滤过期事件、逐个属性同步 UI、命令状态对象、手写清理清单。
- 资源用字符串名字指代，事件从全局流按字符串分发（`ui.onCallback`、`commands.onExecute`），导致所有逻辑只能堆在一个大 `activate` 闭包里，无法拆分、复用或单独测试。基础播放插件从旧实验的 56 行涨到 813 行，其中大半是协议细节。
- 音频工作台“上一操作结果待确认”卡死的 bug，就出在插件自行处理 `unknown` 结果的代码里。

本轮目标：在**不改变产品行为**的前提下，给插件一套好用、好读的高层 SDK，让插件代码只表达业务。

## 2. 本轮与常规流程的区别

| | 常规（插件扩展 API） | 本轮（重构 API 本身） |
| --- | --- | --- |
| 方向 | 自下而上：插件遇到缺口 → 提请求 → 维护者扩展 | 自上而下：维护者设计并发布新 SDK → 插件按迁移指南同步修改 |
| 插件作者能否提新功能/新能力请求 | 能 | **不能**。只能上报“迁移阻塞”（见 6.3） |
| 产品行为 | 可以随插件演进 | **必须保持不变**（等价迁移），不新增、不删减用户可见功能 |
| 兼容层 | 按迁移计划保留 | 旧扁平 `Host` API 只在迁移期间存在，全部插件迁移完成后删除（退出条件明确） |

## 3. 角色与文件所有权

| 角色 | 由谁担任 | 可写范围 | 职责 |
| --- | --- | --- | --- |
| 协调者 | 主会话（Claude Code 主对话） | `doc/rebuild/` 下本轮记录文件；git 提交 | 分派任务、审查各阶段交付、裁决迁移阻塞、执行全量 GUI 测试、汇总报告。不代替维护者写 SDK，不代替作者改插件 |
| API 维护者 | 1 个子 agent | `src/sdk/`、`src/runtime/`、`src/scripts/`（新增公共插件构建工具）、`src/tests/`、`src/package.json`、`src/tsconfig.json`、`doc/rebuild/api.md` 及本轮 SDK 文档；宿主 `src/host/` **仅在协调者批准后**才能改 | 设计并实现 SDK v2、迁移指南、参考迁移（画面调节插件）、公共构建工具；处理迁移阻塞；最后删除旧 API |
| 插件迁移者 | 每个插件 1 个子 agent | 只能写 `src/plugins/<自己的目录>/` | 按迁移指南把自己的插件改到 SDK v2，保持行为不变，清理目录中的流程产物 |

共同约束（所有 agent）：

- 不读取 `temp/`、git 历史中的旧实现。不读取、不修改兄弟插件目录（维护者为编写迁移指南可以只读所有插件）。
- 不运行 `git commit`、`git push`、`git reset`、`git checkout -- <path>` 等改变仓库历史或丢弃他人改动的命令；提交由协调者统一进行。
- 不启动 GUI、不操作屏幕（不运行宿主窗口、不用鼠标键盘模拟工具）。GUI 测试只由协调者执行，避免多个 agent 同时操作同一窗口。
- 构建 Rust 时使用 `CARGO_TARGET_DIR=/tmp/vestra-review-target`，不要在仓库里产生 `target/`。
- 第一方 Rust 禁止 `unsafe`；TS 使用可擦除语法（`erasableSyntaxOnly`）。
- 交付结束时，按第 9 节的格式给出报告。报告只陈述实际执行过的检查及其结果，未执行的写“未执行”。

## 4. SDK v2 设计要求（维护者必须满足，细节由维护者设计）

### 4.1 形态：句柄式（handle-based），借鉴 VS Code

- `activate` 收到一个插件对象（命名由维护者定，例如 `plugin`）。插件拥有的每样东西——命令、窗口、视频/音频处理资源、字幕资源、订阅、任务——都通过工厂方法创建并返回**句柄对象**，后续操作都在句柄上进行。
- 每个句柄都是 Disposable。插件停用时，SDK 自动释放该插件创建的全部句柄（撤销处理资源、关闭窗口、退订、注销命令）。插件不再需要手写清理清单。提供子作用域（例如 `plugin.scope()`），让一个功能模块的句柄可以作为一组单独释放。
- 回调绑定在句柄上（`window.on("adjust", fn)`、命令的 `run`），不再有需要插件按字符串分发的全局 `onCallback`/`onExecute`。
- 句柄可以作为普通对象传给插件内的其它模块，使插件可以拆成多个文件和函数，`activate` 只负责组装。
- 连续变化的值（窗口属性、命令启用状态、处理参数）支持“最后一次写入生效”：SDK 负责合并连续更新、去重、批量下发，插件只描述期望状态。是否引入轻量响应式绑定（例如 `window.bind(state)`、`enabled: () => ...`）由维护者决定；若引入，必须足够小、可读，不得变成一个通用 UI 框架。

### 4.2 协议细节必须收进 SDK（或宿主）

插件代码的默认路径中**不得出现**以下概念；它们由 SDK 自动维护：

- `revision` 计数（处理资源、命令状态、字幕样式等）。
- `expectedGeneration` / `expected` 来源戳：默认表示“当前来源”。只有确实需要条件提交的场景（例如截图/导出绑定原来源快照、播放栏拖动结束时确认仍是同一来源）才通过显式选项传入，并在文档里给出 usage example。
- `operationId` 与 `*.operation(...)` 结果查询：RPC 超时或传输错误后，由 SDK 透明地查询结果并给出确定结论。插件只看到三种结果：已完成、失败（附可显示的原因）、进行中。`unknown`（从未被宿主接收，即未执行）与 `unknownExpired` 由 SDK 处理，不交给插件。注意当前运行器（`runtime/runner.ts`）对除 `dialogs.open` 外的请求设置了 10 秒超时，这是“结果未知”的主要来源，维护者应一并处理。
- `eventSequence` / `generation` 比较等过期事件过滤：SDK 只向插件投递最新且有效的状态。
- UI 属性名转换（camelCase ↔ kebab-case）、逐属性 `ui.set`、打开窗口时的回调白名单：由 SDK 处理（白名单可从 `.slint` 文件声明的回调推断，或由句柄绑定自动收集）。
- `backendUncertain` 一类宿主级故障：给出统一的状态/错误通道，不要求每个插件自己做“检查 / 重试”界面。

### 4.3 保持不变的东西

- `manifest.json` 的格式与职责不变（id、apiVersion、entry、capabilities、生产包 version）。
- 宿主 wire 协议 `vestra.experimental/0.1` 原则上不变，SDK v2 建在现有 RPC 之上。确实需要宿主配合（例如为消除某类“结果未知”需要宿主新增一个查询）时，维护者先向协调者说明理由、影响范围与替代方案，经批准再改，并补宿主测试。
- 插件独立性原则不变：插件之间不 import、不直接通信；共享的只有 SDK 与公开宿主状态。
- 现有全部产品行为不变。

### 4.4 命名与可读性

- 使用领域名词：例如 `player`、`video`、`audio`、`subtitles`、`commands`、`windows`、`jobs`。避免 `ctx` 一类什么都往里放的对象，避免 `processing`、`producer` 这类不说明对象是什么的名字直接暴露给插件作者（宿主内部可以保留原名）。
- 公共类型和方法都要有简短注释与 usage example。
- 维护者交付的 usage example 文档（第 5 节 P1）中，每个示例都应短到一眼能看懂；若某个场景的示例超过约 30 行，说明封装还不够，需要重新考虑。

### 4.5 公共插件构建工具

在 `src/scripts/` 提供一个公共工具（例如 `npm run plugin -- build <name>`、`check <name>`、`test <name>`），替代各插件目录里各写一份、且多处硬编码 `/tmp/vestra-rebuild-toolchain-…` 路径的 `build.mjs`/`build.cjs`/`check.mjs`/`tsconfig.build.json`。它负责：严格类型检查、剥离 TS 类型生成 `dist/`、复制 `.slint` 和其它资源、生成带 `version` 的 `dist/manifest.json`。有 npm 依赖或需要打包的插件（`nes` 的 jsnes、`workshop` 的 Pi 与 esbuild 打包）必须能通过该工具或其明确的扩展点构建。

`dist/` 改为构建产物，从 git 中移除并加入 `.gitignore`；`src/package.json` 增加一条构建全部插件的命令，供协调者的 GUI 测试使用。

## 5. 阶段

### P0 准备（协调者）

1. 确认工作区干净，基于 `rebuild/working` 新建分支 `rebuild/sdk-v2`。
2. 记录基线：全部静态检查结果、各插件现有测试结果、10 个插件代码行数。写入 `doc/rebuild/sdk-v2-progress.md`。

### P1 SDK 设计与实现（API 维护者）

交付物：

1. `doc/rebuild/sdk-v2.md`：设计说明——模块与句柄一览、生命周期与释放规则、协议细节如何被吸收（逐条对应 4.2）、显式条件提交的使用方式、错误模型、与旧 API 的对照表。
2. `doc/rebuild/sdk-v2-usage.md`：usage examples，至少覆盖：
   - 画面调节（连续控制 + 处理资源 + 工具窗口）；
   - 基础播放栏（来源控制、拖动跳转、命令启用状态随播放状态变化、挂载到底栏）；
   - 播放列表（侧栏列表模型、选中、自动推进）；
   - 截图导出（绑定来源快照的任务、进度、取消）；
   - NES（producer 音视频输出与停止）；
   - 字幕（外部文件载入）。
3. SDK v2 实现（`src/sdk/`，必要时 `src/runtime/`），带单元测试（使用假宿主/假 transport，不需要真实宿主）。同时提供一个供插件测试使用的假宿主工具，让插件作者能脱离真实宿主测试业务逻辑。
4. 公共插件构建工具（4.5）。
5. `doc/rebuild/sdk-v2-migration-guide.md`：给插件迁移者的逐步指南——旧调用到新调用的对照、常见模式（合并更新、结果不确定、命令状态、窗口回调、清理）的新写法、目录清理清单（第 7 节）。
6. **参考迁移**：维护者亲自把 `src/plugins/video`（画面调节）迁移到 SDK v2，作为其它迁移者的范例。这是本轮唯一允许维护者修改插件目录的情况。

迁移期间旧扁平 `Host` API 保留可用（其余 9 个插件还没迁移），但旧 API 不再新增任何东西。

P1 完成条件：`npm run check`（tsc）、prettier、boundaries、SDK 单元测试、`cargo test`（若改了宿主）全部通过；画面调节插件用新构建工具构建通过、其测试通过；它的 `main.ts` 中不再出现 4.2 列出的协议概念。

### P2 设计审查关口（协调者）

协调者对照第 4 节逐条审查 P1 交付，重点看 `sdk-v2-usage.md` 是否美观易读（用户最关心这一点；用户本轮不参与中途审查，会在结束后查看最终结果）。审查结论写入 `sdk-v2-progress.md`：通过 / 需修改（列出具体条目，退回维护者）。**未通过前不分派插件迁移。**

设计参考：借鉴 VS Code 的 Disposable、句柄对象、事件绑定与“声明和处理放在一起”这几个模式即可，不要照搬 activation events、contribution points、Extension Host 等整套机制。决定 API 形态的依据是本仓库的宿主协议与 10 个插件的实际需求。

### P3 插件迁移（9 个插件迁移者，并行）

剩余 9 个插件：`audio`、`bilibili`、`capture`、`nes`、`playback`、`playlist`、`subtitles`、`workshop`、`youtube`。同时运行的子 agent 不超过 4 个，按批次轮换；建议先做较小、较独立的（`audio`、`bilibili`、`youtube`、`subtitles`），再做较大的（`playback`、`playlist`、`capture`、`nes`、`workshop`）。

每个迁移者的任务见第 6 节。维护者在此阶段保持可用，处理协调者转来的迁移阻塞。

### P4 收尾与旧 API 删除（API 维护者）

全部插件迁移通过后：

1. 删除旧扁平 `Host` API 和不再使用的类型；确认没有插件、示例或测试仍引用它们。`src/examples/` 下的示例一并迁移或标注为仅供宿主测试的底层示例。
2. 更新 `doc/rebuild/api.md`。
3. 全量静态检查（第 8.1 节）全部通过。

### P5 全量 GUI 测试（协调者）

见第 8.2 节。发现缺陷时：属于插件的退回对应迁移者（重新启动一个只修该缺陷的 agent，附复现步骤），属于 SDK 的退回维护者；修复后只复测受影响项。

### P6 提交（协调者）

更新 `product-status.md`、`sdk-v2-progress.md`，提交并推送 `rebuild/sdk-v2`，向用户报告；是否合并回 `rebuild/working` 由用户决定。

## 6. 插件迁移者任务说明

### 6.1 开工前必须阅读

1. 本文全文。
2. `doc/rebuild/sdk-v2.md`、`doc/rebuild/sdk-v2-usage.md`、`doc/rebuild/sdk-v2-migration-guide.md`。
3. 参考迁移：`src/plugins/video/` 的新版本。
4. 自己插件的现有代码、测试和 `README.md`，以及 `doc/design/plugins/<对应设计>.md`（若存在），用于确认现有行为。

### 6.2 要做的事

1. 把插件改为使用 SDK v2，**行为保持不变**：同样的菜单项（路径、标题、启用条件）、同样的窗口与控件、同样的回调效果、同样的错误提示语义。可以改善代码结构（拆分文件/模块），不能增减用户可见功能。
2. 删除插件内已由 SDK 提供的自制轮子（合并更新类、结果查询/重试逻辑、过期事件过滤、属性名转换、手写清理清单等）。
3. 迁移现有测试，改用 SDK 提供的假宿主；原有测试覆盖的行为必须继续被覆盖。
4. 按第 7 节清理目录。
5. 用公共构建工具构建并测试自己的插件。

### 6.3 不允许做的事，以及遇到问题怎么办

- 不新增功能、不提新能力请求、不“顺手”修改与迁移无关的行为。
- 不修改 `src/sdk`、`src/runtime`、`src/host`、`src/scripts` 或兄弟插件；不在插件里绕开 SDK v2 直接调用底层 transport。
- 如果现有某项行为用 SDK v2 **无法表达**，这是“迁移阻塞”：停止该部分，按第 9 节报告格式写明——哪个现有行为、在哪段旧代码、SDK v2 为什么表达不了、最小复现——然后继续迁移其它部分。协调者会转给维护者裁决（补充 SDK 或给出写法），不要自己发明替代方案。
- 如果发现现有代码本身有 bug，记录在报告里，不在本轮修（除非迁移后该 bug 自然消失，此时注明）。

### 6.4 完成条件（迁移者自检，协调者复核）

- 公共构建工具对本插件 `check`、`build`、`test` 全部通过。
- 在插件源码（不含测试）中执行下面的检查，结果为空；若有必要的例外（例如截图显式绑定来源快照），在报告中逐条说明理由：
  ```
  grep -nE "expectedGeneration|eventSequence|intentVersion|operationId|unknownExpired|backendUncertain|\.operation\(|onCallback|onExecute|ui\.set\(" <插件源码文件>
  ```
- 目录符合第 7 节。
- 报告中列出迁移前后代码行数（不含测试）、删除的自制轮子、迁移阻塞（若有）。

## 7. 插件目录清理清单

迁移后，一个插件目录只保留：

- `manifest.json`
- 源码：`.ts`、`.slint`（以及确实是手写源码的 `.mjs`，例如 `workshop/engine.mjs`）
- `tests/`（统一放测试；测试文件名 `*.test.ts` 或 `*.test.mjs`）
- `README.md`（简短：功能、入口、限制）
- 有 npm 依赖时：`package.json`、`package-lock.json`；有第三方许可证义务时：`licenses/` 与依赖清单（`workshop/dependency-inventory.json`、`youtube/dependencies.lock.json` 保留，内容不变）

删除：

- 交接与取证产物：`FREEZE.json`、`CANDIDATE.json`、`freeze-ready.json`、`CHECKS.json`、`PROVENANCE.json`、`publish.mjs`、`*.log`、`download-probe.json`、`real-extraction.json`、`manifest.production.json`
- 被公共工具取代的：`build.mjs`、`build.cjs`、`check.mjs`、插件内 `tsconfig*.json`（若公共工具需要每插件配置，以公共工具规定的方式提供）
- `API_REQUESTS.md`：其中仍有效的内容由协调者在 P6 合并进 `doc/rebuild/api-requests.md`，迁移者在报告中摘录仍未解决的条目即可
- 已提交的构建产物：`dist/`、`workshop/main.js`

`workshop/vendor/`（21 MB 已提交的 npm 依赖）本轮保持现状，不移动。

## 8. 验证

### 8.1 全量静态检查（P4 结束时由维护者执行，P5 前由协调者复核）

在 `src/` 下：

```
npm run check                 # tsc，0 错误
npm run format:check          # prettier
npm run boundaries            # 插件导入边界
<公共构建工具> build/test 全部插件
cd host && cargo fmt --check && \
  CARGO_TARGET_DIR=/tmp/vestra-review-target cargo clippy --locked --all-targets -- -D warnings && \
  CARGO_TARGET_DIR=/tmp/vestra-review-target cargo test --locked
```

### 8.2 全量 GUI 测试（协调者）

使用全部插件的新构建产物，在同一个宿主进程中加载 10 个插件，通过真实鼠标键盘事件和截图逐项操作。测试期间用户不应操作这台电脑（会切换桌面空间并占用屏幕），并用 `caffeinate -d` 防止显示器休眠。测试媒体：`/tmp/vestra-continuity-media/continuous-900s.mp4`、`native-verification.srt`；ROM：`/tmp/vestra-acceptance-current/current-homebrew.nes`。

| 插件 | 必测场景 |
| --- | --- |
| 全部 | 10/10 激活；右键级联菜单各领域分组正确、每个插件的菜单项都在且可用 |
| 基础播放 | 菜单和底栏“打开”；播放/暂停；拖动进度条跳转；倍速；静音与音量 |
| 播放列表 | 添加文件、选中播放、上一项/下一项、播完提示、外部来源切换提示 |
| 画面调节 | 亮度/对比度/高斯实际作用于画面，复位，连续拖动时播放不中断 |
| 音频工作台 | 拖动均衡器后点预设滑块同步，复位，状态不卡“待确认”，播放不中断 |
| 字幕 | 载入外部 SRT，中英混排显示 |
| 截图导出 | 保存当前画面：确认行可见，PNG 生成且与源帧一致 |
| NES | 打开 ROM 出画面，Stop 后画布清空 |
| YouTube | 公开视频播放（`jNQXAC9IVRw`） |
| Bilibili | 公开视频播放（`BV1xx411c7mD`） |
| 插件工坊 | 生成 → 校验 → 安装 → 生成的计数器可点击 |
| 停用 | 停用任一处理插件只撤销它自己的效果，其它插件效果保留 |

每项记录：操作步骤、截图文件、结果（通过 / 失败 + 现象）。结果写入 `sdk-v2-progress.md`。

## 9. agent 报告格式

每个子 agent 结束时的最终回复必须按以下结构（中文）：

```
## 结果
一句话：完成 / 部分完成 / 阻塞。

## 改动
- 文件列表（新增 / 修改 / 删除）

## 检查
- 实际执行的命令 → 结果（退出码、通过/失败数）
- 未执行的检查及原因

## 迁移阻塞（若有）
- 现有行为 / 旧代码位置 / SDK v2 为何无法表达 / 最小复现

## 发现但未修的问题（若有）

## 数据（迁移者）
- 迁移前后源码行数（不含测试）；删除的自制轮子
```

## 10. 记录

协调者在 `doc/rebuild/sdk-v2-progress.md` 维护：各阶段状态、每个 agent 的分派时间与结论、迁移阻塞及裁决、审查结论、GUI 测试结果。不把关键决定只留在对话里。
