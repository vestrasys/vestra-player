# 共同契约候选 P1（集中API审查）

> 2026-10-18 流程纠偏：本文为候选设计和历史实证计划，第7/8节及“先全部实证再产品实现”的文字不再作为开工门槛；以[工作流开发准入](../plugin-probe-workflow.md)和[最新执行范围](execution.md)为准。M1–M4保留为能力/最终验证台账，不要求全绿才发布工作基线。先完成用户指定安装回退与NES特化清理，随后立即分派产品纵向切片，其余缺口由实际插件请求驱动。

2026-10-17，维护者rebuild_api。当前实际可运行版本仍为`vestra.experimental/0.1`；P1只是候选语义，不是SDK、发布基线或产品实现授权。`src`保持冻结，未构建或运行GUI。输入：B1两文、交互R1及本轮C1的playback/playlist/video/audio/subtitles/capture；NES与workshop C1均已补读，本文纳入八方案API终审候选；交互负责人及独立审查意见另行合并。

本轮不照收各作者proposed方法名。合并成六组有限设施：来源与操作、原生交互、处理/字幕资源、快照任务、外部AV来源、包事务；共用实例generation、operation token、错误、清理规则。插件业务、预设、队列、翻译、Pi编排和模拟核心留插件。批准的是下述必要语义，全部新增实现仍待验证。

## 1. 统一身份、操作结果和来源条件

**批准方向，修改统一身份模型。** 宿主持有`sourceGeneration`、`intentVersion`与单调`eventSequence`。generation标识某次实际接受的新来源；phase=`empty/loading/ready/ended/error`，即使结束/关闭也保留可比较的最后generation及终因，不拿空source抹去身份。快照包含真实source caps、pause/position/duration、输出mute/volume/rate；未知数值明确null。能力不得由插件ID或扩展名推断。

- 每次来源open/producer replace、close、显式restart创建来源意图并推进intentVersion；显式来源控制（pause/seek/rate）也作为用户控制意图推进版本，以防ended后用户操作被迟到auto-next覆盖。effect/style更新和纯状态读取不推进。新open/replace才新建sourceGeneration；restart可保留generation，但必须推进intentVersion并开启新的播放周期。ended事件携真实generation、播放周期/意图版本和终因，不能使用poll时currentSession。
- 接受新来源时原子设置loading、新generation、清旧position/duration和来源能力等待态；明确`startPaused`，普通“播放文件”用false，不让迟到调用再裸unpause。ready/error只提交给其实际关联来源；旧来源事件可记录历史但不能改当前事实。来源事件必须从mpv实际entry/client关联或串行管理事务获得，不能凭Debug文本/当前generation附标签。
- open/replace/control/close/restart均可带expected `{generation,intentVersion}`。条件比较与提交在宿主同一串行仲裁点执行，stale无副作用。自动推进必须带自然结束时的二者；手动B/open、用户重播A或其他显式控制先胜出后，旧auto-next拒绝。插件仍自行决定下一行和重复ended去重，core无队列业务。
- 每个有副作用请求有实例范围operationId，首次提交前生成；重复相同ID+相同参数返回既有结果，参数不同报conflict。状态可查询accepted/pending/completed/failed，timeout只表示客户端未知；operation record采用实例范围有界保留（候选pending128、terminal512），淘汰后unknown-expired且不得自动重执行；任务/安装事务由其资源生命周期延长。不得盲重试toggle/open，不引入持久全局请求数据库。
- 状态事件携eventSequence；查询可带最新sequence，消费者用全量当前事实恢复而非要求无限历史重放。迟到ack不能覆盖后来的本地草稿或新source。数值连续编辑最多单在途+单最新待发，释放保最终值；该调度在TS，core只原子校验/序列化，不建通用手势引擎。

R1输出语义采用真实`muted`，volume不写0来模拟静音；解除保留同volume，0仍0。主动设置音量一次设置volume并muted=false，失败保确认事实；不加“上次非静音历史”子系统。设备输出volume/mute是共享全局事实，停用基础播放不复位；支持时无媒体可编辑，无媒体rate禁用。restart是独立source capability，不强迫所有来源seek0。

## 2. 命令与Slint原生界面

**批准有限能力，合并PB/PL及所有工具的重复请求。** 命令为实例范围ID+唯一主菜单路径+enabled/checked/busy/reason与执行回调；按钮/快捷键/右键投影共享同一事实，不通过兄弟私有command调用共享行为。宿主内建仅通用退出/全屏/包管理恢复入口。菜单树由B1约束，非插件身份业务。

原生接入只实现底栏、sidebar两个有限挂载位置及独立工具Window；包Slint导出组件/global，宿主生成标准import/layout组合，桥接primitive、有限struct/array Model及回调。list stable rowId由TS提供，回调携rowId/modelRevision，旧行在插件校验。不是JSON布局DSL，也不依赖hidden ComponentFactory/create_embedded。显隐偏好与effective/reason分离；close→hide、destroy、disable为不同操作。工具准备后show/hide不重新编译。焦点/全屏/菜单捕获/拖动状态仅暴露必要事实；输入域=编辑器/未捕获画布/producer画布，失焦/菜单/dialog撤销pressed-set。

首次未知UI安装/激活允许明确准备反馈、短闪和shell重建，保留backend/source/插件实例与布局状态。编译失败保旧UI，不能async冒称off-thread。非模态窗口尺寸/close/显示可基于公开Slint；跨系统窗口raise与焦点恢复、菜单native路径draw、原生dialog async都需真实前置实证，见[UI可行性](ui-feasibility.md)。不预建通用停靠/多窗口平台。

原生文件选择最小为单/多文件、Save和目录，selected/cancelled/error分开，归请求实例；停用使迟到结果失效。对话框返回不自动发open或任务，插件按捕获generation重新核对。文件选择不等于强制覆盖许可，输出策略见第4节。

## 3. owner处理资源与字幕

### 3.1 视频/音频处理

**批准所有权、有限descriptor、连续性；修改过强事务/可见要求。** 每实例拥有有限video/audio资源，宿主分配resourceId/owner generation。配置包含完整类型化节点与内部次序、resource revision、expected source/track、operationId。能力发现逐算法/可运行参数说明，不对TS开放任意vf/af/滤镜字符串；宿主适配器内部生成受限成熟后端表达式。范围/预设公式留作者。

跨owner次序固定为媒体stage→manifest id规范字符串→本实例resource局部ID，owner内部顺序按descriptor。排序公开可诊断，不依安装先后变化；使用manifest id作为通用稳定键不是插件名业务分支。当前同ID禁止重复运行。全链每个owner只撤自身，停用/崩溃宿主清理，不靠dispose，历史快照独立保留。

**确认分层：** validated/accepted只表示候选有效并已排队；applied表示后端已接受该revision的全部更新且资源状态已确认，失败不发布该revision；不提供visible-frame/PCM已输出fence。显示“应用中/已应用，画面由后端刷新”，验收看真实像素/声音。多参数事务不承诺物理同一帧/样本无中间值；必须完成整份更新才确认，任一失败恢复上份有效配置，不能遗留半个预设。若后端不能可靠恢复，则该技术路线不符合要求，不能只报错保留半链。

不接受VD“ready-before-present、任何新来源零帧旧配置”的额外绝对目标：新source确认后以CAS应用最新期望，未应用时明确pending；旧请求不得污染新source。用户需要的是正确反馈和连续调参，不为它新增帧栅栏/解码重排平台。暂停视频预览仍属必要验证项：应由成熟滤镜/renderer重绘当前帧，不能seek或短暂播放冒充。

实时数值更新优先常驻节点+后端运行时command，开关尽量用中性参数/受支持bypass，保留动态历史。不得每个滑条事件重建整个图、loadfile、重启播放器或清空动态历史。拓扑确有变化时单独验证局部重配置、失败保旧及连续性；不把“prepare再swap”写成已经存在的mpv原子图交换能力。

用户audio-delay为owner贡献，按稳定链独立合计，合成值越公共范围则拒整次更新；0只撤自身偏移。算法内部缓存/PTS延迟由成熟后端同步设施处理，用户值与实际可观测延迟分开。不能猜250ms补偿；若后端不公开精确内部延迟，不强制新增DSP状态反射，改报告未知/测量证据且必须实际达到A/V要求。此细节须audio作者修订后终审。

### 3.2 字幕

**批准资源、选择CAS与单层呈现；修改回退为有界条件恢复。** 外部规范SRT不可变内容由owner注册，宿主快照保存字节/摘要及成熟libass可复现呈现；TS严格解析/翻译策略，不逐帧绘字。本轮2MiB/10000cue上限超JSONL1MiB，采用受限二进制资源上传通道/分块登记，不靠放宽控制信道上限。资源和内容revision独立于selectionRevision。

字幕是公共单选资源（含关闭），每次选择/关闭/样式偏移更新携expected source与selection/layer revision。样式/偏移为owner呈现层；多个存在时按固定公共顺序仲裁唯一effective layer，能诊断，不创建无限字幕叠层编辑器。register-and-select是整次条件提交，失败不替换旧资源；翻译完成仅产出内容，显式应用再校验source、原文revision及最新selectionRevision。

owner撤销时只删自身资源/层。仅当当前选择仍是该owner拥有的覆盖、且没有较新选择意图，才恢复其覆盖前仍存活的基线选择，否则保留新选择；基线不存在则关闭。保存一个基线引用+selection stamp即可，不建任意时间撤销栈。关闭/用户选内嵌后译文迟到必须stale；复位取消本地应用token并条件切回原文，不覆盖他方选择。

字幕样式按同一标准参考画布/字体/libass版本记录，位图轨道不支持样式须明确；未知字体允许确认替代，捕获快照固定实际字体文件/版本或明确source-unavailable/unsupported，不承诺任意平台逐像素相同。

## 4. 不可变快照与有限任务

**批准CAP/SUB/AU共用snapshot+任务语义，修正capture音频范围。** 任务接受时原子验证expected source/intent与已applied的资源revision，固定来源身份、选轨、媒体时间/范围、效果链顺序/参数、字幕内容/样式/偏移/实际字体、算法与色彩格式、输出选择。只给TS不透明snapshot/job ID和小元数据，大帧不进JSONL。结果未知或pending处理可等当前明确结果，或返回busy让用户重试，不静默使用草稿。

来源lease只保证读原对象或明确失败，不要求复制整个大视频。可持有文件句柄/identity并在重读时校验；移动/替换后继续读到旧内容允许，无法保证则source-unavailable终止，绝不读新路径内容伪称同快照。字幕/小配置可复制，历史快照不能引用已停用插件回调；任务引用结束回收。

有限媒体任务：SDR8bit原始/处理后PNG、2..20张媒体时间连拍、单轨H264/AAC MP4；独立离线FFmpeg解码/滤镜/编码可作为成熟实现，不干扰实时pause/seek。raw不含视频效果/字幕，选定源音频不加工；processed包含冻结的视频链、可选字幕、**冻结音频工作台处理链与用户偏移**，设备输出volume/mute不烘焙。capture C1“所有模式源音频”与AU/B07冲突，需修改，不能作为缩范围批准。

同一受限descriptor同时映射实时mpv lavfi与离线FFmpeg，不接受任意表达式或无法重读的私有处理。先核同算法版本/格式/顺序和字体，像素比较允许已记录的色彩/缩放/编码误差，不能承诺bit-identical screen capture。原始PNG按源呈现方向/SAR校正，非Slint缩略画布截图。

动态规范化快照仅固定配置，不克隆实时DSP内部历史。processed片段以**源起点0到导出起点的实际范围**作为确定性重算preroll（先采用简单可复现方案），处理后裁取目标范围并重置输出PTS；记录preroll、音频track、算法版本、端点和延迟规则，提示准备可能较长，不改变实时播放。后续若要截短preroll须另发布误差/边界，不在本轮暗改。有源负偏移需读取更早/后时间并按公开规则补零/裁切，使目标片段长度与AV时间线一致。任务产物不承诺与实时规范化某瞬间逐样本相同，但参数和重算规则必须一致可验证。

任务状态统一queued/running/cancelling/completed/failed/cancelled，可query(operationId/jobId)，取消先请求后终态。媒体任务由公共设施拥有worker/临时输出；翻译/生成等插件任务可由TS worker执行并公开同状态，不预建全局工作流引擎。owner死亡使其未终态任务取消并回收worker/租约，窗口隐藏任务继续。主窗退出可枚举未终态任务作B1退出选择。

输出临时文件同目录、完成验证后无覆盖发布；已发布连拍PNG保留，未发布清理。默认拒覆盖已有用户文件，Save确认替换也不改变本轮no-overwrite规则。NES草案要求“目标identity条件原子覆盖”不是常规rename能保证，不强制扩公共filesystem事务；替代为新文件/自动新名+排他创建/无覆盖发布，作者修订交互，若坚持覆盖则需具体成熟原子设施证据后另审，不先承诺。

## 5. 外部AV来源（NES初审）

批准有限producer方向：实例拥有worker/producer，宿主拥有source generation、Slint画布/公共输出设施/输入域。RGBA8 256×240和48kHz stereo f32 PCM经有界二进制通道，JSONL仅控制；视频丢旧保新，PCM背压、欠载告警，epoch隔离暂停/复位/读档的旧数据。尺寸/队列上限及格式由声明校验，不为NES硬编码。

消费样本时钟驱动producer，普通source.pause/close按capability转给提供者，unsupported seek/rate/subtitles/capture禁用。来源replace CAS与第1节共用；候选ROM/core在插件准备，真实首帧/首批音频后再提交。source撤销先停止消费/清输入再强制回收，不能依赖插件自愿关闭设备。音频设施可通过成熟safe音频库适配，不要求与普通libmpv共用同一内部设备对象，但来源切换不得重叠播放或泄漏，公共volume/mute一致。

不冻结共享内存slot数量或具体二进制传输实现，先做单producer真实AV/暂停/输入释放/切回普通文件最小实证；10分钟性能是该样本验收，不把固定60fps变成任意设备保证。NES核心、ROM许可、存档序列化和策略留插件。

## 6. 包事务（工坊C1审查）

现0.1 load/disable只管理进程，不当安装成功。候选公共事务保留validate→trial→install/rollback、包摘要/SDK契约/Slint诊断、独立trial实例/命令命名空间、持久安装revision与运行generation分离；operationId支持查询。installation commit前可取消，提交后只能请求明确回退；activation-ready才标成功，失败保旧可运行包。

不批准工坊专用core、模型调用、生成逻辑或任意试载改现有资源。试载是可信包非沙箱，隔离命名/owner和清理不等于隔离fs/network；明确告知试载会执行可信候选。工坊C1已给当前官方`@earendil-works/pi-agent-core`/`pi-ai`及`streamFn`接缝方向，设计允许；实现时锁实际可获得精确版本/完整性并按真实类型编译，不从旧包名摘要照抄。mock只产模型事件/toolCall，真实Pi执行工具、真实tsc/Slint诊断进入后续修复。生产包main.js、可分发SDK运行入口、工坊离线自带typescript JS及许可证；用户不安装Rust、不依赖仓库devDependency或在线npm。

WK-01a..d合并为一个有限管理设施，校验token绑定候选digest+契约版本；候选变更token失效，安装再次核摘要。manifest新增包版本字段需作为0.1→候选的受控迁移，不能偷偷修改0.1 parser。trial隔离运行generation/命令命名空间和拥有的UI/订阅，只对有限计数器自有资源证明可回收，不声称任意媒体副作用回滚；工坊不import计数器实现。

持久管理库保留当前+上个成功版及事务恢复记录，不造包商店/依赖求解器。替换允许旧实例停止后启新实例；activate+必要UI注册ready并持久commit后成功，失败重新激活旧版，旧版恢复失败须明确failed-degraded而非“已保旧可用”。不承诺恢复计数器私有计数，回退新实例从0开始。提交前cancel、提交后tooLate/显式rollback与operation查询按第1节共用；安装管理接管后不因工坊死亡取消已commit包。Slint CPU编译不能抢占时cancel先记意图，等待候选丢弃清理后才cancelled。

## 7. 实现前必要技术门槛与发布条件

1. **先核实时处理可行性。** 官方FFmpeg为eq/gblur/equalizer/treble/dynaudnorm等提供不同程度runtime commands；mpv有vf/af command与lavfi包装，但不能由两份手册推断当前构建每个参数都能无重建更新，更不能推断多参数原子交换。需在已装mpv/libavfilter版本真实验证目标参数、暂停重绘、5秒连续提交、实际PCM/画面和失败恢复。常驻图+运行时command是候选；若关键参数不支持或图更新造成必要场景断音画，记录具体失败、研究成熟safe替代；禁止退到每事件FFmpeg/播放器重启来宣布通过。
2. **UI先有限验证。** 两个嵌入消费者、一个工具、菜单10秒驻留、真实dialog、首次失败回退，见ui-feasibility；不能以headless继续解码证明draw继续。
3. **来源竞态先独立消费者验证。** 快open、旧load/error、natural ended后manual open/restart与auto-next竞争，operation未知查询、owner新旧generation。用真正后端事件关联，不把currentSession改个名通过。
4. **共享快照先一份可解码产物。** processed同时包含视频+字幕+音频链，切源/改参/撤owner后仍固定输入；raw基准/动态preroll/取消/已有文件保护有真实证据。后端版本和descriptor映射写入契约。
5. 汇总修订所有受影响C1方案和R1；NES/工坊终审到齐；SDK、协议、Rust实现、Slint组合、usage和能力发现一致，必要共享能力实际运行并由独立审查者核对后，root才发布共同开发基线/统一产品实现任务。现在不宣布0.2发布、不把P1作为已可编译API；0.1样例保留当前用法，届时统一迁移并记录破坏性变化。

官方候选依据：[FFmpeg filters](https://www.ffmpeg.org/ffmpeg-filters.html)、[mpv运行时滤镜与命令](https://mpv.io/manual/stable/)。本篇只提取运行时命令存在与构建验证必要性，不把在线手册当本机处理/音画连续性证据。原生Slint接口事实详见ui-feasibility的锁定源码链接。


## 8. 有限实现里程碑（需root后续授权，非本次实施）

| 里程碑 | 实际需交付的共享能力 | 可重放usage/必要验收 | 可以宣布的范围 |
|---|---|---|---|
| M1 来源+界面最小闭环 | source generation/intent CAS/operation查询、真实load关联、两个Slint挂载、命令同树、dialog/model | 独立两示例组件：播A→natural ended→用户restart→旧auto-next(stale)；快open A/B；栏/sidebar/工具共存，菜单10秒/dialog实际音画；未知UI失败保旧 | 仅基础公共候选可运行，不是八插件共同基线 |
| M2 实时处理必要前置 | 有限typed video/audio资源、原位命令适配、确认/回滚、owner卸载、字幕selectionCAS | eq+gblur、EQ+treble+dynaudnorm连续5秒，暂停视频预览；有效链→故意无效候选→原链仍真实生效；两个owner撤一留一；翻译A迟到对B/新selection拒绝 | 成熟路线可行/不可行的具体结论；失败时先解决必要路线，不冻结假API |
| M3 快照任务 | source lease、typed descriptor离线映射、字体/字幕快照、processed audio/preroll、job取消/发布 | 在固定t抓raw/processed PNG与含字幕MP4；任务接受后换源/改参数/停owner，产物仍固定旧输入；已存在输出hash不变，取消无临时残留 | 有限任务能力与资源回收可用，不宣称任意导出平台 |
| M4 producer+管理事务 | 单外部producer的二进制AV/input、trial/activation-ready、包版本/commit/rollback | 真ROM首帧与真实PCM→pause→输入release→切普通file；计数器JS+Slint离线校验/坏activate保旧/1.0→1.1→回退，工坊停用后计数器仍可运行 | 八方案所需共享能力实际具备时，才提交共同版本发布审查 |

这些usage是可重放验证顺序，不是已经存在的函数调用。完成每个里程碑保留命令/退出码/产物，签名收敛后换成SDK可编译示例；不为先占并发让产品作者绕未发布设施编码。必要API基线最终版本、兼容0.1的期限与统一迁移清单由root/维护者在可运行后发布；先后里程碑不构成更改工作流许可。
