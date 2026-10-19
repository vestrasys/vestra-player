# M2 第一组处理证据独立复核

2026-10-17，审查者 `rebuild_bootstrap_review`。只读本轮 `src/experiments/processing-probe/main.rs` 与 `processing-evidence/{null-runtime,pcm-runtime}`；用 Python 标准库读取 JSON/PPM/WAV 计算，未构建、未运行播放器或 GUI、未改 probe 预期。后续新实验须另记录，不能覆盖本轮失败。

## 结论

**当前不足以通过完整 M2，可继续必要技术实验，无需因第一组失败请求用户裁决。** 已支持“常驻图目标命令可接受，连续音频参数请求期间软件 renderer 持续产生帧”的有限结论；未证明13项目标参数实际逐项生效、真实设备声音连续、暂停帧预览或完整失败回滚。

最明确失败是暂停预览：两模式 vf-command 和裸 brightness property 均返回 Ok，但三张保存 PPM 完全相同。维护者正在研究这一路径，不把命令成功或偶发 render count 增加写成暂停调参成功。

## 核验结果

| 项目 | 独立核对 | 可推导边界 |
| --- | --- | --- |
| 后端与命令 | report 为 mpv0.41.0 / FFmpeg9.0.2；13项 parameter 全部 Ok，包含视频 eq/gblur 与音频 equalizer/treble/volume/dynaudnorm 控制 | 仅证明本次命令接受，不证明每项滤镜目标被正确改变或产生预期像素/频响 |
| null 连续区间 | audio-slider 5.66636秒，150次循环，每次两命令；failed=0，frame253−82=171；区间内相邻 renderer 帧最大间隔38.5981ms | headless 软件完成帧继续，非原生 draw 或设备音频连续性 |
| pcm 连续区间 | audio-slider 5.61996秒，failed=0，frame250−81=169；区间内最大间隔38.1308ms | 同上；没有把故意 pause 的约725ms全程最大间隔当作连续拖动冻结 |
| 暂停命令 | 两模式 paused-command 的位置及帧计数均稳定，pixelMae=0 | 暂停下未看到亮度命令刷新画面 |
| 暂停裸 property | 两模式帧计数各增1，但 pixelMae=0 | renderer 完成一帧不等于画面参数更新 |
| 无效图 | `af add @bad:lavfi=[not_a_real_filter]` 返回 Err(Command)；events有真实滤镜创建失败；其后 `af` 仍列原 audio 图，不含 bad/local | 支持失败候选未留在公开图列表，不足证明旧图内部实时参数、DSP历史与真实音频效果完整保留 |

PPM 独立 SHA-256：null 三文件均为 `c0c85c29bedf8bb91ed2edc4bc8934131b8bb83c40faa960d6cb5d5355dc8fc3`；pcm 三文件均为 `1f9acfd746e777cdc82e57126f252fcf53ba59a53298233f8d0222c9f8004ba6`。各文件691215字节，包含640×360 RGB有效载荷，不是仅信任 report 的 MAE。

PCM events 明确实际 AO 为 pcm 文件写入器；WAV头为 WAVE_FORMAT_EXTENSIBLE、单声道48000Hz/16bit，共489856样本、10.20533秒，峰值5531，非空且有实际非零音频。每秒 RMS 有变化，但当前输入和动态规范化自身也可能变化，不能把 RMS 差异归因于特定 target。PCM 文件没有音频设备的调度/欠载/硬件时钟证据；其时间轴也不包含墙钟暂停时长。

## 当前证据缺口与最小后续

1. **逐项目标效果。** 现在参数阶段没有保存可关联的 playing 前后像素与音频基准；13个 Ok 不能判13项成功。先用本轮固定/已知输入为每类必要参数做一次强可辨变化和恢复：视频同源同帧颜色/高频指标，音频已知分量频响/幅度与规范化开关效果。准确记录媒体样本区间、缓冲/算法延迟及命令时刻，再验证连续更新。不是新增产品或自写算法。
2. **暂停重绘。** 现有三张同哈希是明确反例，需研究成熟 safe 后端/renderer 的当前帧重绘接缝；插件不能显式 seek、短暂 unpause 或换动态图来伪装暂停预览。后端为同一位置刷新而内部重解码不因此被禁止：须记录日志/PTS，并证明实际画面更新、paused保持、可见position不跳、无临时播放声音。播放中每事件重seek仍不允许。先区别“命令未达目标”与“参数已变但缓存帧未重算”，首轮结果尚不能选择其中一个原因。
3. **失败保旧。** 现有场景是向旧图添加一个坏节点，未覆盖“完整配置已更新一部分后后续失败”的恢复语义。先补已生效旧链的音频/像素证据，再坏候选，观察同一基准效果仍有效；另检动态历史是否被重置。`af` 字符串仍显示初始构造参数，不能据此判运行时命令没生效，也不能据此证明运行时修改保住。
4. **所有权/连续性。** local volume add/remove 是后端标签实验，尚不是两个公共 owner 资源的停用隔离；后续按 P1 有限资源语义接入再验。真实设备音频、原生 draw、A/V同步仍须独立证据，不能由本组文件输出推断。

另一个计时边界：probe 的 topology 记录在命令后先 `observe(0.4)`，再取 `at.elapsed()`；约400ms的记录包含故意观察等待，不能报告为 af add/remove 命令本身耗时400ms，也不能用它诊断同步阻塞。补计时应分别记录命令返回和后续观察窗口，不改既有成功标准。

本次有限结果支持继续定位成熟后端路线，尚不支持发布公共处理 API 共同基线。若后续证明关键参数/暂停重绘/必要实时连续性在所有可用 safe 成熟路线均不可满足，再以具体证据提交取舍；当前尚未到该结论。

## 追加：暂停隔离与首次像素变更候选

同日独立复读新增 `pause-isolation`、`norm-pause-isolation`、`norm-pause-latency` 的 report、PPM、events 及当前 probe 的采样/计时逻辑。原始两组失败继续有效；以下是不同命令路径的新证据，不改写其结果。

**局部 vf 重配置是暂停预览的正向候选，可以继续推进必要设施和 M1；完整 M2 仍未通过。** 三组先尝试 runtime brightness，均无像素变化；随后 `vf add @video:lavfi=[…brightness=-0.3…sigma=2…]` 得到实际像素变化。probe 在这里仅设置 pause=true，再执行滤镜命令，没有显式 seek 或 unpause；带 dynaudnorm 的两组也得到相同正向像素结果，故首组失败不能简单归因为“规范化存在就一定无法暂停刷新”。

独立读取有效 RGB 载荷重算：三组 runtime-brightness→local-vf-refresh 的 MAE 均为48.4920818866，约65.6301%的通道字节改变。三组前图 SHA-256 均为 `ba4f889ab6154ebd176031d677045e21dbba3e5e7c5182099b802eb570942194`，后图均为 `17eb288f08bbf05893af7afee89246d2cbeffeaf9f839d9ed842f91e61e4b0de`。这确认保存产物有显著变化，不仅是 frame count 或命令接受；由于同时改变亮度与高斯，尚不能将变化逐项归因或宣称两个算法各自完成精度验证。

report 的暂停前/最终状态均为 paused=true、position=1.633，重配置增加2个 renderer 完成帧。`norm-pause-latency/events.jsonl` 清楚记录内部 Seek、向前读至目标的 hr-seek 1.633、PlaybackRestart，以及 `restart complete @ 1.633 … (paused)`。这是成熟后端在滤镜重配置后的同位置重解码/刷新证据，不是插件显式跳转或恢复播放。它不因此违反用户暂停预览目标；但 before/after快照不构成对期间每个position/pause值的连续观测，后续仍应核对呈现PTS与实际声音。

### 109ms 数字的含义

`norm-pause-latency/report.json` 报 local-vf-refresh 命令返回1.114417ms，firstPixelChangeMs=109.457583；runtime命令firstPixelChangeMs为null。当前代码将 `at`/commandWall 放在命令之前，命令返回后立即记 `ms`，再观察1.2秒，因此这里命令耗时没有混入等待窗口。

renderer 对每个完成RGB帧取每23字节一次的64bit签名，render完成并取得Frames锁后记录时刻；查询commandWall之后首个不等于旧帧签名的时刻。所以109ms表示**这次样本从提交命令到首次检测到抽样RGB变化的 renderer 完成时间**，包含后端刷新、渲染和调度成本；不是屏幕显示延迟、设备声音延迟、完整图更新fence或稳定性能上限。稀疏签名可能漏掉变化，后图全图MAE只能补证最终变化，不能独立重建首次变化时刻。

签名时序数组没有写入该report，故本审查可核算法和摘要一致性，不能从落盘原始时序独立重算109.457583。events约3.4655秒出现PlaybackRestart，与commandWall3.3509秒之后约0.11秒的刷新量级相符，但事件是轮询记录，不是同一计时测量。若后续需要精确延迟回归，应保存采样时序/目标PTS，当前单次数字不应冻结为API保证。

证据归属补充：109.457583ms仅是作者report记录，本审查未独立精确重算；后续保留稀疏render签名原始序列即可补计时可核性，无需为此重跑全套。每组实际只保存两个after PPM（runtime-brightness和local-vf-refresh）；本审查的独立MAE来自这两个文件之间，不声称持有暂停初始PPM。初始帧相同仅来自report的initialMae=0记录。

### 仍缺的证据

三组均为ao=null，没有设备声音或PCM输出；日志虽然有 `starting audio playback`，但同段终态为paused，不能据此断言有临时发声，也不能证明完全无临时声音。原生显示、可见position不跳及暂停无临时声仍需后续实证。局部图重配置也不能直接推广为播放中每滑条事件重建图的许可；播放中连续性/动态历史、逐参数效果、失败保旧与公共owner清理仍按前文检查。

这些未完成项是M2及共同发布门槛，不构成必须等整个M2无限收尾才开始M1的理由。当前已有足够具体的暂停候选继续受限实验；不需要重新设计整套API或因首轮失败申请用户裁决。
