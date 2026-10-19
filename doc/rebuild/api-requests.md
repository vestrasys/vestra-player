# B/C阶段公共能力请求登记（工作记录）

2026-10-17。当前实现仍冻结`vestra.experimental/0.1`；以下是设计期能力缺口，不是已发布接口，也不是准许作者现在实现平台。最终按完整usage、所有权、失败取消和独立验收集中审查。

| 请求 | 分类 / 关联 | 维护者设计期意见 | 待共同基线补齐 |
|---|---|---|---|
| PB-01 常驻栏/全屏显隐/input上下文 | 能力缺口，B-01/02/10 | 方向可行；标准Slint import组合有限底栏/侧栏，未知UI首次准备与日常显隐分开。禁止按基础插件名特判 | 挂载冲突/可见性偏好、瞬时显隐、菜单/拖动/焦点上下文的公开事实 |
| PB-02 命令/菜单/快捷键与焦点域 | 能力缺口，B-02/08 | 命令同一状态源；编辑器/主画布/NES输入域区分。标准Menu可能走平台native，必须实测draw | 命令路由/状态、主画布输入域/失焦释放、关闭与焦点恢复 |
| PB-03 单文件选择 | 能力缺口，B-03/10 | 成熟原生异步dialog候选合理；取消不改变session；Future本身不证明UI连续 | parent/平台线程适配、独立cancel结果和实际驻留验证 |
| PB-04 结构化来源/条件控制 | 能力缺口，B-04/08 | session/loading/ready/error/ended真实关联，操作携generation原子校验；mute不能等同volume=0。最新值单在途/release最终seek可行，换源丢弃旧拖动/迟到ack | 可靠mpv事件关联、来源能力、confirmed mute/volume/rate、close、请求结果查询 |
| PL-01 侧栏/model/菜单/多选dialog | 能力缺口，合并B-01/02/03 | B1仅隐藏sidebar，不增加转窗；真实Slint model/struct桥接+stable row id/selection callbacks，队列/顺序策略插件自持。用户偏好/实际可见性/空间不足原因分离 | 有限sidebar挂载、集合类型/selection桥接、多选结果与cancel |
| PL-02 可靠ended与conditional open | 能力缺口，合并B-04、PB-04 | 原子expectedCurrentSession或等价来源CAS必要；不能state检查后再无条件open。natural/replaced/closed/error分开，自然结束保留终止generation供条件比较；超时需request token查实际结果 | 通用会话事务/请求关联/终态tombstone；不向core搬队列业务 |

UI候选、稳定性边界及验证门槛见[可行性评估](ui-feasibility.md)。PB/PL作者已收到上述意见。最小反例：列表项自然结束后，自动下一项提交前用户手动open另一来源，旧自动推进必须原子拒绝；重复ended/迟到ack不能反向覆盖新来源。该边界需要宿主事务保障，不能靠兄弟插件通信或mpv Debug文本补救。

此阶段不冻结具体函数签名；后续完整插件方案可能要求合并调整。实现违反现有契约的回归另行登记，不能混作新能力请求。

## P1集中决策：八份C1合并（2026-10-17）

已读playback、playlist、video、audio、subtitles、capture、NES、workshop完整本轮C1和交互R1。所有新增均分类“能力缺口”，不把作者草案错误要求改成现有实现bug；0.1仍冻结。下面“批准”仅批准必要语义进入候选，签名/实现/版本尚未发布，具体约束见[共同契约候选P1](baseline-proposal.md)。

| 作者请求 | 决策 | 合并设施 / 必要修改或替代 |
|---|---|---|
| PB-01..03、PL-01a/b/c、VD-04、AU-04、SUB-02、CAP-03、NES-02/03/04、WK-02 | 批准有限语义 | 命令同树+两个Slint挂载/独立window+primitive/struct/model+原生dialog+input域。close hide≠dispose；preferred/effective分离。首次准备短闪与日常连续分别验证，不扩停靠平台/DSL |
| PB-04、PL-02、VD-02、SUB-02、NES-01/03 | 批准并统一 | sourceGeneration+intentVersion+eventSequence+operationId；条件比较与提交原子。restart推进intent令旧auto-next stale；自然终态保tombstone；可靠mpv关联，禁止poll-currentSession伪装。mute保volume，主动音量原子解除mute，取消“恢复上次非零值”额外历史 |
| VD-01/03、AU-01/02 | 批准owner语义，技术前置未过 | 类型化有限节点、稳定顺序、revision/CAS、accepted/applied分离、失败恢复旧配置、撤owner留兄弟。常驻节点运行时命令优先，不能每事件重启/清图；多参数确认事务不保证同一物理帧/样本无中间值。具体后端连续更新/暂停重绘/失败恢复先实证 |
| VD-02的ready-before-present/零旧帧 | 拒绝作为新增绝对目标 | 以明确pending/applied反馈、CAS防污染和实际连续性替代，不增帧栅栏平台；不能用拒绝此额外目标删掉暂停预览或正确参数要求 |
| AU-03、SUB-03、CAP-01 | 批准immutable snapshot语义，修改动态历史要求 | source/track/typed链/字体字幕小数据及lease；固定原源0..start preroll重算动态处理，不复制实时DSP历史，不声称逐样本相同。source lease保证原对象或明确失败，不强制复制大视频 |
| AU-01延迟报告 | 修改 | 用户owner偏移和算法缓存/PTS同步分开；可观测延迟记录unknown/实际测量，不猜固定250ms或强制自建DSP introspection。实际AV同步仍为必要验收 |
| SUB-01/04 | 批准并限形态 | 单选selectionRevision+owned内容/呈现层；翻译worker/词典留TS。旧结果按source/input/selectionCAS拒绝；撤owner只在仍拥有当前覆盖时条件恢复一份存活baseline，否则保较新选择。任务共用状态，不造翻译引擎；2MiBSRT走二进制/分块资源，不挤1MiBJSONL |
| CAP-02 | 批准有限任务，要求修稿 | raw源音视频；processed视频+可选字幕+确认音频工作台链/用户偏移，volume/mute不烘焙。拒capture C1排除所有音频处理的缩范围。FFmpeg离线候选须与实时同typed映射/顺序/版本、preroll可复现；PNG/20张连拍/单轨MP4，不建任意表达式平台 |
| NES-01..04 | 批准方向，具体AV传输待前置 | 同sourceCAS仲裁、bounded二进制RGBA/PCM、消费时钟、epoch/input撤销、公共pause/close/output。核心/ROM/存档留TS；共享内存精确slot数不是最终签名。真实producer/声音/切源先验证 |
| NES条件原子覆盖存档 | 拒绝无证据保证；要求改有限no-overwrite | 普通check+rename不能保证目标identity CAS。新名/排他创建/无覆盖发布保旧文件；若确需覆盖须成熟设施证据再审，不先扩全文件系统事务 |
| WK-01a..d | 批准有限包事务 | digest校验token、隔离trial/cleanup、manifest版本迁移、activation-ready、持久当前+上版、query/幂等/cancel/tooLate/rollback。失败可恢复旧版但不能假承诺旧激活永不失败；degraded明确。Pi真实锁版/离线typescript+生产JS由工坊包交付，不要求Rust终端编译 |

统一待修改责任：playback/playlist同步R1与source intent；video删除零帧绝对门槛并区分applied；audio明确延迟可观测和preroll；capture补processed音频链与no-overwrite一致；subtitles收敛单层仲裁/一份条件baseline与大内容通道；NES改覆盖保护有限替代；workshop保持有限真实Pi/离线工具与事务边界。作者仅修设计，root未下发实现任务。

待技术证据：M1来源+UI，M2实时滤镜更新/失败恢复，M3独立快照产物，M4producer与包事务；详见P1第8节。当前可发布的只有0.1阶段A及其已标注边界，P1不能作为八插件可用基线。
