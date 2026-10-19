# Pi 插件工坊方案 C1

日期：2026-10-17。阶段 C 设计稿，未实现、未验收；依据 [交互 B1](../product-interaction.md)、[菜单 B1](../menu-map.md)、[初版 API](../../rebuild/api.md)；本文不发布新接口。

## 1. 职责与使用场景

工坊是可选 TS 插件，用户通过一个原生窗口完成需求→Pi 工具动作→候选文件→真实校验→诊断修复→试载→显式安装→回退。Rust 只提供通用 UI、校验和包生命周期；Pi、提示词、mock/provider、候选生成策略留在工坊包。
独立运行只依赖共同宿主/SDK、Node 和随工坊分发的 JS 依赖；其他七个产品插件可全部缺席，无媒体亦可完整操作。组合场景不暂停、不重开媒体来源，不占播放栏。
有限样例为原生计数器：`community.counter` 1.0.0 点击增加 1，1.1.0 增加 3，计数初值 0；窗口有“增加”按钮及“计数→增加 / 归零”本地菜单。按钮/菜单执行同一包内 action；两版本均独立可运行。
不建应用商店、远程包仓库、任意插件编辑器或付费模型服务。本轮仅支持这份结构化计数器需求，界面常显“模型：本地确定性 mock；工具与校验真实执行”。

## 2. 入口与命令表

| 功能 / action | 唯一主菜单路径 | 窗内入口 | 行为与启用条件 | 权威来源 |
| --- | --- | --- | --- | --- |
| `workshop.open` | 插件→AI插件工坊… | 无重复主入口 | 打开/置前唯一工坊窗；插件启用时可用 | 窗口公开可见性 |
| `requirement.choose` / `generate` | 同上 | 选择需求JSON…、需求编辑、生成 | 无活动生成任务；输入通过schema才可生成 | 需求快照/任务revision |
| `repair` / `cancel` | 同上 | 重试修复、取消任务 | 修复需真实失败报告；取消需可取消任务 | agent与校验/事务终态 |
| `trial.start` / `trial.stop` | 同上 | 试载、停止试载 | 当前摘要校验通过；运行中可停止 | 公共trial handle/generation |
| `install` / `rollback` | 同上 | 安装此版本、回退上一版本 | 校验通过且试载清理完成；回退需管理设施提供保留版 | 包事务/activation状态 |

包管理另有 B1 的“插件→管理插件…”公共入口，不由工坊实现或接管；工坊卸载后仍能启停已安装计数器。上下文按钮不再提升为其他顶级菜单。

## 3. 界面与生命周期

独立非模态 Slint Window，初始820×640、最小640×460逻辑像素，屏幕不足时内容滚动；底部状态和取消始终可达。控件顺序：

```text
AI插件工坊                 模型：本地mock
需求  [选择需求JSON…] [多行输入/结构化摘要]
[生成] [重试修复]       当前任务、版本、阶段
工具动作 / 真实诊断（滚动；文件、行列、code）
产物：manifest / main.ts / main.js / counter.slint
校验摘要、候选hash、当前安装版本、试载状态
[试载] [停止试载] [安装此版本] [回退上一版本]
状态 / 错误 / 操作ID                    [取消任务]
```

初始无任务、不自动生成或试载；选择JSON后展示解析结果，仍须点生成。工具日志为有上限的原生列表，完整记录写工坊自有目录；文件项仅展示自有候选文本，不能浏览兄弟包。
再次打开恢复需求、诊断、产物及运行状态，已显示则只置前。标题关闭/Escape（无编辑与弹层）隐藏窗口，保留agent、任务和试载；试载继续时明确显示状态，重新打开可停止。关闭不等于取消或卸载。
打开焦点进需求编辑；后台进度不抢焦点；文本内Space/方向键归编辑器，关闭后恢复来源焦点。计数器试载窗独立非模态，标题带“试载”；其关闭仅隐藏，停止试载才销毁资源。
停用工坊中止agent、终止其验证子进程、清除trial、关闭自有UI/订阅；安装事务已转交公共管理设施，按事务规则收敛。已安装计数器拥有独立生命周期，不随工坊停用。

## 4. 状态、真实循环与失败

需求JSON限64KiB：`{"kind":"native-counter","id":"community.counter","version":"1.0.0","step":1}`；1.1.0只允许step=3，拒绝未知字段/其他ID/版本/非有限数。文件选择接受单文件，以内容schema为准，不信扩展名；取消保持当前输入，读失败/坏JSON/超限显示局部错误且不启动任务。
输入可手工编辑同一结构；启动冻结需求、SDK契约版本、任务ID/revision。编辑产生下一草稿，不改变进行中任务。重复生成/安装按钮禁用；每次候选写入增长revision并使旧校验、试载证明失效。
任务显示queued/running/cancelling/completed/failed/cancelled及阶段，不虚构百分比；生成完成只表示agent结束且候选达标，安装另有状态。模型最多12轮工具动作、最多2次修复、总时限120秒，超界失败并保留诊断。
Pi计划使用官方 `@earendil-works/pi-agent-core` 的 `Agent`，`@earendil-works/pi-ai` 提供对应模型流类型；安装实现阶段锁定可获得的精确版本及lockfile。官方原地址已重定向，不把旧搜索摘要中的包名当契约。
[官方 Agent 文档](https://github.com/earendil-works/pi/blob/main/packages/agent/README.md)明确提供 `streamFn`、类型化工具、工具事件及abort/idle控制；[官方 coding-agent SDK](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sdk.md)另提供高层session。查证日期为2026-10-17；本有限场景选择前者，避免默认发现用户配置、bash及环境工具；不需要高层CLI或TUI。

```ts
// Pi接缝示意，待锁版后按官方类型完成；不是现有SDK可运行代码。
const agent = new Agent({ initialState: { systemPrompt, model: localModel,
  tools: [readCandidate, writeCandidate, validateCandidate] },
  streamFn: mockModelStream, toolExecution: "sequential" });
const off = agent.subscribe(recordToolAndMessageEvents);
await agent.prompt(frozenRequirement);
// cancel: agent.abort(); await agent.waitForIdle(); off();
```

mockModelStream只消费模型请求上下文，发出符合Pi类型的assistant/toolCall/终止或错误流；它不得写文件、直接调用工具、运行校验或伪造toolResult。真实Agent调度工具、记录结果、将诊断送入下一模型轮。
工具限 `read_candidate(path)`、`write_candidate(path,text)`、`validate_candidate(revision)`；路径canonicalize后必须在该任务目录，拒绝逃逸与符号链接越界。无任意shell/fs工具，无直接安装工具；安装只由用户操作公共事务。
确定性首次模型响应要求write manifest/TS/Slint，并在TS写入`const step: number = "1"`（升级为`"3"`），随后toolCall validate；真实tsc给出类型错误。下一模型轮必须观察该toolResult的文件/位置/错误码，才发write修复为数字，再toolCall validate。
诊断未包含预期真实错误时mock失败，不能盲目输出修复；候选缺失、工具异常、模型格式错误也是真失败。校验日志保存命令/工具版本、退出码、摘要和逐次诊断，不能用文件存在或agent文本“成功”代替。
校验分层：schema/包边界/契约与capability检查→本地TypeScript strict noEmit→编译JS及检查声明入口→宿主内置Slint interpreter编译/导出Window/property/callback匹配。每层都绑定同一完整包摘要，任何改动重跑。
TS生成与检查由工坊随包分发的固定版`typescript` JS库在Node子进程执行，不能依赖仓库devDependency `tsc`或在线npm安装；候选只依赖随公共SDK分发的类型/运行入口。开发阶段`tsc`仅dev依赖不足以交付此功能，打包验收必须包含离线工具及许可证。
宿主加载生产`main.js`和Slint解释产物，不编译Rust；校验服务复用宿主内置Slint版本，与实际加载一致。若宿主不支持JS入口或SDK可分发导入则阻塞实现基线，不能让终端用户安装Rust工具链补洞。
试载在临时generation和命令命名空间运行，未写安装库；校验后显式点试载，等待activate完成及UI资源创建确认。用户实际点增加/归零及本地菜单，观测0→1→2或0→3→6；没有测试后门读取插件私有计数。
试载停止等待资源撤销确认：进程、窗口、命令、订阅全部释放，再允许正式安装。同ID已安装实例可保留运行，trial必须隔离冲突；本有限样例只申请自有窗口/命令，无媒体写入或网络。
安装冻结包摘要和expectedInstalledRevision，由公共设施再次验证后进入prepare→activate→commit。prepare保留旧版与恢复信息；新activate失败不提交，恢复旧版且确认旧版ready；成功只在新activation完成、持久版本原子提交后显示。
替换允许旧版短暂暂停运行；不能声称任意插件副作用可回滚。本样例只有实例资源；若新旧切换失败，显示旧版是否已恢复，恢复也失败则明确“回退失败”并留可重试管理入口，不能标旧版可用。
新包首次安装失败回到未安装；升级失败保持上个持久版本。手动回退交换到保留的上个成功版本，重新激活并观察+1行为，计数重置0；不承诺恢复插件私有运行状态。
提交前取消：abort模型/校验、停止trial、撤销未提交候选，等待清理终态再显示cancelled；已提交后取消返回tooLate与已提交版本，提供显式回退，不把回退称为取消。
RPC超时表示结果未知：按钮显示“正在查询结果”，按operationId查询，不重发新安装。取消与commit竞态由事务原子判定；迟到事件须匹配工坊generation/任务revision/operationId，不能覆盖新任务。
工坊崩溃/停用：公共管理继续已接管事务并恢复一致版本，收回临时trial资源；候选临时目录下次启动回收，保留有限诊断。已提交包继续可用。关闭主应用按B1选择继续任务或取消并退出；不是隐藏进程承诺。

## 5. 公共 API usage 与缺口

初版已有：`host.plugins.list/load/disable`、`host.ui.open/set/show/close/onCallback`；load仅spawn且无稳定activation公共结果，不能拼成安装事务。以下全部是待审请求，非现有签名。

```ts
// 拟议调用顺序；由maintainer确定最终命名与SDK类型。
const report = await packages.validate({ directory, digest, sdkVersion });
const trial = await packages.trial({ validated: report.token, ownerTask });
await trial.waitReady(); // activation完成，非进程启动
await trial.stop();     // 等待资源清理；超时查询其handle
const op = await packages.install({ validated: report.token,
  expectedInstalledRevision, requestId }); // 仅用户点击后
const result = await packages.operation(op.id); // committed/failed/cancelled/unknown
```

| 请求 / owner | 会话、输入→输出 | 错误/取消与清理 | 最小观察 |
| --- | --- | --- | --- |
| WK-01a校验 / 公共包管理（B-09） | 不可变候选digest+SDK版本→分层诊断、短期校验token；manifest增加受审版本字段/JS入口 | invalid/versionMismatch/staleDigest；取消编译且清理临时产物，不运行activate | 错TS有真实code；坏Slint有行列；修复后才出token |
| WK-01b试载 / 管理持有handle，工坊拥有租约 | token+ownerTask→trialId/隔离generation、ready/failed、资源摘要 | activationFailed/conflict；stop和owner死亡均清窗口/命令/订阅/进程；请求结果可查询 | 已安装+1和试载+3共存；停试载仅+1仍可点 |
| WK-01c事务 / 公共管理，独立于工坊存活 | token+expectedRevision+requestId→operationId、版本/运行generation/commit状态 | stale/versionConflict/activationFailed；提交前cancel，提交后tooLate；原子发布与恢复记录，重复requestId幂等 | 升级失败旧版ready；工坊死后查询一致结果 |
| WK-01d回退 / 公共管理 | packageId+expectedRevision→保留版激活与新revision | 无保留版/冲突/恢复失败明确；只清被替换实例，保留用户产物 | 1.1.0回1.0.0，按钮/菜单实测+1 |
| WK-02 UI / 实例拥有（合并B-02/03/10） | 命令、独立窗、关闭/焦点、原生单JSON选择、日志model、本地Slint菜单 | unavailable/invalid/cancelled分开；关闭隐藏，停用移自有命令/回调；dialog不堵播放 | 选择取消不改输入，菜单驻留与播放继续 |

工坊自持任务、agent与日志，不要求通用agent后台中心。公共管理只读manifest/验证产物/生命周期事实，不开放生成包私有状态；工坊不得import生成包、读取其运行对象或兄弟包目录。
WK-01/02已提交API maintainer待评估；交互意见经root转交。没有共同基线前不以Node直接改安装目录、解析私有runtime stderr或使用load冒充实现。
BYOK未来只替换模型provider/streamFn及其凭据配置；同一工具集合、诊断、候选摘要、用户安装门槛和事务均不变。本轮网络模型入口禁用，无密钥读取、无付费调用。

## 6. 验收场景

证据必须来自本轮新产物；日志证明调用顺序，屏幕/实际点击证明UI效果，不能互相替代。以下是计划，无一宣称已通过。

| 场景 | 用户实际操作 | 可观察结果 / 必需证据 | 独立/组合 |
| --- | --- | --- | --- |
| 唯一入口与关闭 | 菜单开工坊、输入、关闭再开 | 同窗/草稿恢复，生成后台继续；鼠标键盘与窗口证据 | 独立；组合播放栏可操作 |
| 文件输入 | 原生选择有效JSON、取消、坏JSON/超限 | 接受后摘要；取消不变；错误不生成；系统面板证据 | 独立；选择面板驻留播放持续 |
| 真实修复 | 点生成1.0.0 | Pi toolCall→真实写文件→tsc失败→toolResult→下一模型修复→全层通过；完整事件/诊断/摘要链 | 独立、离线 |
| 校验反例 | 候选坏Slint/越界路径/摘要变化 | 编译诊断或拒绝，试载/安装不可用；真实校验日志 | 独立 |
| 试载及清理 | 试载，鼠标点增加、菜单增加/归零，停止 | 0→1→2→0；资源列表确认清空；关闭试载窗不算停止 | 独立；组合兄弟窗保持 |
| 安装及独立性 | 点击安装、等ready，停用/卸载工坊，管理窗重启计数器 | 持久1.0.0，新窗口按钮和菜单+1；无需工坊/Rust编译器 | 独立生产JS离线包 |
| 升级与回退 | 生成1.1.0、试载+3、装后+3、点回退 | 版本/revision和真实0→3→6；回退后0→1→2 | 独立 |
| 失败保旧 | 使用本轮专造activate抛错的候选升级 | 真实激活失败/旧版ready，旧按钮仍+1；无误报安装完成 | 独立 |
| 取消/未知/崩溃 | 验证中取消、commit竞态、延迟答复、停用工坊 | 清理终态；tooLate可回退；operation查询无重复提交；管理恢复 | 独立、受控故障注入 |
| 日常连续性 | 菜单驻留10秒、工具显隐、JSON选择；首次准备另测 | 屏幕和实际音频持续、session不变；首次Slint编译记录准备提示/耗时/闪动边界/布局恢复 | 组合基础播放+其他工作台 |

## 7. 审查与版本

C1：完整有限提案；对齐B1，无新增全局菜单。待root/交互审窗口与本地计数菜单，maintainer审WK-01/02及JS/SDK分发，独立验收审取消提交竞态和activation失败恢复反例。
实现门槛：共同基线明确包版本格式、分发运行入口、校验token、trial租约、activation-ready、事务持久化恢复、operation查询及UI接缝；未解决时记录缺口，不删验收场景或绕私有API。仅集中审查通过且收到实现任务后开始代码。
