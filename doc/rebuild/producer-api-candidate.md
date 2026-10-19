# 有界 AV 提供者候选

当前为 M4 共享设施实验，不是共同基线发布或 NES 产品验收。宿主必须以 `producer-audio-probe` feature 构建；默认构建不提供实际 producer 后端。SDK 见 `src/sdk/producer.ts`、`source.ts`。

## 数据与时钟

具有 `producer.av` 的实例调用 `producer.prepare({width,height})`，返回私有 Unix socket、producerId、epoch 及固定 48000Hz/stereo/f32le/RGBA8 描述。至多两个实例；每 owner 可保一个 active 和一个未提交 candidate，以便首 AV/CAS 成功前保留旧源；音频 ring 9600 帧、视频队列八帧，单音频包至多 4800 帧、RGBA 至多 4MiB。SDK 仅允许一个在途写入，写完成不代表宿主已接纳或呈现。头先验证 epoch/尺寸/长度，PCM 必须 finite 且在 [-1,1]。

`consumedFrames/48000` 是实际匹配 PCM 消费的软件媒体时钟；欠载写零且不推进它。视频按该时钟选择当前 epoch 到期帧。设备缓冲不能撤回，不承诺硬件时钟、声学连续或首帧 fence。当前配置仅在设备明确支持时选 1024 帧，约 21.333ms。

## 来源与暂停控制

准备并预填当前 epoch 音视频后，以 `source.submit({kind:"openProvider",producerId,startPaused,expected},operationId)` 接入统一来源。能力不支持 seek/rate/restart/export 时明确拒绝，原操作可查询，既有 replay/conflict 优先于新请求预检。关闭或切换来源撤销旧 active 提供者；未激活候选可 `producer.release`。

暂停/恢复是两级握手。普通控制者使用完整 source stamp 提交 pause intent，返回 `pending` ticket。提供者通过 `producer.state(id).pendingControl` 读取 `{producerId,stamp,cycle,epoch,baseSample,paused}`，在自身真实帧边界冻结/恢复核心并释放输入后，原样 `producer.ack(control)`。暂停生成新 epoch 并立即隔离旧 PCM；恢复保留该 epoch。baseSample 是请求时实际消费位置，不能自行猜测。

ACK 回包仅表示提供者 echo 已接受；还须当前 CPAL callback 完整确认 epoch/paused 后，原请求者自己的 `source.operation(operationId)` 才完成。错误 owner/字段/stamp 的 echo 为 stale 且保留 pending；无 pending 为 unavailable。唯一上一成功 ACK 可返回历史 receipt，不重复操作。五秒未完整完成会令原操作 failed，随后来源 Error 并强制回收。待处理超时/致命错误期间新 Control 拒绝，不能以 resume 更新 stamp 绕过错误；主动新 open/close 可替换来源。

## 设备健康与证据边界

`health` 为 ok/recoverableXrun/deviceChanged/streamInvalidated/deviceUnavailable/otherError。Xrun 与默认设备 reroute 记录历史并允许继续消费；后三者 sticky，立即阻止数据消费/输出并保同 epoch 最后 clock，再由控制循环将来源置 Error、回收。总数、分类数、最近八条详情和溢出计数可见；不清历史伪装无故障。

早期两次失败保留于 `host-failures/`。`host-protocol-before-ack/` 的有限正向仍含 active Xrun，不能证明新提供者 ACK 或无声学间断。正常 ACK 作者与独立跨 owner/超时结果须分别归档后引用。

尚缺：真实 ROM 到宿主的公共 AV、load/reset/state discontinuity、物理输入及可靠 release、完整包验证/trial/安装回退、准备/释放未知结果恢复。当前 synthetic boundary 不能替代提供者核心实际帧边界。

## 当前有限验收

正常 ACK 作者 JBbLNI host/script 均0，完整材料见 `producer-evidence/controls-author/`；保留一次 recoverable Xrun。独立双 runtime e4Qo9X host/outer 均0：provider owner1 与 controller owner2；原请求者101/102从 Pending 到 Completed（232/247ms），103缺失 ACK 在5070ms失败；新 WAV 来源后旧 Notice 为 unavailable，旧 producer state 不可用、socket已删除，当前来源不变。正式材料在 `producer-evidence/independent-controls/`。前两轮消费者跨 RPC 非原子采样失败原样保留，不改写为通过。该结果只证明协议与资源回收有限路径，不是 NES 核心、物理输入、声学输出或完整 M4。

## 物理输入有限候选（原生待验）

锁版Slint1.16.1启公开safe但明确不稳定的 `unstable-winit-030`，使用Winit0.30.13 physical KeyCode。不是 stable Slint API，也不用hidden/internal接口。`producer.declareInput({producerId,keys})` 仅当前active所属owner可调用，最多32个有限物理码（KeyA..Z、Digit0..9、方向、左右Shift/Control/Alt、Enter/Space/Tab及有限编辑导航键）。F11/Escape保留宿主中央处理。左右Shift始终是两个物理码，NES Select等逻辑映射只属TS提供者。

返回/`producer.state.input` 为 `{state:Notice,pending:Notice|null,keys}`。Notice含scope `{owner,producerId,ticket,inputEpoch}`、单调sequence、排序完整pressed集合、captured和reason。初始sequence0不排队。提供者逐帧读取pending，应用完整集合后 `producer.ackInput({notice})`；相同scope/sequence重读不得重复产生核心down/up。FIFO最多32条，保短down/up顺序，每条1000ms截止。精确actor/frontNotice ACK才移除；唯一上一成功ACK支持无副作用receipt重放。

点击真实画布且真实焦点/当前来源允许后capture，声明物理键才拦截，其他输入交Slint编辑控件；不从text猜。失焦/隐藏/menu/dialog/pause先revoke完整空集。空集barrier未ACK前不允许新capture/key覆盖，旧in-flight ACK不能消费它。native queue满导致transition无法接纳时，host撤成TransportFailure空集barrier，不能丢keyup留下pressed。周期仲裁主动查expiry，失败停止来源消费，不依赖提供者继续轮询。source切换/owner撤除直接撤消费与routing，提供者收到失效后亦须停止核心并释放输入。

当前State5/Delivery5/实际adapter满keyup释放回归通过，全部构建材料见input-build；不能据此声称原生键盘/GUI焦点/真实核心release已经验收。CUA仅有原子pressKey，没有跨click长按接口，持续held键开菜单仍需另实证，不能用输入注入后门替代。
