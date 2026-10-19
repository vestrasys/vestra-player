# 全新实现执行记录

> 执行纠偏已落文档：[复盘](workflow-retrospective.md)。安装回退有限闭环与NES特化清理完成，[工作基线](development-baseline.md)已发布，首批基础播放/播放列表已正式分派编码，进度见[产品台账](product-status.md)；不再以M1–M4全项验收作为全局开工门槛。

## 2026-10-18 用户范围与执行顺序修订（优先于下文旧门槛）

本节是用户最新指示，覆盖本文及历史设计中冲突的范围和前置验收要求；保留旧记录用于追溯，不将旧门槛继续作为延期理由。

- NES 本轮只要求真实模拟器能够运行并向播放器输出游戏音画；暂不要求键盘/焦点/菜单交互、快速按键、存读档及其完整交互验收。已有通用输入接口保留，供未来插件绑定键盘和播放进度等场景；不为本轮继续完善 NES 交互而阻塞插件开发。已发现的欠载等问题如实记录，不冒称无故障。
- 存档格式、序列化、ROM 校验、文件读写、复位和恢复策略属于 NES 插件。审查公共 API 与宿主，删除 NES 身份判断、按钮映射或存读档业务；通用输入释放、音视频队列与旧数据失效机制可以保留，但不得暴露 reset/loadState 等 NES 特化原因。清理后需同步 SDK、源码、usage 与必要检查。
- 新增两个独立产品插件：YouTube 视频播放及下载；Bilibili 视频播放（本轮未要求 Bilibili 下载）。原八个组件扩为十个。播放列表能够选择/加入 YouTube 视频并播放；播放列表与 YouTube 插件不得互相读取实现、导入代码或使用专用消息通道。通过公共来源描述、提供者能力及来源状态协作；站点解析、网络请求和下载策略留在站点插件，Rust 不增加站点业务。新增插件先补最小入口/菜单/失败行为设计并集中审查，不重开全部旧方案设计。
- 当前先完成通用试载、安装和回退：真实激活与必要 UI 完成、试载资源清理、成功版本持久提交、失败恢复旧版及明确的恢复失败、可查询的取消/结果；不能把 load/spawn 当安装成功。
- 随后完成公共 API 的 NES 特化清理和可执行性检查，记录可用能力及已知缺口，发布供插件实现使用的共同工作基线，立即分派独立插件 agent 编码。不得继续等待全部公共能力的完整产品验收后才允许插件开发；插件 usage 暴露的缺口按维护者请求/迁移流程迭代，最终产品验收仍检查实际完成情况。
- 保留 Rust 公共设施、TypeScript 插件业务、Slint 原生界面职责，插件独立性、第一方 safe Rust 和禁读旧实现约束不变。原生 UI 工具绑定问题保留为验收缺口，不成为所有实现工作停滞的理由。

2026-10-17：用户明确授权开启新 goal，执行更新后的工作流。

## 当前状态索引

下文是保留原结果的执行历史；判断当前进度先看本节，不把早期“源码冻结”或“阶段A未完成”当作现状。

| 阶段 | 当前状态 | 已有依据 |
| --- | --- | --- |
| A 最小实际宿主 | 完成有限基础审查 | [bootstrap-review](bootstrap-review.md)，实际libmpv、TS进程、Slint及连续控制基础 |
| B 统一交互 | B1设计已形成 | [产品交互](../design/product-interaction.md)、[菜单映射](../design/menu-map.md) |
| C 插件设计 | 原八份完成必要窄修；新增 YouTube/Bilibili 待补设计 | [集中审查](../design/review.md)，不等于产品实现 |
| D 公共能力与共同基线 | development-2026-10-18.1工作基线已发布；完整产品验收另行进行 | 来源及处理资源有有限独立证据；双槽原生组合已实际操作；剩余门槛见下 |
| E 十产品实现与验收 | 首批基础播放/播放列表开始分派，完成0/10 | [验收矩阵](../design/acceptance-matrix.md)仍待产品实现和真实交互证据 |

D阶段门槛逐项保留：

- M1：来源有限消费者及结束后seek修复已独立验证；[V8独立原生报告](composition-evidence/independent-native-v8/review.md)确认同主窗双槽、struct/model、层级菜单驻留及语法失败保旧的有限结果，早期V4另有工具/原生打开取消观察。[V12独立复验](composition-evidence/independent-native-v12/review.md)确认响应式/全屏布局、focus/menu自动隐藏抑制及系统清理错误实收的有限正向。真实90°旋转源曾触发软件renderer SIGABRT，[原始失败](snapshot-evidence/raw-author/rotation-host-crash/command.json)保留；[自动旋转独立复验](snapshot-evidence/independent-rotation/review.md)已有限通过四角/切源/效果共存及SAR/rawPNG。[非零origin独立复验](snapshot-evidence/independent-native-burst/review.md)已有限证明origin2的公开播放/seek归零与native/raw/排除字幕导出等效，并拒绝超出实际视频尾部的产物；metadata duration仍可能高估，未知/负origin及任意时间戳跳变未覆盖。完整全局快捷键/工具焦点恢复、保存/多选/目录、面板驻留时音画、超过3秒拖动锁等仍未齐备，不能发布共同基线。
- M2：typed资源、协议往返、双owner及正常撤销有独立消费者证据；独立滤镜实验已有连续运行和暂停像素变化候选。[字幕独立报告](subtitle-evidence/independent/review.md)新增分块/CAS/owner/关闭来源与暂停显示关闭像素的有限通过。[样式独立复验](subtitle-evidence/independent-style/review.md)的main/replay及内嵌SRT撤层对照均退出0，撤层恢复字幕ROI精确匹配默认。尚缺公共资源路径下逐项真实像素/音频、故障恢复真实旧效果、完整字幕能力和实际交互连续性，不能用公共资源列表代替这些结果。
- M3：[来源对象绑定](snapshot-evidence/independent-source/review.md)及[公共raw任务](snapshot-evidence/independent-jobs/review.md)已有独立有限通过，含同对象播放/离线读取有限进展、固定时刻PNG、取消/无覆盖及owner清理。[旋转/SAR复验](snapshot-evidence/independent-rotation/review.md)补齐本轮rawPNG方向与比例的有限证据。[processed PNG独立复验](snapshot-evidence/independent-processed/review.md)的13个任务与像素对照有限通过。[MP4独立复验](snapshot-evidence/independent-mp4/review.md)已有限通过选轨/owned偏移/source0 preroll/外部字幕/旧快照及端点等实际产物对照；[native/连拍独立复验](snapshot-evidence/independent-native-burst/review.md)新增plain SRT冻结、TS有限连拍、真实活跃PNG/MP4编码取消和origin2归一化的有限通过。[音频填满但视频不足变体](snapshot-evidence/independent-audio-tail/README.md)已独立通过raw/processed拒发布和合法区间解码对照；验证期/发布竞争及本节其它未齐能力仍待验收，未发布完整M3。
- M4：[CPAL设备前提](producer-evidence/audio-device-prerequisite/README.md)及共享AV作者消费者有有限结果；tmP43M活跃Xrun保留，不证明声学连续性。typed health/提供者ACK已通过必要模块测试及作者有限运行；独立双owner的正常控制、5秒超时与迟到ACK隔离已有限通过，见本文件最新执行记录。[真实ROM核心前提](producer-evidence/nes-core-prerequisite/README.md)已验证固定JSNES/Thwaite画面、音频回调和状态回放，并已由作者接入宿主真实ROM有限AV/暂停ACK；仍缺独立原生呈现/输入/discontinuity集成。包stage有限实现通过测试；[真实Pi循环前提](workshop-evidence/pi-prerequisite/README.md)已有限验证真实工具诊断修复；trial/install/rollback及产品工坊共享设施仍待实现与验收。

Goal已由用户恢复active；此前usageLimited仅为历史限制。产品范围现为十个组件，NES 验收按顶部最新修订收敛。新增能力仍为阶段D候选；`vestra.experimental/0.1`加实验能力发现不等于P1共同基线已发布。

## 目标与阶段

以 [工作流](../plugin-probe-workflow.md) 和 [产品交互要求](../player-interaction-requirements.md) 为依据：

1. API maintainer 先建立最小可运行 Rust 宿主、成熟播放后端、TS 插件接入、Slint 原生界面、初版 API 和 usage examples。
2. 制定全局菜单和界面规范，各插件提交设计，集中审查；据此调整公共能力并发布共同开发基线。
3. 统一发放插件实现任务，维护者持续管理接口、缺陷、版本与迁移。
4. 全新独立验收者验证设计、实际交互、真实媒体处理、插件独立性及组合生命周期；修复必要问题，交付可运行程序和中文报告。

范围包括基础播放与播放栏、播放列表，以及画面调节（含高斯）、音频、字幕、截图/导出、NES、可选 Pi 插件工坊六探针。工坊使用真实 Pi 循环、mock 模型调用；不调用付费模型。DLSS 不在本轮范围。

第一方 Rust 禁止 unsafe；Rust 管公共设施和资源，TS 管插件业务，Slint 管原生界面。不自造通用 UI 框架；核心不识别插件身份实现业务。成熟后端必须满足连续播放，不能重复旧的每次调参重启播放方案。

## 旧实现隔离

新 agent 必须以无历史上下文启动。禁止读取 temp、Git 历史旧实现、历史源码/测试/API 副本或通过其他 agent 间接取得旧代码。只向作者提供整理后的需求和新设计；后续产生的新 src 和公共契约可按角色读取。

## Goal 工具状态

已实际调用 create_goal，但工具拒绝：线程存在未完成旧 goal。旧目标仍为 blocked，工具不支持替换 objective 或恢复状态；不能将未完成旧目标标为 complete 以绕过限制。本文件保存用户授权的新执行目标，工作继续，不声称新 goal 已创建。后续如工具支持目标替换再同步，不改变完成标准。

用户随后执行 `/goal clear` 清除旧目标，并明确要求直接启动新 goal。已重新调用 create_goal 成功，状态 active；新目标覆盖本文全部阶段、插件范围、隔离约束与真实验收要求。上段为创建受阻的历史记录，不再是当前阻碍。

## 当前责任与进度

- root：协调、整理无旧实现的输入、阶段审查与记录。
- rebuild_api：全新 API maintainer，负责阶段 A 的新 src、初版契约及工程规范；单点构建。阶段 A 未通过前，不发布插件实现任务。
- rebuild_bootstrap_review：全新独立审查者，只读检查阶段 A 基础与契约可信性，写 bootstrap-review.md；当前不操作 UI 或并发构建，不要求提前实现后续产品能力。
- 产品交互与插件设计：阶段 A 就绪后启动。
- rebuild_interaction：全新交互负责人，阶段 B 只设计统一菜单、界面形式和能力缺口，独占 doc/design/product-interaction.md 与 menu-map.md；不实现插件或冻结公共 API。
- 独立验收：使用全新 agent，设计形成后安排。

当前阶段：A，最小宿主及初版 API，尚未完成。

阶段 A 进展：已建立实际 libmpv 软件渲染到 Rust/Slint Image 的桥接，控制与渲染拆分工作线程。作者报告连续 45 次亮度提交期间 session 不变、时钟及渲染帧持续前进，仍需核验原始证据与原生画布；不是产品菜单或全插件验收。初版契约及工程规范已写入 api.md、engineering.md，软件渲染性能与未实现产品能力记录在 bootstrap.md。完整组件登记见 scope.md。

独立阶段 A 审查已发现必要修复：同步写插件 stdin 可能阻塞 UI，甚至让停止的超时回收无法开始；示例 dispose 的 unsubscribe 与运行时停止拒绝 RPC 的规则冲突。维护者正在修复，尚未将阶段 A 标为完成。产品菜单、嵌入挂载和任务/effect 所有权属于后续设计能力缺口，不能与已实现基础的缺陷混为一谈，也不能用缺口无限扩张阶段 A。

维护者随后将发送改为 64 条有界队列和非阻塞 try_send，由独立 writer 线程写管道；报告不读输入插件的 6000 条请求场景可触发停用回收、其他播放继续。独立审查者正在核验修复及当前证据，可在实现冻结后执行针对性的 headless 集成，不与维护者并发构建。维护者已通过 CUA 操作本轮独立 bundle，观察 Slint 中的实际视频并点击示例暂停，尚在收尾画面持续前进证据。阶段 A 保持待独立结论，尚未进入产品设计。

阶段推进：独立审查者已在冻结的新实现上执行 npm run verify，退出 0；45 次调参 1495.63ms，session 为 1、position 0.7667→2.3、渲染帧 24→70，42 次状态观察，堵塞插件最终 PID 不存在且其他播放继续。此证据与修复后源码足以支持开始交互设计，不表示产品连续交互已通过。维护者另经 CUA 观察同会话画面时间码 10.033→14.900，正式记录及独立报告收尾中。

当前阶段：B，已启动统一产品交互设计；API maintainer 收尾阶段 A 证据并持续提供能力咨询。各插件交互设计及全部插件实现尚未分发，共同开发基线尚未发布。上述阶段 A 待办叙述保留为执行历史，以本段当前阶段为准。

阶段 A 正式交付已核验：bootstrap-review.md 明确可进入交互设计，独立 headless 结果及作者原生记录已保存在 bootstrap-evidence 及同名说明中。作者原生截图在工具会话可见，但未保存全部原始 CUA 截图；持久化证据诚实区分 take_snapshot、作者操作记录与独立 headless，不将其升级为独立完整 GUI 验收。

阶段 B 初审：菜单按领域集中，截图/连拍/片段导出需统一归入视频领域，NES 打开/控制/存档不能散落文件与工具；插件工坊与管理在插件域明确归属。播放列表优先保留已讨论的可隐藏侧栏需求，不能只因当前 API 尚不支持嵌入就改成外部窗口以减少工作。交互设计负责人正在据此修正草案，尚未批准。

阶段 B1 已经交互负责人修订并经 root 审查，作为阶段 C 设计输入：领域菜单为文件/播放/视频/音频/字幕/视图/模拟器/插件，无通用工具杂项菜单；右键与主菜单共享命令树。默认播放栏常驻、列表右侧可隐藏，工具独立非模态。首次未知 UI 接入的组合准备与日常 show/hide 分开记录，不承诺首次编译零成本；已接入后的菜单驻留与调参必须连续。B1 不是共同实现基线。

阶段 C 已分发首批**设计任务**（禁止产品编码）：

| 作者 | 独占设计文件 | 当前任务 |
| --- | --- | --- |
| rebuild_playback | doc/design/plugins/playback.md | 基础播放、常驻栏、命令和公开来源能力的使用设计 |
| rebuild_playlist | doc/design/plugins/playlist.md | 可隐藏侧栏、队列策略及 session-ended 隔离设计 |

首批 C1 已提交：playback.md、playlist.md；均只改设计文件。基础播放明确无媒体速度禁用、拖动只保留最新值/释放提交、静音与来源控制待公共契约；列表明确 stable rowId、自然结束关联与原子条件推进，不把队列策略塞进核心。交互负责人正在审查，维护者意见登记在 api-requests.md，尚未批准具体签名。

第二批设计作者已启动：rebuild_video 独占 doc/design/plugins/video.md，rebuild_audio 独占 audio.md；都禁止产品编码。画面合并高斯，音频工作台不负责基础音量/静音；菜单各在所属领域。初版公共源码继续冻结。验收矩阵草稿 doc/design/acceptance-matrix.md 已建立，场景全为待验证，后续须由作者细化并独立审查。

交互审查 R1 已写 doc/design/review.md：两基础的入口、布局、连续性和独立性方向通过，仍需窄修静音、显隐表述、失败导航锚及用户重播与自动推进竞态。全局 B1 已同步：checked 为用户偏好、静音为真实 mute 且保持音量、无媒体速度禁用。作者修订未完成前不标为最终批准。新字幕作者 rebuild_subtitles 已获设计任务，独占 subtitles.md；其他设计作者继续轮换，不实现产品代码。

服务用量中断后复核：video.md（159行）、audio.md（158行）、subtitles.md（133行）已在中断前写出有限 C1 草稿，最终 agent 交付消息失败不代表文件不存在。这些草稿保留为待审，不追认批准或从头重做。恢复后启动最后三位全新设计作者：rebuild_capture→capture.md，rebuild_nes→nes.md，rebuild_workshop→workshop.md；隔离旧实现及设计-only约束不变。待三份交付后集中完成八方案/API审查，公共源码仍冻结。

capture.md 和 nes.md 已交付 C1，均为设计-only；工坊正在收尾。API maintainer 已恢复集中能力审查，产出请求台账决策和 baseline-proposal.md，不修改冻结的 src；交互负责人审第二批方案，写 review.md R2。root 已提出跨方案冲突：capture 草稿将加工片段的音频限定源音频，而 audio 草稿要求冻结处理链；统一审查需明确 raw/processed 音频、设备输出量与处理参数边界，不能自行删减处理快照范围。动态规范化采用有记录的离线预热/重算候选，不要求复制内部 DSP 历史；具体可用能力仍待实现实证。

八份 C1 草稿已齐（workshop.md 已交付），统一等待集中审查与必要修订。rebuild_playback 已恢复，仅修 R1 的 mute/output 和 restart/auto-next 意图竞态等设计矛盾，不改代码；随后轮换其他作者回填审查。API maintainer 与交互负责人均已收到完整八方案，不再以缺方案为由等待；设计签名合并与实际公共能力就绪仍是共同基线发布门槛。

R2已覆盖六探针及八插件共存，无需用户裁决；workshop交互可直接进入候选，其余五方案有明确窄修（详见review.md）。playback与playlist的R1修订均已回填C2；capture与video作者正在回填R2。原始必要场景全部保留，删除的仅是作者自行增加的首帧零过渡、DSP内部历史克隆、存档覆盖事务等过度约束，不删连续播放、真实处理、快照或来源隔离要求。API提案仍未发布实现基线。

playback/playlist/video/audio/subtitles/capture已回填C2，NES最后窄修中，workshop C1交互已准入候选。API维护者已交付P1和完整请求决策：六组有限公共设施、M1–M4实证里程碑，当前源码仍0.1。root已读P1并启动独立baseline-review，确认覆盖范围及原则，不重新无限设计。

已明确授权rebuild_api进入阶段D的**必要共享基础实验**，不是产品插件编码：优先M2实时处理风险最小实证，验证本机构建的成熟滤镜运行时参数/暂停重绘/连续性/失败恢复，保存具体成功与失败证据；再推进M1的来源事务与界面公共设施。作者不能拿候选签名直接实现产品。独立审查者拥有baseline-review.md；维护者单点写共享src和构建，GUI时段先协调，第一方safe Rust及旧实现隔离不变。P1未发布、整体goal未完成。

M2首轮已产实际report（processing-evidence/null-runtime、pcm-runtime）：十三个运行时命令均返回成功。root直接按audio-slider墙钟区间重算渲染间隔：null模式5.666秒内+171帧、最大间隔38.598ms，PCM模式5.620秒+169帧、最大38.131ms；不能使用包含故意暂停的全程最大间隔证明播放卡顿。以上仅后台renderer证据，不代表屏幕/扬声器连续性。关键失败：暂停后vf-command与裸brightness property的像素MAE均为0，尚不能证明暂停重绘，维护者正在研究成熟公开路径；独立审查者另核原始像素/PCM/失败恢复，不以command成功替代处理成功。

设计收敛：NES已回填C2；capture/audio及B1/R2已同步源0→start完整preroll，video已清除剩余visible-fence/原子图交换硬承诺。八交互方案方向均可进候选，仍须实际公共能力、正式SDK usage与最终独立发布审查才能进入产品实现。

M2独立核验见processing-review.md：三张暂停PPM在每种模式内hash一致，暂停重绘未通过；真实mpv ao=pcm输出有效非零，但只证明处理路径有产物，不证明每个参数效果或扬声器连续性。坏图命令失败后af旧图列表保留是结构证据，不代替实际效果/动态历史保留。维护者追加局部vf重配仍未改变暂停像素，正在研究成熟后端刷新路径。暂停预览的产品要求是画面更新、paused及可见position保持、无临时播放声音；不把它扩成禁止成熟后端内部为当前同帧刷新而重解码，但不得由插件显式seek/unpause伪装。

M2追加forced-redraw仍像素MAE=0，尽管重复render计数增加，不能标通过。维护者已落processing-feasibility.md并继续定位。为推进不依赖暂停滤镜的工作，经API维护者明确划分，启动全新rebuild_session：仅写src/host/src/session.rs及内嵌单测，不改backend/main/SDK、不构建。模块管理通用source/intent/event身份、entry关联输入、阶段/终因及有界操作记录；不实现队列业务，也不猜测mpv事件关联。实际backend映射与集成仍由rebuild_api单点审查/构建。此为D必要公共能力，不是产品作者提前编码。

暂停隔离测试出现正向证据（pause-isolation/report.json）：普通音频null输出、移除动态音频链并观察1.2秒时，同label vf局部刷新产生像素MAE48.492，pause=true且position1.633保持，无插件显式seek/unpause。单独runtime brightness仍MAE0。root直接核report，此结果仅支持成熟内部刷新候选，不能替代完整链结论；维护者正在以norm-pause-isolation单组对照区分动态音频链与观察时间影响，并需记录实际首次像素变化延迟。

恢复核验：`norm-pause-isolation/report.json`在完整动态音频链下也记录局部vf刷新后像素MAE48.492、paused=true、position1.633不变，不能将先前失败归因于动态规范化。新增`norm-pause-latency/report.json`同样正向，renderer签名首次变化距命令约109.458ms；这是本轮软件renderer候选证据，不是原生窗口呈现延迟或真实设备无声证明。早期长流程失败仍保留，原因未定，不据此宣布完整M2通过。已委托独立审查者复核新PPM和计时逻辑。

来源模块`src/host/src/session.rs`已经写出，包含source/intent/cycle仲裁、有界operation记录、旧事件拒绝与五个内部单测。恢复时尚未由maintainer接入backend或编译执行，不能当作可用SDK能力。维护者与helper此前均因服务用量限制终止，root已向原agent发送继续任务并重新核对文件，不从头重建或转用旧实现。

当前仍在阶段D：必要共享能力实现与有限实证；已有阶段A源码不再全局冻结，但产品插件实现仍未授权。八份交互设计已提交，后续以共同设计/API/SDK/宿主实际对齐和独立审查为发布门槛。

新暂停证据已由原独立审查者追加复核到processing-review.md：三组PPM均有实际变化，允许继续候选。109.458ms只由report和当前抽样签名代码支持，未保存原始签名时序，不能写作独立重算或屏幕呈现延迟。`ao=null`不能证明暂停刷新期间设备无临时声音。

下一有限交付已明确：rebuild_api接管session集成，增加后端确认事实入口及load失败终态，以真正mpv entry关联验证source候选。rebuild_session转为隔离辅助任务，仅拥有`src/host/src/composition.rs`与新`src/examples/composition-demo/`：标准Slint import/global别名组合底栏与侧栏、原生Image和primitive桥接；不改主路由/SDK、不构建或操作GUI，由maintainer单点审查集成。此拆分不是另建UI框架，也不提前授权产品实现。

M1来源语义补充实证：root执行`python3 doc/rebuild/source-evidence/check-keep-open.py`，退出0。脚本生成独立0.7秒视频，通过本机mpv0.41 CLI IPC、`keep-open=yes`和null音视频输出观察2.5秒，收到`eof-reached=true`但退出前无`end-file`。原始事件和report同目录保存。此为后端语义检查，不是Rust宿主/插件或GUI通过证据；已交maintainer处理保末帧时的自然结束关联，不能仅依赖EndFile，也不能把未关联的poll状态贴到新generation。实际libmpv集成仍须独立消费者覆盖自然结束、重播与迟到推进竞态。

composition辅助模块已交付并冻结，包含两个独立Slint示例和usage说明，尚未编译或GUI验证。root只读审查已交maintainer注意日常条件显隐可能重建子组件、滚动/焦点恢复，以及最终回调外发须保持有界非阻塞。global别名只解决命名冲突、不复制相同源文件的singleton，示例和包加载需保留隔离边界。独立审查者亦复核EOF原始事件约0.734秒出现eof和pause、无end-file，并直接通知maintainer；不能将pause=true本身当自然结束。

M1来源候选已接入实际backend及`src/sdk/source.ts`，保留0.1旧路由并接入同一来源仲裁。root发现enum variant字段未camelCase的风险，maintainer已补`rename_all_fields`；另已提出SDK显式operationId须正safe-integer校验、terminalReason/error需有限类型后方可共同发布。作者`verify-source.mjs`已产result，root直接读取临时目录`vestra-source-sTUkjl/result.json`：坏路径进入error，快速坏A/有效B后generation3 ready，真实自然结束为natural；restart保持generation3并从cycle3/intent3变为cycle4/intent4；pause确认且close保generation3、进入empty。作者单测/集成报告尚非独立通过。已要求持久化原始结果，独立审查者正准备独占`src/tests/source-review.mjs`，待维护者构建冻结后执行真实消费者验证，不以脚本存在作通过。

M1来源有限独立实证完成：rebuild_bootstrap_review在冻结构建执行`node src/tests/source-review.mjs`，退出0；root直接核对result与command，已保存四份原始文件到`source-evidence/independent-review/`。35条source事实/40条backend诊断，覆盖真实坏源error、三轮快A/B保持新源、保末帧natural结束、同generation重播新cycle/intent、旧auto-next stale、operation replay/conflict/query及同包重载新owner命名空间和旧定时close清理。未强制制造本机迟到旧FileLoaded/error或已经排队旧回包，不升级为所有竞态通过，更不证明原生UI/菜单/设备音频。作者结果亦已持久化到source-evidence，SDK已补正safe-integer校验与有限终因/错误类型。

为推进实时处理公共实现，按maintainer给定边界把`src/host/src/effects.rs`交rebuild_session：仅typed节点、数值校验、稳定owner排序、受限成熟表达式与runtime命令生成及内部测试，不执行后端、不构建。maintainer继续负责实际应用、失败回滚、owner生命周期和Slint组合最小集成。M1来源有限结果不代表共同基线已发布，全部产品插件仍待授权。

Slint组合已实际编译并由root通过CUA操作真实双槽视频窗口：侧栏选择/显隐恢复保选择，底栏pause保持23.87秒/frame720、play后前进，session始终1。有限观察及未覆盖边界见`composition-evidence/root-native-observation.md`，不能据此宣布菜单/dialog/model或完整M1通过。窗口关闭调用超时后已检查无实例存活。

来源独立补验发现必要缺陷：自然结束后seek0.2并unpause，实际position和帧前进，但公开phase一直ended、第二次自然结束未形成新周期。`source-ended-review.mjs`测试退出1而宿主退出0；原始failure/logs/command已由root归档`source-evidence/ended-seek-failure/`并交maintainer修复。先前有限source通过保留原范围，不覆盖或掩盖新反例。effects辅助文件亦已完成冻结，仅格式化、未build/test，待maintainer集成。

该来源缺陷已修后独立复验：原`source-ended-review.mjs`未改预期，退出0。root直接读取并另存`source-evidence/ended-seek-fixed/`四份证据：首轮ended cycle1/intent1/position1.15→seek后ready cycle2/intent2/position0.2且仍paused→unpause ready intent3/position0.25→第二轮ended cycle2/position1.15。generation保持1；失败目录不覆盖。maintainer亦报告session与effects合计10个单测实际通过，后者仍未完成owner实际应用与回滚，不作为完整M2通过。

下一辅助拆分已由maintainer给定接缝：rebuild_session仅写`src/host/src/commands.rs`及内部测试，负责owner/local命名空间、单一领域菜单树快照、状态revision/冲突、禁用busy执行拒绝与owner清理；不渲染、不操作播放、不构建。允许顶级域由构造参数输入，不识别插件身份。maintainer继续实际owner处理资源/TS-serde接入，再接命令路由与原生UI；共同基线未发布。

处理资源候选已落`processing.rs`并开始接宿主路由：宿主赋owner/manifest，完整typed配置受source generation和资源revision保护；仅候选，尚未独立运行。root读审指出owner撤销时video失败不能早退跳过audio清理、state需如实暴露后端不确定；维护者负责修正。独立审查者获`src/tests/processing-review.mjs`准备任务，待构建冻结后验证真实serde、两个owner撤一留一及拒绝stale，不将资源列表结构当作真实像素/PCM或DSP历史证据。

命令辅助模块已交付`commands.rs`和5个内嵌测试，仅rustfmt未构建。root发现顶级domain Vec转BTreeSet导致丢失统一产品菜单顺序，已交maintainer保留构造输入顺序的通用修正；不能只靠域归属正确忽略实际排布。该模型仍待宿主路由、Slint菜单呈现与真实交互验证。

两项读审问题已落具体代码：processing停用分别尝试video/audio两stage并聚合失败，失败stage写入公开backendUncertain并拒后续更新，不再早退跳过音频；这仍须实际故障验证，不能从代码推断无残留。commands经maintainer确认无并发后由helper窄修保domain_order，新增逆字典序域输入/反向注册场景，子路径稳定排序；已格式化尚未独立运行。processing真实消费者脚本已由独立审查者准备并通过node语法检查，等待维护者构建冻结，不能将该准备视为真实处理通过。

处理资源有限独立实证完成：审查者在冻结构建亲自运行`src/tests/processing-review.mjs`退出0，work后缀wUbHA8（不同于作者Smhf7r）；root直接核验并复制四份原始文件到`processing-evidence/owner-independent-review/`。typed camelCase描述往返、两个owner同localId、撤helper保自身资源、过期revision/generation与无效候选拒绝均通过。30次video提交932ms，source generation/intent/cycle保持1，position0.266667→1.233333、帧8→37，无StartFile/FileLoaded。该区间不足5秒，不冒充连续拖动验收；公共列表不证明实际像素/PCM、故障回滚效果、DSP历史或残留清理。完整M2仍待这些实证。

下一M1辅助任务已按maintainer接缝分派：仅`ui_values.rs`实现公开Slint Value与JSON有限struct/model转换（深度8、节点4096、array1024、string64KiB），无布局DSL/业务/图像、不构建；maintainer负责菜单路由/呈现和真实原生dialog。该模块支持本轮列表数据需求，不代表已发布通用UI框架。

界面数据桥已交付冻结，含5个内部测试，补累计文本1MiB及canonical字段冲突拒绝；root只读核过ModelRc所有权、共享预算和显式错误，但未构建或运行，真实Slint声明/列表渲染仍须集成验证。菜单候选将用公开PopupWindow和标准控件投影同一命令树，先做驻留draw实证；不依赖hidden no_native_menu或把该原型冒充最终层级/焦点/快捷键行为完成。原生dialog采用成熟异步候选，实际调用仍待接入。

命令实际RPC和同树popup已开始接main/host.slint，异步原生dialog候选亦已落代码并引入rfd；正在构建，尚无本轮真实面板通过证据。root读审提出两项集成问题：菜单值转换失败不得unwrap默认值而静默消失；runner统一10秒RPC超时不适合用户任意驻留的文件面板，必须保长交互请求直到选择/取消/实例结束，不能用测试消费者自加长timeout掩盖SDK丢结果。独立审查者正在准备命令真实消费者，仅验证协议/实例隔离，不替代菜单驻留draw或系统面板操作。

命令注册有限独立真实消费者已退出0，root核验并归档六份原始文件至`menu-evidence/independent-commands/`：配置域顺序、子树排序、精确owner/revision回调、disabled/busy拒绝、冲突/旧revision及停用重载均符合该协议。headless通过不等于原生菜单通过。

root首轮CUA原生操作见`menu-evidence/root-native-first-run.md`：菜单显式停留11秒前后截图视频图案和时钟前进；随后点击文件入口导致Slint deleted-parent panic，没有见到原生面板，菜单交互未通过。失败日志另存`popup-click-failure/`，维护者正在修生命周期。屏幕中文领域名缺失亦已记录待定位。runner对dialogs.open已取消普通10秒超时，但尚无面板实际驻留确认/取消证明；其余RPC仍保原超时。

原生修后复验尚未进入操作：弹窗回调先访问root再close的修正已构建；字体尝试锁定版Slint源码import本机Arial Unicode（旧register_font_from_path假设经编译证伪，未依赖内部接口或复制字体）。V3进程虽存活，CUA读取两次均超时且未见系统提示。root对本轮PID81452执行1秒线程采样，主线程326个样本全部停在Host::load读取regular manifest.json的__open，尚未UI编译；原始样本归档`menu-evidence/v3-startup-hang/`。此为实际启动挂起定位，不能假称已给Documents授权或已验证字体/崩溃修复；maintainer正在有限临时目录staging以区分路径限制，不改系统保护。

V4自包含暂存实例已通过有限原生复验，详见`menu-evidence/root-native-v4.md`及`v4-native/`原始日志：中文实际绘出、文件入口不再panic、系统面板停留11秒后取消仍收到结果、真实路径选择成功；独立工具与同位置暂停亮度像素变化、四次音频滑条拖动、工具close/show保参数均有实际CUA观察，退出后host/runner消失。暂停像素在音频链创建之前，未覆盖带norm复杂公共路径；面板驻留时主窗draw/声音、保存/多选/目录、完整层级菜单和最大刷新间隔仍未验，不能宣布完整M1/M2通过。新增stage-native-evidence.mjs只打包本轮构建，解决本次运行资源路径，不复用旧实现或修改系统保护。

下一有限块已执行分工：maintainer负责实际来源关联Ready时重放最新typed处理图，补“记录保留但mpv运行时参数未随新源恢复”的能力缺口；独立审查者准备实际像素消费者，诊断仅opt-in有界实验产物，不新增正式任意帧API，也不把当时source快照冒充缓存帧的来源/PTS栅栏。helper仅改composition.rs与新mount-demo，让标准主壳保菜单/video属性并桥接两个独立owner的底栏/侧栏及真实struct/model，main挂载生命周期/prepare替换仍归maintainer。不同实验仍未冒充同主窗集成完成。

契约对齐待办另已登记维护者：P1要求有副作用操作可幂等重放/查询；processing候选目前只递增资源revision、无请求重放，尚不满足共同基线全部未知结果恢复承诺。后续可在有界owner资源revision/配置或公共operation记录内收敛，不预建持久全局请求数据库，也不为短期候选静默降低基线要求。此提醒不阻断当前切源缺陷修复。

切源重放代码已开始落processing/backend：仅接受实际关联FileLoaded，按entry与load ticket（含cycle）确认，避免同generation restart漏重放；最新typed图重新序列化而非读取mpv初始graph字符串。root读审提醒keep-open EOF在snapshot内转换phase，新增processing来源metadata也须同步，尚未验完不能追认为切源效果成立。

composition辅助块已交付冻结：复用普通host.slint主壳、保菜单与video属性，新增有界JSON回调/struct model桥及两个独立mount-demo包、3个内部测试；仅格式化，SDK/ui.mount路由及同主窗prepare替换仍由maintainer集成。独立重放消费者脚本已准备：A红角/B蓝角合成素材，灰心像素验证baseline→runtime参数→B暂停重放；采用opt-in低频有界PPM诊断并声明来源association none，不能用observedSource当真实帧栅栏。当前未运行，不标通过。

切源重放已获有限独立像素结果：五窗口consumer退出0，root核验并完整归档`processing-evidence/replay-independent/`（原始JSON/stdio与全部PPM/素材压缩包）；作者四窗口另存replay-author。中心值128.082→186.301→B暂停186.301→含norm暂停调参216.575→同generation重播216.575，每窗3帧，实际红/蓝角标区分来源。确认同load新cycle重放，不推导物理显示、首帧fence、PCM或DSP历史；故障恢复真实旧效果等M2缺口仍在。

层级菜单视觉辅助块也已交付`ui/menu-view.slint`：仅标准Slint控件、状态/子级显示与有限键盘/游标/滚动，无build/GUI。树栈、父Popup生命周期、焦点回归及同主窗mounts仍待maintainer实际编译集成，不把源码存在视作交互通过。

root集成前读审发现该菜单行按钮在activated外部回调后再focus，父Popup同步关闭时可能重犯V1 deleted-parent故障。经maintainer确认无并发编辑，helper仅窄修行点击/Back为先focus再外部回调，Close回调后不再访问父对象；源码已冻结交回，尚未以真实GUI证明该组件安全。没有降低原菜单交互验收要求。

同主窗mount主路由/SDK已落，root与独立审查者交叉读审后收敛候选风险：准备队列最多8、同owner一个pending；停用立即撤logical mount并隐藏旧slot，剩余源码坏导致重编失败也要广播host.error；重建保size/position/fullscreen；candidate回调加shell generation，仅成功提交壳世代后才可投递，旧壳/失败candidate不靠owner存活误放。均待最终构建和实际交互，不以静态修正标通过。

下一原生时段明确交独立审查者独占CUA，root不同时操作GUI。计划验同窗底栏/侧栏/菜单、rowId与pause、日常set/show无prepare、未知坏组件保旧、坏剩余源码下停owner仍立即隐藏及退出清理。maintainer须先给冻结且真实ready的唯一自包含bundle/本轮fixture，不能由LaunchServices返回0或进程存在推定可用。旧processing消费者因新增source metadata仅改为原必需字段断言并加新来源确认检查，其余预期保留；属于兼容增量使用修正，不通过改API绕回归。

主窗实际编译先发现旧video别名与Popup外部焦点调用错误，原日志保留在`composition-evidence/shared-startup-compile-failure/`及V6目录；Rust测试通过不能证明动态Slint源码可编译。修后V7实际挂载成功，但独立CUA点击层级菜单叶子使原宿主84548发生SIGSEGV，原始macOS报告及stdio保留在`composition-evidence/shared-menu-native-crash-v7/`，整轮不通过。维护者将菜单回调改为仅投递带shell generation的内部事件，下一UI tick再关闭Popup、更新模型与路由，避免销毁仍在执行的Slint回调栈；并新增实际Compiler空壳/双槽定义检查，而非仅源码字符串断言。

V8有限独立复验已重走原崩溃步骤：侧栏第二项、底栏暂停、Commands→播放→暂停/播放，原实例仍存活，两挂载保留，视频由31.95秒/frame964恢复到35.12秒/frame1058。其余显隐、驻留、坏组件与清理仍在同实例测试，暂不标整轮通过；V7失败证据不覆盖。

下一M1窄块已批准：helper仅拥有`src/host/src/window_state.rs`纯状态模块与必要边界测试，maintainer单点接入主壳/SDK及构建。通用bottom/sidebar的preferred/effective/reason分离，普通窗口空间不足收起、全屏侧栏临时override、底栏3秒idle overlay与menu/dialog/focus/drag抑制；不识别插件名称，不添加布局DSL。几何输入必须来自扣除chrome和普通底栏后的实际画布可用区；用户显示侧栏所需放大与可用屏幕限制由真实宿主处理，纯状态不伪造成功。原V8冻结直到独立有限操作结束，后续才解冻集成。

V8独立有限操作已结束并交还GUI：显隐恢复选中值、重排后rowId、菜单驻留真实帧前进、键盘导航、未知语法错误保旧以及剩余源码损坏时停owner即时隐藏均有观察；正式报告待写。cleanup错误事件因消费者未订阅，不能宣称实收，保留后续验证。原84720由reviewer明确Close并有runtime.stop；随后对已退出app请求AX使工具拉起无参84906，该实例不是原测试延续，也不同于V7无Close的SIGSEGV。root只读ps确认并TERM精确84906，再核两PID均不存在。

V8正式独立报告已交付并由root阅读：[independent-native-v8/review.md](composition-evidence/independent-native-v8/review.md)，同目录保存完整stdio/launch及仅本轮staging的故障注入原文。有限正向与错误实收、旧壳排队回调等未覆盖项区分，当前状态索引已同步。window_state纯模块交付含7个边界测试，仅rustfmt、未构建/运行；root只读核对不代替maintainer实际集成或原生验收。

响应式下一窄块另交付`platform_window.rs`：仅当前应用公开safe Cocoa绑定，show前采集windowNumber，show后严格唯一新增可见窗关联，随后按ID取screen可用区、scale与窗口/content几何，不以key/main/title猜主窗；不可确认明确Unavailable，非mac亦不伪造能力。helper仅rustfmt、未build/GUI，root只读不代替实证；maintainer负责target依赖、随成功shell世代更新关联及实际坐标换算/屏幕限制。独立验收者已准备单实例响应式/全屏有限清单，等待新接口、冻结构建和真实ready后再操作。

响应式接入检查：maintainer已执行window_state的7个单测，报告全宿主32 tests通过；实际main新增window_tick、窗口状态/全屏请求、mountState和交互锁路由，但Slint测量/overlay及SDK尚未接齐、未编译，不能当原生闭环。root已只读确认这些路由确实落盘。

菜单事实读审又发现必要风险：Slint1.16.1标准PopupWindow的公开接口没有is-open/closed通知，close-on-click-outside可绕过宿主menu_open，留下永久自动隐藏抑制。helper只读锁版官方源码确认，内部ContextMenu的is-open不属于可用公开接口。maintainer决定尝试标准no-auto-close与主窗内容区明确dismiss路径；仍须真实编译和外点击/Escape/失焦等原生证据，未宣称修复已通过。root另允许使用普通主窗内overlay减少Popup复杂性，选择由维护者落实，不能并行造第二套UI框架。

窗口接入已产生真实构建结果：maintainer报告cargo check与TS check退出0，随后32个宿主测试（含实际Slint Compiler空壳/双槽定义检查）及build退出0；新platform safe调用也已实际编译。main已有geometry/window.state/fullscreen/mountState/interaction，Slint菜单最终采用普通主窗内MenuView overlay，menu-open由宿主唯一驱动，外点击与Escape走deferred dismiss，不再依赖不可观察的Popup自动关闭。普通栏按组件preferred-height测量、隐藏占0，全屏header/diagnostic/padding归0。root读审还提醒成功换壳须清旧focus/drag/popup锁，维护者已窄修，待最后必要preflight/build后提供V9实际实例；尚未原生验过，不能宣布响应式/全屏通过。

处理请求幂等共享辅助已交付`processing_operations.rs`与5内测试，仅rustfmt、尚未集成/构建。每owner按stage/resource/revision及完整typed payload保留有界记录和不可重执行高水位，pending128、terminal512、最多64个历史key。root静态核对并要求接入台账明确revision token与P1 operation语义的等价映射、发送前确定与query未知结果恢复，不另造无意义第二ID，不默默删除基线要求；入记录前仍须有限typed校验，不能保存任意大JSON。冻结GUI审查期间不并发接入此模块。

V10已实际ready后由root触发独立reviewer独占CUA（PID87997，`shared-ui-v10`，staging后缀w1XZU7）；root仅只读日志，未操作GUI。已确认普通隐藏偏好、FS侧栏临时隐藏、底栏偏好false不被活动强显、preftrue下idle隐藏/活动恢复以及退出普通窗口保选择的有限结果。**全屏布局未通过**：真实截图大白区/左缘残影、部分控件无frame；Slint根仍960×600而报告fullscreen=true。锁版源码读审确认is_fullscreen读Slint属性，不能单独当OS全屏尺寸确认。CUA实际缩窗拖动两次报noWindowsAvailable、窗口没变，空间收起未验；不换其他未授权UI自动化绕过。独立reviewer仅继续一次cleanup错误实收后结束，坏FS上的focus锁/临时sidebar等留修后必要补验。

maintainer源码持续冻结；helper只读锁版Slint/winit官方源码诊断root尺寸/resize机制，不修改或调用内部接口。下一修复将以关联本窗的safe native frame/content事实区分请求与实测几何，不能仅凭新布尔字段或headless结果把真实白区判通过。

全屏问题定位取得直接锁版源码证据：helper查`i-slint-compiler-1.16.1/layout.rs`239–250，顶层width/height绑定标为fixed并约束min/max；`passes/lower_layout.rs`1645起的check_window_layout/adjust_window_layout把root尺寸引用改写成fixed-width/height。root已独立只读这些源码段确认，足以解释实际原生扩窗但Slint绘制仍960×600。最小候选是顶层改preferred-width/preferred-height并保产品最小尺寸，不能用额外手动viewport或内部resize API掩盖。原生白区/残影是否全解决仍须新构建及截图，V10失败不覆盖。

V10原生时段已交还：reviewer明确Close后不再请求AX，确认原87997及该staging全部进程消失，无无参重拉。普通底栏控件实际完整可见；FS白区仍失败，焦点控件无frame导致focus长期抑制/FS临时侧栏未验，空间缩窗仍工具错误未验。

**清理证据更正：** V10实际staged示例存在host.error订阅且宿主有stderr诊断，故没有错误实收不是“没订阅”。维护者读审定位：plugins.disable排系统id0重建，随后Node Exited重复disable把同owner mount_queue/preparing清掉，导致剩余坏源码重编根本未开始。即时隐藏死owner、旧底栏仍可用的观察保持有效；系统清理事务/编译失败/错误通知尚未通过。V8同形场景也不能由“源码坏且界面隐藏”倒推重编真实发生，reviewer会追加更正，不删除原有效观察，不倒推旧次必然同根因。

GUI交还后批准最小修复：顶层preferred尺寸+产品min约束、底栏min/preferred/max适配、disable幂等并保系统cleanup；保持完整范围。原生resize须先在修正固定约束后再试一次，避免为原不可resize窗口的工具错误预建不必要API；必要时才加明确实验的真实set_size入口。处理opbook接入留后续独立块。

V10正式独立报告已由root阅读：[independent-native-v10/review.md](composition-evidence/independent-native-v10/review.md)，raw日志、窗口事件、实际订阅端源码与stage故障字节齐备。其失败不因后续修复删除。解冻后3项窄修已实际Compiler/build退出0：preferred1100×720/min720×480、底栏min/preferred/max夹取、重复disable在实例已撤时幂等return保首次系统cleanup。

V11尚未交CUA的启动核对另发现换壳PhysicalSize/新窗未定scale导致逻辑高809而非720；维护者改用公开LogicalSize按旧scale恢复。position公开路径经只读核查与size不同，保PhysicalPosition原枚举，未测跨屏仍明示。V11已精确回收，将保启动反例，不重跑坏版本。

V12实际Compiler/build退出0后唯一ready（PID88534，shared-ui-v12，stage后缀GG8pQF），根1100×720、canvas1084×582、bar60、native关联成功的日志已核给reviewer。maintainer已实际followup触发独立验收，root再次list确认其running；本轮只验修后必要resize/FS外观、focus/menu抑制、临时sidebar及cleanup真错误实收，源码重新冻结。尚无V12原生结果，不按日志宣布修复已验收。

V12独立CUA首个修后结果：OS右下角resize实际可用，缩到logical768×491/canvas752×353，侧栏真实消失且preferred=true/effective=false/reason=space；再放大后侧栏恢复、second选择保留、source1继续到52.28秒/frame1570。V10的noWindowsAvailable保原记录，不倒写当时成功。无需新增window.setSizeAPI，维护者已确认不加；FS/交互锁/cleanup实收仍由同实例继续，尚未整体判通过。root另已读startup-scale-copy-failure/README，确认V11完整stdio/launch与809状态、V12尺寸对照均保存，作者启动日志不冒充独立截图。

V12继续取得独立实际结果：FS截图铺满2940×1846区域、root1470×923，白区/残影消失；栏内LineEdit输入held后静置4秒仍显示，canvas放焦后4秒隐藏，右键菜单4秒保持。FS临时展开侧栏后普通preferred可改false而当前FS仍显示，退出后普通side隐藏/pref=false，source1视频持续。F11首按一次无变化，实际FS由菜单进入，不把那次键盘算通过；完整全局键和超过3秒pointer hold仍未测。

V12清理窄补验已真闭环：损坏stage bar后Disable sidebar立即隐藏，原stdio行20551明确ui-owner-cleanup Parse error，20552收到host.error、20553订阅端observed host.error；旧底栏随后仍可暂停236.23秒/frame7087。不是仅日志或代码假定，重复disable修复后真实执行了系统重编。原始启动仍有一次layout尚未有效前的window-policy Invalidviewport，reviewer将保报告；维护者获准后续窄修首个布局测量等待，不clamp伪造viewport、也不吞已ready的异常。V12正式报告尚在收尾，独立GUI将明确Close后交还，root继续不操作。

为D2并行推进，maintainer已直接给helper精确独占新subtitle_state.rs接口：有限source/selectionRevision CAS、不可变SRT内容/metadata、atomic register-and-select、单baseline条件恢复和owner清理；只纯模块与必要边界测试、不mod/build。TS严格解析、真实hash/上传、libass/style/backend仍是后续共享能力，未用状态模块替代真实字幕。

V12正式独立报告已交付并由root阅读核对：[independent-native-v12/review.md](composition-evidence/independent-native-v12/review.md)，同目录完整stdio/launch、window-state/source/errors提取与stage故障字节。启动单次错误、F11首次未转换、跨3秒拖动/设备音频/跨系统等未测均保留。reviewer明确Close后不再GUI，原88534及stage进程不存在，已交还；root实际通知maintainer解冻，立即进入processing有界token/query接入。共同基线仍未发布，八产品实现与最终全验范围不变。

processing有界token/query已实际接main/backend/SDK，首次cargo check与TS0；全targets先发现composition-probe复用模块缺import及Clippy分支要求，已按原问题修复，随后all-targets tests退出0（host37、probe共享测试、实际Compiler）、Clippy -Dwarnings0、build0、TS0并freeze。旧processing-review只按受控迁移改同token异参conflict及badgen消耗revision，不删原owner/serde/不load/连续30次要求。

独立operation消费者已实跑退出0，临时目录`vestra-processing-operation-review-mMpQQQ`，root已直接读result.json、external-review.json与command.json核对：无来源failed留存、completed/failed query与same payload replay、异参conflict、纯静态invalid不consume、跨source旧receipt不改变当前revision3、514个真实stale终态淘汰后unknownExpired且不重执行、64key上限、helper owner2/3重载前query未知且各自revision1新记录均通过。headless/silent16秒，host exit0。限于公共结果与真实backend接入，不强造pending饱和，不宣称像素/PCM/DSP/故障回滚通过。正式归档与旧处理资源回归正在收尾，不重复已绿五窗或GUI。

subtitle_state.rs辅助交付含8内测试，仅rustfmt未mod/build；root只读核过资源/选择/单baseline/Arc与限额语义，后续caller仍要在同串行仲裁点验证翻译原文content revision，不能只凭selection stamp或TS预检查。真实hash/上传/libass/style仍未实现，维持M2完整范围。

处理operation两次独立正式报告已由root阅读：[operation-independent/review.md](processing-evidence/operation-independent/review.md)，永久raw两子目录均pass/host exit0；原处理30次915ms、gen/intent/cycle1、frame10→38，不升格5秒/像素/PCM。冻结已交还，maintainer实际mod字幕状态后host45tests/alltargets/Clippy/build0；mountState已补当前epoch+owner双guard，未确认时pending，不冒用旧slot确认。

root另完成本轮专用mpv CLI字幕有限实证：[subtitle-evidence/README.md](subtitle-evidence/README.md)，自建素材、IPC、原始PNG与版本信息均保存。暂停0选字幕真实1503像素变化、关闭精确回基线；sub-add ret为null需真实track-list关联。auto保持off/cached会选轨，暂停样式改33416像素/偏移清除并恢复。首轮sidecar自动加载干扰保原记录，不冒称渲染失败。

字面SRT格式风险已实际确认（raw标签被解读，实体SRT字串直接绘出）。root与helper独立找到成熟WebVTT派生路径：正文单次escape &<>，FFmpeg现有decoder保护ASS特殊字符，root实际PNG见标签/花括号/字面\N在同一行原样绘出；官方锁版源码亦核对。不修改原始canonicalSRT，不自造renderer；sub-text诊断不等原文的false结果保留，不能用文本属性作视觉fence。尚非host/SDK字幕通过，仍须真实公共上传/选择/CAS/回滚与软件renderer实证。

helper已交subtitle_upload.rs有限纯byte状态+6内tests/rustfmt，尚未mod/build；维护者负责成熟base64/SHA256、SRT库、串行CAS及libmpv，helper未把内存收齐当字幕有效。全部八产品与M2–M4范围不变。

M2字幕真实driver接入继续：maintainer报告状态/上传/format共16测试实际0，成熟subtp/SHA校验已落subtitle_format.rs，候选SDK已落subtitles.ts；来源同generation实际reload seam保外部不可变资源并推进selectionRevision。helper另交subtitle_style.rs纯typed范围/稳定owner层+4tests，仅rustfmt未build，720/1080单位与窗口/视频参考须实际验证，未猜映射。reviewer已独占准备subtitle-review.mjs、node语法检查0，涵盖>1MiB分块、失败保旧、source/selection/原文revision、op/query及双owner，等最终wire和host冻结才跑，未提前GUI/像素宣称。

root并行完成M3必要前置实证：[snapshot-evidence/README.md](snapshot-evidence/README.md)。source lease CLI接受File后替换路径并unlink旧对象，held File→child stdin→FFmpeg /dev/fd/0仍seek导出旧红[254,0,0]，路径重读新蓝[0,0,255]，真实PNG与原始命令保存；dup共享cursor也实测，不将clone当并发独立reader。未Rustworker/原地写入防护，不当snapshot完成。

离线能力实查另发现PATH ffmpeg9.0.2缺ass/subtitles，但本机现成ffmpeg-full9.0.2具备所有目标滤镜，原清单保存，不安装/改系统链接。使用full绝对路径+持有文件stdin+派生VTT已产实际PNG，rootview_image见标签/花括号/字面\N，日志libass0.17.5/CoreText/PlayRes384×288与系统Arial字体；与mpv默认截图不pixel-equal，未对齐字体/单位/色彩，不能把全部差异当字幕或宣称一致。完整M3音视频链/字幕字体快照、preroll、换源撤owner、取消/无覆盖发布仍未实现，原scope不变。

字幕实际driver已落subtitle_backend.rs及main/backend/SDK/cap路由：有界上传、SRT+派生VTT双digest、串行CAS/inputOrigin、真实sub-add auto+track-list差分、select/remove与reload；首check发现SendError转换缺失后窄修，尚未独立消费或像素通过。root读审提出三项必要边界：native裸轨id须关联真实(entry,ticket)，过期load或加载/关闭期间owner清理不得用旧id碰新来源；add_file发生部分成功后差分/查询失败须清候选并恢复旧选择，不能直接drop tempfile返回假保旧；native位图/未知codec不能统一styleSupported=true。均已交maintainer处理，不因代码存在标能力完成。

M3管线另获真实加工MP4/音频前置结果：root自建幅度变化AV，以source File stdin读源0，eq/gblur+派生VTT与typed同款norm100ms/5帧（volume/EQ/treble中性）、用户delay+.2，再截2.3..4.3秒，真实H264/AAC两秒/60帧、解码192000交错48000stereo样本。完整预热PCM参考以source2.1..4.1表达同延迟，AAC归一化RMS误差2.518%；错误先截再norm丢历史误差14.092%/首400ms16.46%。所有原始命令、MP4/PCM/ffprobe与素材在snapshot-evidence/processed-preroll-cli；root看首帧见2.300秒/69帧与加工字幕。不是snapshot/jobs完成，字体一致、换源撤owner、取消/发布仍未做，未把非零PCM当设备声音证据。

字幕driver修前述三静态问题后最新Clippy/alltarget build/TS0，作者实际用独立reviewer原subtitle-review实跑pass/host0（tsAZDF），覆盖>1MiB与CAS/owner等；仍非独立实跑。root再核到note_source在source.close同generation/Empty尚未清native binding，已提必要窄修；reviewer追加第三helper“真实资源→close→实际empty→停helper”回归，原断言不删、语法0未运行，等maintainer最终freeze含该guard。不得以旧轨id的close期清理误报uncertain，作者/独立结果继续分开。

字幕公共路径独立有限验收已实际完成：[报告](subtitle-evidence/independent/review.md)。协议工作Ya9WQF通过1,410,783字节/1500cue/22块、CAS/重放、双owner和close→Empty→owner清理；像素工作uZv2Xf在真实宿主公开upload/commit后暂停0秒显示字幕，5933像素改变，关闭后精确恢复无字幕RGB。root实际查看PNG，见一个字面标签/花括号/\N示例；不扩大成全部字形/样式或物理屏幕验收。后续owner样式已进入实现，首次真实PPM发现撤层后字幕消失而非恢复默认的反例，维护者正在修复，未把59/46测试绿代替效果。

root有限字体CLI已完成，记录见subtitle-evidence/font-provider-cli：mpv关闭系统subtitle provider和嵌入字体，只加载单一本机Arial symlink，实际face/index为ArialMT/0；离线FFmpeg同face/index但仍CoreText。仅匹配本例，标准FFmpeg filter无禁fallback公开选项，精确字体lease与一致性尚未完成。未复制系统字体或扩造renderer。

字幕样式外部轨撤层反例已有限修复：仅override mode切换时对binding匹配的自有外部轨走公开sub-reload并重关联；slider同mode不reload。独立style工作3VrS1P的main12秒/replay7秒两run均host exit0，root读command/result/consumer确认：main11窗保持gen1/intent1/cycle1/paused/pos0；小白131→大黄2661变化像素（963黄），z层青963且a较晚更新不抢优先级，撤z回a、撤a回原默认的字幕下60%ROI exactDiffChannels=0。失败静态校验/stale保旧不是后端半失败回滚。replay同genrestart与B新来源默认内嵌SRT在无新字幕mutation时仍黄，原source restart会播放后由consumer一次pause，不能称restart全程暂停连续；B内嵌撤层正在追加有限真实参考对照。PPM frameSourceAssociation none，不把稳定缓存帧当PTS/物理显示fence。

M3 source_lease.rs辅助纯模块已交：safe File+MutexReader、len/mtime/Unix对象身份、stdin clone串行与4内tests，仅rustfmt未mod/build。维护者接管source接受时身份绑定、worker拿锁/取消、前后验证与失败丢输出；仲裁/UI不得等待导出Reader锁。元数据校验不能抵御保时间同长恶意覆写，未标强不可变snapshot或公共jobs完成。

独立样式必要窄补LWb5nz两新实例均exit0：control直接B默认内嵌SRT，test先A上黄层再B Ready黄，撤层后两个实际样本与独立control字幕下60%ROI exactDiffChannels=0/MAE0；根读result已核。不是灰底假恢复，也不扩成任意ASS/字体通过。全部源码freeze已交还maintainer，可继续M3，正式原始归档见subtitle-evidence/independent-style。

M3下一有限接缝已授权：source.open接受时独立打开同一对象两份File并核dev/ino，一份由mpv公开fd://N读取并保活，一份给Lease串行离线worker，避免实时和离线共享cursor；SDK来源仍原用户path。先实际libmpv读/seek/替换路径实证，再rawPNG任务，不snapshot时重开当前路径冒同来源。异步load/close旧File保活须明确，平台/相对引用不支持需如实报，源码单点构建继续。

M3实际source fd接缝已有独立有限通过：[independent-source报告](snapshot-evidence/independent-source/review.md)，m7mZwU/host8秒exit0。root读raw确认原路径inode47908879→47908888且旧路径unlink后，旧A seek公开2秒仍红，同gen重播cycle2/约.033秒仍红，新open原path gen2/cycle3才蓝；四窗snapshotSource=true且真实container mov/mp4。不是离线worker独立cursor或snapshot/jobs通过，相关门槛继续保留。

固定官方NotoCJKsc Regular 2.004单face+OFL原许可证已入src/assets/fonts/noto-cjk，commit/digest/大小见snapshot-evidence/font-asset.md；root重算SHA吻合。额外有限font-asset-cli实际mpv provider none/embeddedfonts no与FFmpeg均选NotoSansCJKsc-Regular face0，root看两PNG本例中文及字面标签正确；不同默认size，不称pixel一致或任意glyph覆盖。字体资产和CLI不是公共font lease完成。

M3职责已拆分且未冒产品完成：helper独占media_worker.rs（held lease/rawPNG/可取消worker与no-clobber发布），maintainer独占media_jobs.rs及backend/SDK仲裁；root按维护者最小接口完成font_asset.rs并交还集成，只有rustfmt0未mod/build。固定SHA/face0及共享Arc、有限cmap覆盖检查/3内tests不能当实际fontselect/任意shaping/fallback或fontlease接入完成。root要求snapshot.capture携expected完整source stamp，防同gen seek/restart迟到捕获；raw方向/SAR归一与已有输出保护保留必要验收。reviewer仅准备M3消费者工具与草稿，公共wire未冻结，不提前运行或宣称通过。

作者两agent因服务usage限制明确errored后，root实际核无cargo/rustc/host接手shared集成/单点构建，未重复源码起步。新raw注册表/worker/SDK全落，root补worker可展开panic终态、状态/结果一致、publicationStarted发布失败锁存和fallible线程启动；实际host70/probe57 tests0、Clippy0、build0。repo tsc实测阻塞读lib.es2017声明（sample/lsof），终止143不当通过；同lock临时npmci工具链+原tsconfig/fresh typeRoots实际0。作者ESlMrA公共raw烟测0，证据raw-author。

独立ZKvAA3主run及yzlXEi唯一same-source补run均0，正式报告independent-jobs/review.md；root读raw核两job旧A@12s与独立原A参考RGB exact0，noClobber哈希不变、requested cancel终态、owner实child/tmp/final清。B播/旧A导出182ms跨两个child不扩同worker；补同A对象同pid97143的106ms pos.0667→.1667/frame2→5，实际两PNG参考exact0，限于此轮非长压/设备声音。

root补SAR作者J9Q25Z成功320x180 SAR2→640x180。旋转第一素材旧metadata选项无效导致错误宽度预期，分开保无效fixture；修官方input display_rotation90且ffprobe验证真matrix后JfcFPp宿主实际SIGABRT/mp_image_crop601，未运行到导出，不能假通过或误归worker。两种反例及成功SAR原始现场分别永久归档。rebuild_api服务已恢复，root正式交还全部shared/契约/唯一builder，root此后只协调/docs；优先修旋转renderer崩溃，再按已审顺序processed快照/字体映射/真实音频MP4，不缩scope。

root并行完成有限1080字幕CLI映射研究，见snapshot-evidence/style-mapping.md：libass公开force_style支持PlayRes1080，ScaleX/Y的内部倍数1不能套ASS文件百分比100，误用巨大裁切反例完整保留。修正后的72字号两端黄色横bbox170..470一致、底部差1px，但黄像素407/710及全帧MAE.689/max155仍不同，显式RGB未消除差异；不当processed/fontlease或pixel一致通过。所有真实CLI/PNG/日志/分析保存。固定字体本地读取曾等待，维护者精确路径水合后原handle自然完成，不重复启动或把未启动算效果失败。maintainer/helper正在修真实rotationSW断言；reviewer仅准备四角方向/效果链共存消费者，未运行未通过。

自动source geometry已实际接入，固定env probe删除。初始metadata安全stage与后台持续离屏render/发布门控分开；实际decoder参数决定cardinal物理转置，下一tick观察输出尺寸/rotate0后才开放发布。Processing原vf/af set会删几何stage，已改同label add只替自身，runtime slider路径保留。作者no-env K6Fa8X host0/rawPNG180x320；root读result/command/log核decoder320x180 rotate270→cclock→out180x320 rotate0，不是仅禁旋转。维护者实报host73/probe60/Clippy/build0，独立rotation消费者4172已启动原四角/切源/效果撤销/raw参考预期，尚未取最终独立结果，不重复root作者run。

自动geometry独立复验YtSdxK已正式完成，两顺序main/tail host0且全原断言通过，见independent-rotation/review.md。root读raw核六方向/SAR、restart/两typed eq/helper撤层后geometry仍正确、raw各自独立参考exactDiffChannels0，并view实际r90 PNG见绿黄/红蓝及黑边。0/±90/180有限支持，不推任意角/首帧fence/物理显示/设备声音。旧SIGABRT保持独立原始证据。freeze已全交还维护者，继续原processed快照/字体/音频MP4顺序，不重跑绿套件、不发布共同基线。

按维护者明确最小接口，root仅新写font_carrier.rs并冻结交还：FontDirectory持固定FontAsset与Arc<TempDir>，单host构造后克隆共享唯一物化字体，create_new/完整写入/flush/Unix只读font，目录最后Arc回收；1内test验证摘要/克隆存活/最终清理，只有rustfmt0尚未mod/build。维护者负责主机初始化与Subtitles/Frozen接入，helper已独占交offline_descriptor.rs纯mapper（同样未构建）；没有把纯模块写出当processed能力或字体呈现通过。

processed PNG本轮已集成上述模块。root审查指出实时storage像素滤镜与离线SAR缩放先后顺序必须一致，以及来源Ready不等于当前load画布已经确认；维护者分别落实typed effects→SAR→字幕及geometry.ready_for门控。SDK已改Raw/Processed判别联合与capture overload。维护者最终冻结报告check/Clippy/build/TS均0、host78/probe65实际通过；root直接读测试日志确认font_carrier生命周期测试已执行。作者LvQxtZ host0及consumer-result记录旧快照在改效果/撤样式/切B后仍导出冻结字幕和处理；root实际view PNG见灰底红角标及黄中文/字面标签，589黄像素只是作者烟测，非独立parity。成功及三次失败均保存在snapshot-evidence/processed-author。已正式重启独立reviewer执行准备好的公共processed消费者，结果尚待；共同基线仍未发布。

独立processed消费者BagMMp实际host0/13个PNG任务完成；首轮外层参考调用PATH FFmpeg无subtitles滤镜失败，原始工具失败单存，不能当产品失败或通过。reviewer只修自有分析工具显式使用已验证的ffmpeg-full，复用同批原始产物，analysis-only退出0。root直接核result：旧快照撤owner/切源前后及独立旧recipe逐通道差0，raw独立参考差0；SAR2 storage模糊→SAR参考差0，故意倒序参考MAE6.88099；正负字幕offset/off对照精确符合；三配置CAS分别stale、U+10FFFF缺字unsupported。root实际view旧快照最终PNG见黄中文/字面标签及红角标网格。限于外部SRT/固定Noto/PNG，不推实时离线glyph parity、排他字体provider、MP4或声学。正式归档由reviewer收尾，freeze已交还维护者。

下一有限公共块进入owned audio offset/实际选轨/MP4 source0处理。root指出仅限制当前偏移总和会在撤owner后越界（+2000/-2000/+2000），维护者接受分别限制正/负贡献总和，保证任意子集撤销仍可表示；不是插件身份特判。helper仅交offline_audio.rs有限mapper/3内tests，rustfmt但未build/test，现归maintainer集成；字符串图不等于PCM效果或延迟补偿通过。capture C2的processed显式不含字幕选项也登记为未齐能力，不能要求截图前先改变播放器字幕状态。

维护者进一步授权helper交media_tracks.rs有限事实模块：从mpv公开track-list读取实际audio ID/ff-index/selected/external与有界元数据，Selected/None/明确ID解析仅在lavf且明确非external/stream index可得时允许离线映射；不猜ID等于索引。3内tests只有rustfmt，尚未由maintainer构建或实际双音轨验收。root只读复核后指出offset-only资源撤销也须递增聚合configurationRevision（空node图相等不能略过），已交maintainer修。native字幕只读提案保持后续候选：同源stream lease/实际索引、后台提取解析，首批仅验证可等效的纯文本SRT；不以ASS→SRT有损转换充冻结，不占控制线程，不当当前已实现。

MP4集成继续：owned offset-only配置、[]/0撤资源、按逻辑配置撤销递增revision已落；root复核Book.admit的独立high-water确认撤可见资源不会忘掉旧revision，不新增重复仲裁。helper另受maintainer授权补实际selected video lavf ff-index（拒封面/image/external/歧义），已接入PNG/MP4，不再硬编码第一video；还有2纯tests待本块最终检查。后台worker新增MP4 H264/AAC、完整二次解码验证、取消/无覆盖发布与版本facts候选。首check和一轮tests已有日志，最终版本仍需作者真实产物/窄lavf旋转回归和独立冻结，不能当MP4通过。

独立MP4 reviewer只生成了vv4LJI双音轨AV/PCM参考（没有运行宿主）：12秒48000Hz实际源AAC解码，A两轨440/880Hz且B660/1320Hz可辨；完整source0规范化与错误start4重置在导出首300ms NRMSE .82336，而全段仅.02164，实际证明单看全段会漏掉缺preroll。独立AAC192k参考roundtrip首300ms .01575/全段.000971，4秒解码192000 samples；±250ms参考由独立PCM索引与补零构建。root直接读preparation-result，正式消费者运行前固定有限误差门槛并保留此反例。实际public MP4任务尚未运行。reviewer同时发现public mute设置缺口，已提交maintainer补基础播放既定能力，不以私有mpv注入冒充公开验证。

MP4作者nGMVaR已实际生成processed/negative/raw/silent四产物，host0；root直接读result与四份ffprobe：均320×180 H264、2秒，前三另有48000Hz双声道AAC2秒，silent确实无音频流。前三真实解码各192000交错samples，对作者参考NRMSE约.0157；不等于独立验收、实时设备声音或任意音轨通过。maintainer报告public muted bool已接入并真实设true；最终冻结仍待窄lavf90°回归与取消单测修复。新增版本诊断曾在等待Reader获准前执行，破坏既有“取消不启动encoder”原断言；失败日志保留，维护者按原预期调整到获准后且先检查取消，不通过改断言掩盖。独立reviewer保持仅准备消费者，不提前run。

音频/MP4一轮冻结已实际形成：修后host87/probe74、Clippy/build/TS0，root读tests-fixed末尾与lavf旋转log；udAjjm真实90°源host0/PNG180×320。维护者归档mp4-author/freeze-sha256.json及作者产物并发出运行授权。但root在候选端点说明对照worker时发现processed先trim start/归零后再trim duration，可能误保留原end外帧；独立reviewer明确尚未启动宿主，仅fixture准备，因此正式交还修改时段，没有并发换binary。maintainer接受缺陷修复，旧整秒作者绿证据不覆盖此边界。

root实际[端点CLI对照](snapshot-evidence/endpoint-cli/README.md)保留首轮工具时间基假设失败及修正对照：30fps `[.31,.32)` 无帧；统一AVTB后旧relative流程出1帧、按绝对start:end后归零出0帧。原低精度TB的秒数裁剪还会量化，已交维护者一并处理；不是宿主修后通过。独立MP4运行继续等待新freeze，原参考/全段与前300ms误差门槛不降。

端点修后已重新freeze：offline.video_range升AVTB、在原PTS按绝对start:end裁剪后才归零，raw clip同样处理低精度时间基。作者ZHNoEN公共路径host0：raw/processed `[4.31,4.32)`均failed且publicationStarted=false/未发布；`[4.31,4.38)`均实际2帧，原始保存mp4-author/endpoint。root直接读result与consumer原始事实；host88/probe75、Clippy/build通过，SDK未变沿用本块TS0。root额外重算冻结清单35个source/SDK/binary SHA全部吻合，见mp4-freeze-root-check.json；hash相符不等于语义通过。独立reviewer已正式启动session12989，以自有vv4LJI fixture/reference执行完整公共MP4消费者，并同run补四个非对齐端点case；尚未取得独立结论，不动共享源/二进制。

独立MP4 kgTUpq已实际host0/analysis0，全部原阈值与四端点断言通过，freeze交还。root直接核result：raw第二轨/selected频谱区分，设备17/mutedtrue→91/false导出PCM精确不变；旧processed在换源/改参/撤贡献owner后PCM精确不变；source0参考首300ms各NRMSE≤.06，故意错误preroll约.8227未混淆；±250ms脉冲实测.371/1.251与独立样本位移参考一致；120视频frames/字形mask对独立recipe MAE0；正首部/负尾部静音RMS0。四非对齐端点仍0/2帧、既有文件no-clobber失败且保原、最终无child/temp。取消本轮facts=null，只证明请求被接受/终态cancelled/未发布，不证明曾进入活跃MP4编码或二次验证阶段；root已把这一剩余窄证据交maintainer后续补，不因测试名称扩结论。正式报告/完整材料由reviewer归档，原生字幕/连拍和其它M1–M4门槛继续保留。

上述MP4正式报告已完成，root核正文与原始事实相符，并view已保存的处理后首帧；完整产物/PCM/probe/参考及取消阶段限制均在independent-mp4。

下一native/sampler候选已接入：helper仅交native_subtitle.rs，再由maintainer mod/单点集成；有界成熟copy提取/严格plain SRT/cmap/owned VTT均在worker，native snapshot固定pending descriptor、coverageValidated=false，不伪造未读内容摘要；提取后只在job facts记两digest/覆盖，不改snapshot.state。legacy字幕delay比较typed style.offsetMs预期（合法非0保留）；未跟踪的legacy颜色附加值拒processed，避免静默漏效果。PNG新增sampleTime复用同一不可变快照，严格t<冻结duration；TS而非核心负责2..20张/间隔/唯一目录和停止后续策略。

作者8gKYz3 host0/pass，五PNG实际已生成；root读consumer原始snapshot确认native subrip/stream1、source-zero、coveragefalse/固定style/font，并view burst/1.png见1.500秒/45帧与黄中文字幕。root读取active-child.txt，PID4640/parent4605确是libx264 MP4编码命令，不是version或queued；随后job6 requested→cancelled/publicationStarted=false。此为作者活跃阶段证据，不冒独立通过或所有取消阶段。当前check/host91/probe78 tests/Clippy/TS0已报，最终freeze与独立native/burst仍待；rebuild_bootstrap_review已正式重启准备media-native-burst-review.mjs，未提前run/build/GUI。

native/sampler冻结后独立RMpbEu实际host0但consumer失败/外层exit1：非零origin2源capture成功，违背本块声称的零起点拒绝契约；正式失败/preparation/11产物已存[independent-native-burst](snapshot-evidence/independent-native-burst/review.md)，无重跑/降断言。该run未到完整像素分析与active MP4取消，不标总通过。文件与准备副本SHA均46cb50bd…，ffprobe format/video/subrip start2；公开source.state实际position2、duration10。意外成功回包未保存，不据此伪造其字段。freeze已交还。

根因与整体小修已获得外部后端证据：maintainer实查mpv demuxer-start-time0却time-pos2，故该属性不是“真实文件origin=0”的证明。root的[字幕提取CLI](snapshot-evidence/native-origin-cli/README.md)只证copyts原3..4等与加start_at_zero后1..2等不同，随即撤回“只改字幕flag即可支持”的假设；那不保证播放器/视频同域。只读helper进一步按官方源码及真实CLI定位到lavf matroska的auto skipinfo；[完整调查](source-evidence/origin-investigation/README.md)实证probe-info=yes+rebase=yes可得start2/time0、seek0/1/3正确，native copyts+start_at_zero与归零域一致。未改Vestra共享源码或把CLI称宿主已修。

同素材实际视频span归零后[0,8)，末帧7.967，但metadata duration仍10、越界seek会停末帧。root据此要求导出不可借最后帧或短MP4冒充请求范围：例如[7.5,9)实际video短缺，即使audio apad使format时长足够也须按video实际覆盖拒绝。maintainer已采纳成熟探测/重基准选项、capture冻结真实origin/worker-held ffprobe核匹配、统一native时间域与MP4 stream覆盖校验；先前全worker origin0 fail-closed只是过渡，不把非零来源永久排除。本goal保留普通本地媒体时间域闭合要求，下一作者zero/origin2正向等效及尾界负向实际验证后再freeze独立。


独立native/burst正式收尾：最终冻结上的HsHXOs消费者及独立分析均退出0，[报告](snapshot-evidence/independent-native-burst/review.md)与完整原始材料已归档。13张PNG及60帧MP4逐RGB通道与各自独立参考精确一致；旧快照在撤owner、切源及改变样式后仍保留旧来源/效果/native字幕。origin2公开Ready及seek0均归零，worker实际origin匹配，三个导出模式与plain0参考等效；超出实际视频尾部的PNG及raw/processed MP4明确失败、不发布。首轮origin错误保留，非以删除失败断言换取通过。真实PNG及libx264 encoder运行后公共取消均进入cancelled，匹配子进程消失且无临时产物；这不证明所有验证/发布竞态。无音轨素材未冒称音频尾部变体验收，metadata duration近似问题及未覆盖时间戳范围仍明确记录。审查者已交还冻结；维护者继续M4的CPAL0.18.2独立小实验，不改变现有宿主默认音频路径，尚未发布共同基线或分发八产品编码。


M4设备前提与管理缺口：CPAL0.18.2小实验首轮真实运行因deviceErrors=1判失败，原版未留错误详情，保留first-result/stdio且不能追溯归因。窄修增加有界错误记录、PCM限读及每阶段首64样本签名后，run60066退出0，153次回调、A/pause/B消费29184/19968/29184帧、pause非零0、A消费停止、deviceErrors=0；root另按真实输入字节独立重算签名，三项与记录精确匹配且A≠B。该有限前提不证明硬件drain、声学连续性、producer二进制背压、来源CAS/音量静音或输入释放。维护者继续共享AV纵切，helper仅独占新producer_binary.rs纯framing模块，不并发修改backend/main或构建。包设施只读检查另确认：runner虽await activate后发送runtime.ready，当前Host::load仍在spawn/insert后立即返回，ready/failed未形成公共activation barrier；必须在M4补齐真实激活/UI准备与失败清理确认，不能把load返回当安装成功。


M4工坊依赖预查（只读、未安装或实现）：独立审查者按[core精确包元数据](https://registry.npmjs.org/@earendil-works/pi-agent-core/1.3.0)、[ai精确包元数据](https://registry.npmjs.org/@earendil-works/pi-ai/1.3.0)及官方v1.3.0源码核实，两包可锁1.3.0，MIT、Node>=22.19.0，tarball在内存按registry SHA512完整性比对通过。core integrity为`sha512-2jvpxZs47PFD5mSQb/yK0pGHJ5yO7Nu22u9mFjHbSLRgu62vZ9rupimDSIp4arnqOadyv0AWvYHtCSpFoJV7ag==`，ai为`sha512-KaBI8x7q7sPoFUXICNGD9efHzDG+oRNwVvkA9jdIsSQWH1RjFssOlEGsbLtewUPPgCrvmz0l2KTgSLXTyj3acw==`；正式安装时仍须由锁文件与实际获取重核，不将预查当已分发。两个tarball未包含LICENSE，分发需显式保留[官方根许可](https://github.com/earendil-works/pi/blob/v1.3.0/LICENSE)及实际依赖notices。

实际接口依据为[v1.3.0 Agent类型](https://github.com/earendil-works/pi/blob/v1.3.0/packages/agent/src/types.ts)、[Agent实现](https://github.com/earendil-works/pi/blob/v1.3.0/packages/agent/src/agent.ts)及[公开事件流](https://github.com/earendil-works/pi/blob/v1.3.0/packages/ai/src/utils/event-stream.ts)。可由真实Agent注册read/write/validate工具并用streamFn替换模型；mock仅发toolCall/assistant事件，真实Agent负责执行工具、回填结果和下一轮，不能由mock伪造toolResult。v1.3上下文messages包含system及工具声明，不沿用旧context.tools假设。tsc真实诊断放模型可见的content，不能仅放details；取消必须同时abort/waitForIdle及按signal终止回收真实验证子进程。prompt返回还需核errorMessage/stopReason/最终同digest校验，业务成功不是“循环结束”。本项只是后续必要包设施与工坊实施的接口准备，未调用模型或付费服务。


M4并行有限模块交付：维护者精确分派后，helper交付producer_audio.rs的CPAL/rtrb有界队列、epoch-tag/暂停确认及5项未执行单测；root交付package_stage.rs，随后两文件均冻结归维护者单点整合。package Candidate只捕获私有TempDir快照与SHA256、独立解析带version的新候选清单，旧plugins parser尚未迁移；限制1024文件/64MiB累计/32MiB单文件/UTF8路径1024bytes/depth12，拒symlink与特殊文件，metadata核对只检测已知普通变化。Clone共享stage寿命，entry必须是stage内生产.js/.mjs；不执行、不验证JS依赖图/Slint、不实现安装库。5项FS测试仅写出，rustfmt退出0，尚未构建或执行，维护者需整合检查。

额外M3负例只备素材：[audio-tail preparation](snapshot-evidence/audio-tail-preparation/README.md)由独立审查者实际生成origin0视频2秒/音频与容器4秒，自有裁剪[1.5,3)可完整解码，但video仅.5秒、audio及format1.5秒，说明只查容器时长可误过。未启动宿主或GUI；下一统一冻结须经公共raw/processed任务验证拒发布，不能把CLI反例准备当worker通过。


M4共享模块整合进入实际编译：维护者check3退出0（2.01s），当前producer/CPAL/rtrb/package-stage的all-targets类型检查通过，不代表新单测或运行验证通过。package-stage独立静态审查未发现纯捕获模块阻断，但提交/试载仍需保证stage不变或重新核digest，旧manifest parser对version的迁移尚待接线。root来源路径读审另指出并交维护者修复：provider未ready的校验应在SourceSession状态变化之前且保已有operation replay优先；不支持restart不得绕过operation.finish；close/切文件需真正回收旧producer而非仅停消费，以允许同owner再次准备。主线下一补这些路径并执行必要模块测试及真实纵切。当前pause只隔离宿主音频epoch；提供者frame-bound控制request/ack、输入释放和实际核心暂停仍是共同基线前必要接缝，不能把host pause当NES核心已暂停。


M4新增模块已实际执行必要单测：root复核维护者本轮producer-tests原始日志，host107/probe89均0 failed；package_stage5项、producer_audio6项、producer_binary4项及adapter_preflight保状态/receipt优先回归均实际ok。同模块在host/probe重复编译执行不按两倍独立功能计数。修后Clippy日志退出成功（2.44s），初次lint失败保留，非反复重跑已绿媒体套件。首个verify-producer.mjs作者消费者已写、node语法检查通过，但此时尚未实际执行host；root另指出close后先手动release会掩盖自动回收，以及未激活第二candidate就openFile不覆盖active producer→file，已要求首次运行前补齐真实路径或明确保首run后补验，不能用脚本存在当运行通过。


首个真实producer消费者DxvGP0已完成且失败，不能追认为通过：feature host build实际退出0（7.93s），run使用公开SDK与真实独立进程/PCM/RGBA；关闭后先absent及活跃第二producer→file两处脚本已补，但本次未走到后段。root直接复核consumer-failure：ordinary file正常ready；not-ready预检operation.failed且旧stamp保持；候选epoch1 callback acknowledged，2048PCM与首帧已入，但deviceErrors=1，故host拒激活并runtime.failed，未产生PRODUCER_SMOKE_DONE。该设备错误与早前小probe首轮同为无详情计数，不能据此推定相同原因。下一必须保存有界错误详情并按成熟CPAL错误语义定位，保原失败；不能忽略设备错误或反复重跑直到0当路线已证。此时尚无producer真实纵切通过结论。


producer分类诊断m1MzFa仍实际失败，但已得到新证据：CPAL0.18.2原始Error经预分配rtrb8条移出错误回调，非RT保存kind/message及epoch/paused、overflow；本次`epoch=1 paused=true Xrun: A buffer underrun or overrun occurred.`，active=false、receivedFrames=0、acknowledged=true、overflow0，在PCM开始前即发生。不能反推DxvGP0或小probe旧无详情错误也为Xrun。官方[错误类型](https://github.com/RustAudio/cpal/blob/v0.18.2/src/error.rs)与[CoreAudio实现](https://github.com/RustAudio/cpal/blob/v0.18.2/src/host/coreaudio/macos/mod.rs)区分Xrun、DeviceChanged、StreamInvalidated等，不由总计数认定stream已死，也不据此忽略实际故障。root同意维护者先核supported/default缓冲事实，再只做一次公开支持范围内的较大固定buffer配置修正并真实运行，保两轮失败和life计数；若仍失败再设计有据的显式恢复/epoch确认，不循环重开直到错误0。此时仍无真实producer AV纵切通过结论。


固定buffer唯一配置tmP43M已有真实结果：host与消费者退出0、脚本pass=true。公开state实收支持范围15..4096、requested=actual1024帧/48k，一buffer21.333ms；首候选激活前deviceErrors0、2048PCM/首帧齐。root直接核对完整result后确认：第一producer激活后仍Xrun（`epoch=1 paused=false`），其后deviceErrors持续1，不能声称1024解决Xrun、全程错误0或声学无故障。协议有限正向仍有效：消费时钟与帧计数推进、pause新epoch消费31744保持320ms且旧包未污染、resume至62464；not-ready保旧、seek/rate failed不留pending、旧CAS拒绝；close后在关socket/手动release之前旧id已unavailable；第二producer真实激活消费至20480且其errors0，直接切file后第二id先unavailable、文件时钟/帧恢复。videoFrames只呈现计数，首次playback观察还可能旧来源，不推导像素fence。

下一按CPAL typed错误区分可恢复Xrun/设备通知与失效/断开，保原始history和健康事实；若允许Xrun后自然继续，需明确策略并以真实后续callback/消费进展支持，不强求每次重开/新epoch或反复扫buffer到0。当前数据提供Xrun后的有限继续证据，但还未有该健康策略/独立AV/提供者frame-bound确认/输入释放，因此未发布完整M4。两早期失败已持久保存于producer-evidence/host-failures，未覆盖原始结果。


M4继续推进而非收缩范围：维护者采用typed最小健康策略候选——Xrun/DeviceChanged保历史可继续，Invalidated/Unavailable/未知为sticky fatal并停止活跃消费、公开source错误；尚待接线/实际验测。root按维护者精确签名交付producer_controls.rs纯单pending模块，Notice绑定producer/stamp/cycle/epoch/baseSample/paused，提供者全匹配ACK才交还原requester/operation，错ACK不取走，5秒到期/显式cancel；实际Book终态、提供者死亡/切源撤销和frame-bound动作均留外层。5项测试仅写、rustfmt0，未执行；模块无clock，适配层必须在处理ACK前按当前单调time先expire。源码已交维护者单点整合，不把纯echo当NES核心状态证明。

[真实NES素材](producer-evidence/nes-fixture-preparation/README.md)已从固定官方源码成功构建，未装全局工具：Thwaite revision00e36745188bc165990f60eed6b093c3ce6ad0e3；局部cc65工具revision6bd01e8a5b0e86389734fe22c679792214c01dc6。显式make thwaite.nes首轮原-static工具链接失败保留，改用本机cc编译未改源码的局部dte工具后显式make成功；无默认run或模拟器。ROM 40976bytes，SHA256 ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173，真实iNES header证mapper0/PRG32768/CHR8192/vertical/no trainer/battery；legacy PAL flag0不冒称NTSC-exclusive。源码、工具源码与许可、命令和产物均归档，仅素材准备，尚未JSNES/宿主消费或产品验收。


真ROM核心接缝已有限执行，未跳到产品实现：[JSNES前提](producer-evidence/nes-core-prerequisite/README.md)由root使用官方精确jsnes2.1.0 tarball（registry SHA512核匹配）及本轮Thwaite，在新系统tmp直接加载，未npm install/宿主/设备/Cargo。首轮360frame只版权→菜单，实际查看图片后诚实保留；第二轮再Start后离线实际看到城镇/Pine教学，540frames/431301个48k双声道sample callbacks、第二Start后102081非零、124种帧摘要。两轮均以真实toJSON文本→新核心load同ROM/fromJSON/releaseAll再同90帧输入，逐帧RGBAhash、样本计数及f32le PCM hash精确一致；第二state1325678bytes。实际Node退出0、原脚本/日志/状态/PNG/离线转换命令/依赖归档及许可均持久化。实际低位R转换匹配该版本官方Screen路径，不根据README的ARGB名直接猜字节序。这里不证明真实producer/Slint/声学/焦点输入/性能或插件存档文件；仍需完整M4与后续八产品。


控制接线新增单测已实际执行：root复核producer-controls-tests日志，host113/probe95均0 failed，包含producer_controls5项及recoverable通知继续/sticky fatal立即挡消费并保clock回归；不重复计数同模块在两binary的执行。此轮Clippy仍有diagnostics元组type_complexity必要窄修，已交维护者；真实ACK消费者尚未得到本轮通过结论，不把纯模块测试当frame-bound动作证明。


控制接线修后Clippy实际退出0（2.91s），root已读fixed原日志；TS修正采用准确Notice type alias保持Json赋值，无as any绕过。独立reviewer正在准备跨requester/provider与5秒missing-ACK消费者，先确认候选接口，未获binary冻结不运行；作者自控正常路径也尚待本轮实际执行。root另要求确认超时pending_error在Source Error落地前不能被下一同来源resume推进stamp掩盖，按有限控制preflight/主仲裁顺序处理，不扩大成任意竞态证明。


M4正常ACK与跨owner独立结果已实际核验。作者JBbLNI host/script退出0，[完整原始材料](producer-evidence/controls-author/run/result.json)保留Pending/replay/错echo拒绝/准确echo后原Book完成，仍有一次recoverable Xrun且不冒称无故障。独立同冻结binary的e4Qo9X host/outer均0，root直接核result/双方原始日志：provider PID13145/owner1，controller PID13159/owner2；101/102在232/247ms从Pending→Completed，故意未ACK的103在5070ms从Pending→Failed，来源Error/位置不再推进。新WAV Ready后旧第三Notice ack返回unavailable，新来源stamp不变；旧producer不可查、socket已ENOENT。本轮观察health均ok/xrun0只适用于该样本，不推翻旧Xrun事实或升级声学证明。两轮原始消费者错误GfFTgC（跨RPC快照误判）与3aNsmX（正常回收时不可用未收敛）完整保留在independent-controls对应失败目录；第三轮仅修owned观测/生命周期处理，保原跨owner/5秒/lateACK目标且宿主binary未改，非通过修改宿主或弱化目标换绿。独立审查已明确归还冻结/唯一设备时段，维护者继续真ROM→宿主实验；整体M4/共同基线/八产品未完成。


[真实ROM→宿主作者zJ53XJ](producer-evidence/rom-author/README.md)已实际host/script退出0，root复核完整result/command：固定JSNES/Thwaite540暖机后真实核心至632帧，音频生成431301→504782；pause与220ms held消费位置均34816、core帧/游标不增，resume实际推进至67584，关闭后旧producer直接不可查。deviceErrors0仅本轮事实，恢复时软件underrun1024→2048仍保留；pause丢弃2723个预生成样本（≤4096），新PCM基于Notice.base映射，不证明零丢样/checkpoint精确回退或声学连续。独立controls的完整[报告](producer-evidence/independent-controls/README.md)及passed-run/已归档，两次owned采样失败保留。

物理输入下一有限模块已纠正领域边界：root拒绝在Rust核心固定九物理键→八NES按钮，维护者改为最多32个有界ASCII physical code声明、完整pressed-set/scope/sequence/capture/revoke；NES A/B/Select及双Shift合并留TS，双Shift在host分别记录。helper已获仅producer_input.rs精确单文件授权，尚未构建/执行。Slint1.16.1普通KeyEvent只有text/modifiers/repeat，锁版公开safe winit hook需显式unstable-winit-030；该公开但不稳定的特性必须坦诚记录，不偷换hidden API或声称stable。主hook/焦点/可靠release及source/pause/menu/dialog联动仍待维护者接线与原生实测。另prepare已候选允许同owner active+一个candidate（总2上限），给“新ROM就绪前保旧源”留出实际路径，尚未用新ROM切换验收。


物理输入状态5项纯测试已由维护者实际执行通过，新Slint feature编译通过；不等于原生输入验收。root按单文件授权交付producer_input_delivery.rs：最多32条完整状态FIFO、每条1000ms确认期限、revoke替换普通队列为不可覆盖的空集屏障、严格提供者/完整Notice确认及仅一条成功回执重放；过期不自动出队，由外层停止来源。新增5项交付测试尚未执行，仅rustfmt实际0，源码所有权已交还维护者整合。独立审查者只读检查模块，原生焦点/菜单/快速按放与超时实测仍待进行。

输入接线首all-targets check因composition-probe缺新模块引用失败，维护者补齐后check2实际0、TS2实际0。root只读指出native keyup在32FIFO满时若仅Busy会丢失不可重试事件；维护者在实际InputBinding适配分支改为TransportFailure空集屏障，补集成回归（旧front确认stale、屏障前captureBusy、确认后pressed空）。input_筛选8项实际0，其中Delivery新增5项与adapter1项通过，另2项为既有匹配；State5此前另行通过，不重复当作原生验收。独立静态审查未发现Delivery合约阻断，周期超时/真实code验证/旧scope撤销为外层门槛。all-targets完整测试仍在运行，GUI/物理按键未执行。


输入候选构建冻结：维护者实际all-targets host124/probe106均0 failed（含实际Slint Compiler）、Clippy -Dwarnings0、TS0、feature build0，首失败及修后日志/source与binary摘要归档producer-evidence/input-build。冻结binary随后用于独立audio-tail唯一21xTnp运行（未重新构建），host/outer均0；root直接解析归档result，raw/proc两短尾任务实际video.5秒而audio/format1.5秒，均export-failed/source-unavailable、publicationStarted=false；两合法对照均30解码帧PTS0..0.966667、48000 mono PCM样本、完整解码0。见独立报告及原材料；不扩大为完整M3。设备时段已交还，下一原生输入probe尚待启动。

root另按维护者精确单文件授权交付package_activation.rs（初次激活屏障，不是完整安装器）。发现只读status与无now变更不能可靠锁住超时后，先请求维护者收敛签名，批准所有变更显式now、status可变并返回Result：每owner单调时间，deadline相等即永久TimedOut；runtime ready与必要UI全部完成才Ready，失败/成功/超时均终态，64owner与每owner64初UI上限。5项测试仅写未运行、rustfmt实际0，源码已交还；后续Host接线、trial/install/rollback/Pi仍待实现。


原生输入首轮V7OZ1P未通过，不隐藏为准备状态：唯一.app及真实ROM READY后，root获独占CUA，但inventory running而按bundle/path绑定均cgWindowNotFound、名称Invalid，未发送任何键鼠/未得截图；见[实际观察](producer-evidence/input-native-first-observation.md)。root按已授权stop正常收尾，result stopped/input receipts0/controls空/close completed；最终消费888960、软件欠载4787072、deviceErrors1活跃Xrun，不能报原生音画连续。维护者进程栈仅证明winit事件循环，未定位窗口不可见原因。下一先查现有window事实及探针供帧/RPC节奏，再有限重跑；不因工具障碍删验收范围或标完整基线。


Pi不再仅停于资料查证：全新隔离agent只在workshop-evidence/pi-prerequisite及新局部/tmp依赖实际运行官方Pi1.3.0（两包exact integrity重新核匹配）+TypeScript5.9.3 JS；root直接读脚本/events/model-requests/validations/result并重算候选及编译JS摘要。真实Agent3模型轮、6工具动作、1修复/984ms；第一实际TS2322退出1，标准toolResult.content确实进入第二模型轮，随后mock模型产生修复toolCall，真实编译退出0，独立JS行为1→2→0、摘要匹配。mock不直接做工具，未调用网络模型/用户配置密钥/付费API。初次npm配置冲突及退出码误断言失败完整保留；7.0.2最新包使用native发布结构，本前提固定可获得5.9.3以验证随包JS编译路径。README准确区分未独立故障注入的取消/恶意路径/子进程超时及未做Slint/试载/安装/生产离线打包；不以此宣布产品工坊完成。


原生输入第二诊断PjXeX5仍未通过CUA绑定，见[实际观察](producer-evidence/input-native-diagnostic-observation.md)。同冻结binary只修消费者单poll/≤4frame批次并记录publicwindow：1100×720、nativeUnavailable null、activefalse；CUA ID/path仍cgWindowNotFound，未动作/无截图。用户可见性问题保持待答，不假定锁屏；未绕CUA显前。root正常stop，实际核心9445含540warmup/消费7108609/软件欠载208895/errors0/maxpollgap591.048ms，较首脚本改善但非音画连续证据，input receipts0/controls空/close completed。发现TS逐FIFO先全部down/up再core.frame可使CPU漏采quicktap，下一应修TS采样策略而非Rust NES特判。主app仅按原315秒上限留给迟到可见性反馈，自包含artifact已隔离，可继续activation共享接线，不阻塞整个goal。


初始激活接线已落WIP：load accepted+owner+pending、plugins.activation(owner)查询、接受初ui.open/ui.mount即登记、统一respond完成、runtime事件及周期tick锁超时、先存最多128终态回执后停用；默认120秒，实验管理load支持1000..120000ms期限。查owner只在已取得owner时恢复，不冒称load幂等/丢接受响应恢复或完整安装事务。首alltargets check经仅当前两文件精确FileProvider下载后原handle自然exit0（55.72s），未重启；freshTS0。root直接核activation-tests原log，5项实际passed/0failed（44.30s编译，测试0.00s），包括无先前status时晚ready也锁TimedOut。范围错误code与runtimefault具体失败保留已窄修；Clippy/build及真实loader作者场景尚在进行，headless UI unavailable负例不当成Slint编译/原生成功UI证据。


用户恢复goal后按纠偏执行：全新隔离rebuild_packages接手当前新src通用包设施（sole shared writer/Cargo builder），先核activation失败并继续试载安装回退；site_plugin_design仅新增YouTube/Bilibili及公共来源协作最小设计，禁止读兄弟实现、无站点core分支。root准备development-baseline.md能力/缺口清单，当前仍待发布，插件编码分派数0，不以文档准备冒充开工。NES键盘/存档继续暂缓；不重跑已绿全套。


新增站点设计三文件已集中窄审通过C2，并同步menu-map与playlist范围补丁；不重审旧八方案。YouTube普通Slint播放/下载工具与Bilibili独立播放工具共享通用source路由，列表只保存原URL；SITE-01通用resolver/typed媒体描述是局部实现缺口，未在core加站点分支，也不作为其它插件开工前置。自动重解/登录/任意格式兼容不扩为全局门槛；当前仅设计通过，产品实现0。root另按维护者授权交付package_store.rs及三项未执行FS测试（仅fmt0），维护者首check进行中。


安装设施实际实现进展：新维护者已接package_manager/SDK（validate/trial/stop/install/rollback/query/cancel）及进程reaped/backend owner清理双ACK；首check发现两处错误转换待修，尚未宣称可用。root按独占授权交付package_store.rs单catalog持久Installed/pending/receipt、CAS、bytes重核与current/previous；静态审查指出commit与receipt分写的不一致后，已补prepare/commit/abort_with_receipt同次发布、pending终态容量预留、512非pending回执有界淘汰、引用GC及installed枚举，5项FS测试仅写未跑、fmt0，交维护者sole构建。rename后目录sync失败内存已更新，manager按实际安装记录/receipt收敛；原生/产品成功不由静态审查证明。下一是生产JS计数器真实试载安装坏升级保旧回退闭环，不继续做NES交互。


root亲核packages-check/tests原log：check7.97s实际0；Store5项测试0failed（含pending重开、revision回退、字节变更、receipt/GC及当时淘汰方案）。独立审查指出任意字符串requestId在淘汰后无法区分新请求/旧重放；协调者明确将实验契约收敛为最多512条持久receipt、prepare前满则busy、不淘汰、不引入新epoch平台，existing精确重放/查询继续可用；不能宣称P1过期unknownExpired已实现。维护者将按明确变更窄修容量测试及active64终态回收，结果待复验。此改动不是产品验收已过；安装实跑仍下一步。


安装首个真实有限闭环UMsFMb已作者outer0：root直接核自有tmp/result，trialId2 stopped且cleanupConfirmed=true；install-one committed owner3；bad-upgrade failed/restoredtrue owner5且持久rev1；install-two committed owner6；back-one committed owner7/rev3；cancel-upgrade cancelled/restoredtrue owner8；exact replay与commit后tooLate断言实际通过。生产JS+真实Slint Compiler，不是可见点击或counter数值行为验收。Host受控SIGTERM并await close（signal回执，不写host exit0）。硬512容量变更后的单受影响测试实际0，原五项其余不重跑；下一同artifact独立两轮及自包含native Slint协议，公共Reset/LoadState清理仍待维护者必要源码批次。尚未发布工作基线/分派产品，不因首轮通过夸完成。


安装原生Slint作者DQ91KP outer0：root亲核result true与候选capabilities commands/ui.window，包含真实创建工具Window后的激活、trial清理、安装/坏升级恢复/回退/取消终态；没有CUA可见点击。独立sflrUc两轮outer0，root亲读lifecycle/reopen/process：停止工坊manager后计数器仍ready，重开以持久current1.0/previous1.1/rev3启动真实JS PID26443，公共commands执行产生[1,2]；两host为测试后SIGTERM并await close，不报host exit0。冻结源码/binary/SDK摘要核一致；报告归档进行中。进入最后NES特化语义清理及受影响检查，随后发布开发工作基线立即分派playback/playlist，不再扩包变体或NES交互作为开工门槛。


NES特化清理源码已落：producer_input公共Reset/LoadState与SDK原因合并为通用Reconfigured/reconfigured，不改producer_audio.reset。root对host/src与sdk做词边界扫描，未见NES/JSNES/ROM/save/loadState/buttonDown业务；剩余ShiftLeft/ShiftRight均通用physical code及纯测试，不是NES按钮映射。仅静态审计，不把它当原生键盘验收；受影响必要检查待维护者最后交付。


2026-10-18 正式发布development-2026-10-18.1工作基线（wire仍0.1）：维护者实际最后TS/format/Clippy/2契约test/build/boundaries0；root重算final-source-hashes14项全匹配。安装有限闭环及独立复验满足本次准入、NES特化已清，不等待其它能力全产品验收。开始分派playback与playlist全新隔离产品作者，独占自身目录，不改共享源码或读兄弟；维护者持续真实请求处理。当前产品验收完成0/10，不能将首批授权当已完成插件。


首批产品任务已实际spawn（隔离上下文）：product_playback独占src/plugins/playback，product_playlist独占src/plugins/playlist；维护者rebuild_packages通过followup继续处理两作者真实请求。基线已发布并非仅计划；当前2/10分派、0/10独立验收完成，可运行切片待实际代码交付。新台账product-status.md记录全十组件，后续按槽位继续分派，不缩为只做两插件。


首产品代码已落：playback main/interaction/底栏Slint与dist、playlist main/queue/sidebar/link均存在。root发现drag固定旧intent会拒后续seek，作者用SeekFence完成receipt推进/外部意图取消并报告6项Node逻辑test0；playlist原每source事件堆叠24RPC Promise投影会无界积压，已改一在途+dirty/cache，作者报告7项逻辑test0（含1000请求并发1合并）。这些不代替真实媒体。宿主带有限canvas shortcut build0，首playback生产JS包UXVLl0经真实Compiler报3个conditional编辑区ID不可访问；root亲读原log并交作者修，不改Compiler/不把非Windowwarning改独立窗。首激活尚未成功，0产品验收通过，原失败保留。


playlist第一实际UI切片N0c8xH真实Compiler与activationReady已通过：root亲读result owner2 running/ready、shell2/sidebar preferred/effective true/width280/nativeActive true，命令树确已注册。尚无点击/系统多选/真实选播端到端证据，不将source empty的激活算播放功能通过。playback conditional scope失败仍作者修复中；下一修后双包共载保留同一.app时段给CUA，避免只测启动又杀掉。三agent曾用量限制停止，用户恢复后已followup续当前文件，未重做/未重启活跃检查。


## 2026-10-19 用户新增：Goal 最终宿主 GUI 修复步骤

在其余实现与功能验收之后、最终交付及 goal 完成判定之前，必须完成 [最终宿主 GUI 修复](final-host-ui-fixes.md)：纠正大按钮逐层替换式菜单为参考此前例子与 PotPlayer 的紧凑右键/悬停级联交互；定位并修复中文、混排和按钮文字不可见问题。按宿主共享 UI/绑定/渲染实现问题归责，不要求各插件各自绕过，亦不预设必须修改 API 契约。最终须独立实测可见文字、菜单鼠标/键盘/辅助功能正确性及播放连续性；编译、激活或 AX 树正确不能代替。详细要求以所链文档为准。

本次仅按用户指示登记最后步骤，goal 保持 paused，不启动修复或测试。
