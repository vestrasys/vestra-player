# 有限 MP4 独立复核

2026-10-17，独立审查者 `/root/rebuild_bootstrap_review`。自有 AV fixture/reference 的准备先于宿主运行；首个 freeze 后 root 发现非帧对齐 end 问题，独立宿主尚未启动即暂停。维护者修复绝对原始 PTS trim 并重新明确冻结后，实际执行：

`node src/tests/media-mp4-review.mjs --run-work /var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-media-mp4-prepare-vv4LJI`

宿主及外层完整分析均 exit 0，全部原门槛与新增端点预期通过。没有 build/GUI、降阈值、并发作者实例或重跑旧 PNG/rotation 套件；结束后立即明确交还冻结。作者烟测不是本轮参考。

原工作目录 `vestra-media-mp4-review-kgTUpq`。完整 [result.json](run/result.json)、[消费者事实/逐 job 状态](run/consumer-result.json)、[实际命令/退出](run/command.json)、stdio/执行 consumer/helper、12个已发布 MP4、完整视频 frame hashes/音频 PCM/ffprobe、独立参考 recipe/字体字节及 LICENSE 全部持久保存。工具分析命令在 [analysis/commands.json](run/analysis/commands.json)；准备期所有工具命令/stderr 与原始源信号/源 AAC 解码/校准参考在 preparation/。raw RGB 分析 buffer 的结果保留为量化结果，未另冒称全部 RGB buffer 都是磁盘 artifact。

## 独立信号、预先门槛与流映射

新 A/B 各有单视频320×180/30fps/12s，红/蓝来源角标；源4.0…4.2s有白色视频时标。A 的两个 mono/48kHz AAC 轨含440/880Hz载波、不同脉冲及显著幅度跃迁，B为660/1320Hz。raw参考从**实际源 AAC 解码**取，不以源编码前 f32 假作原音频。

真实 source.audioTracks 中 selected 是 streamIndex1，明确第二轨是 trackId2/streamIndex2、language=zho、selected=false（title实际null，未靠title猜轨）。capture.videoStreamIndex实际为本 fixture 唯一视频 ff-index0。产物解码频谱验证：selected 的440Hz幅度 .070914 对880Hz .0000746；第二轨880Hz .052934 对440Hz .000000878，明确导出了不同真实音轨。

主范围 [4s,8s)，每声道192,000samples@48kHz。独立 reference 从源0处理 volume−6dB→dynaudnorm(framelen100/gausssize5/maxgain4/peak.9)，之后用自写 PCM 索引实现±250ms及补零，不照搬候选 asetpts/aresample graph。准备期：错误start4重置DSP的前300ms NRMSE=.823359，而全段仅.021645；AAC192k roundtrip 前300ms .015754、全段 .0009707，证明只看全段会漏掉preroll错误。

host 运行前与root锁定全段/前300ms各 NRMSE≤.06，前窗为14,400samples/channel；不做每产物 lag/gain拟合。实际所有声轨 mono/48kHz，主片段解码均恰192,000samples/4s，prefix48,000/1s、tail28,800/.6s，本轮没有被1024sample容差掩盖的尾差。完整 correlation/gain/RMSE/MAE/maxAbs/sample counts 与原 ffprobe 的PTS/时长保留；不泛称AAC永无priming/padding。

| 实际产物 | 全段 NRMSE | 前300ms NRMSE |
| --- | ---: | ---: |
| raw 第二轨，17音量/真实静音 | .001506 | .016345 |
| raw 第二轨，91音量/解除静音 | .001506 | .016345 |
| raw selected 第一轨 | .001157 | .015027 |
| processed 旧快照 +250ms（前/后两产物相同） | .019046 | .034201 |
| processed offset0 | .000971 | .015754 |
| processed −250ms | .000745 | .009825 |
| +250ms首部片段 [0,1) | .000569 | .003565 |
| −250ms尾部片段 [11.4,12) | .035713 | .036209 |

processed-zero 与错误 start4 DSP-reset reference 前300ms NRMSE=.822693，明显排除局部起点重置规范化。正确参考对实际前300ms仅 .015754，不以实时DSP内部历史作比较对象。本文只列关键误差；如旧+250ms全段 correlation=.9998187/gain=1.0001405、negative-tail correlation=.9993680/gain=.9952926，误差并非被当成精确0。

## Raw、无音轨及旧 processed 输入

公开 `playback.set('volume',17)` / `set('muted',true)` 后，source.output实际为17/true；改到91/false后再次固定 raw 第二轨。两份实际 raw MP4 解码 PCM **逐样本相同**，且分别匹配源AAC未处理参考，排除设备gain/mute、helper已生效的−6dB/norm/+250ms被烘焙。raw-none 实际只有H264视频流、无audio stream；不是输出静音AAC来冒充无轨。

旧 processed snapshot 为 A gen1/intent1，固定第二轨、time4/end8、processing revision2：helper 的视频eq .12、音频volume−6dB+dynaudnorm(maxGain4/peak.9)、owned offset+250ms；外部字面中文字幕、黄色Noto样式、selection/presentation revision1。SRT/VTT digest 与独立本轮 bytes 相符；font digest `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`/face0/coverage确认与实际固定文件一致。

先取得旧产物；随后改变offset、停贡献helper、发布新视频/音频参数、新中文字幕与青色style，再切 B。仍由原 snapshot 产生第二旧 MP4：两份完整解码的120帧 frame-hash 行完全一致，192,000samples PCM也精确相同。实际仍对应 A/880Hz/旧字幕及旧DSP，不漂到 B/1320Hz/新配置。

独立旧视频 recipe 由本轮 A、手写字面 VTT、固定字体字节、显式eq/SAR/subtitle/绝对range/H264参数构造，不 import 宿主 mapper。完整120帧 RGB对实际 MAE=0；用有/无字幕参考差在字幕区选出的79,924个 glyph-mask pixels，实际对独立有字幕参考 MAE=0，未只靠全帧平均低声称有字。旧 [处理后首帧](run/processed-old-after-switch-disable-first.png) 与 [raw首帧](run/raw-second-low-muted-first.png) 可查看。

这是本例有限视频/glyph等效与固定资产摘要证据，不是FFmpeg/libass provider排他、任意文字shaping或另一字体替换证明。只有一个公开固定字体，不能据此宣称动态字体全范围已验证。参考也用成熟FFmpeg/libass，共同实现问题仍可能存在。

## 偏移、补零与有限 AV 对照

音频零/正/负偏移采用独立源PCM索引 reference；负偏移确实读取片段之后的源内容，正偏移确实包含片段之前内容。+250ms prefix前.2s、−250ms tail最后.2s各9,600samples纯零区，实际 RMS/max均0；边界旁≥50ms留作AAC pre-echo隔离，完整原始波形未抹除。不是用NRMSE除零处理静音。

同一3300Hz脉冲：raw输出.121s，processed +250ms为.371s，独立reference也.371s；另一个raw脉冲1.501s，processed −250ms为1.251s，reference也1.251s。二者分别明确位移±.250s，满足host前固定的参考≤20ms/相对raw≤30ms门槛。raw/processed视频白闪均为输出帧0…6@30fps，对应源4.0…4.2s；音频相对同一个视频时标的偏移得到有限实证，不是设备端AV延迟/逐样本实时DSP一致。

所有已发布主片段完整解码120帧；prefix30帧、tail18帧。无字幕选项的 zero/negative/prefix/tail snapshot 公开includeSubtitles=false且不要求expectedSubtitle，没有借此改变播放器selection来冒充选项。

## 非对齐端点、发布与取消

独立完整源帧probe保存于 [source-b-frame-times.json](run/analysis/source-b-frame-times.json)。实际 `[4.31,4.32)` 无源帧，`[4.31,4.38)` 只有源PTS4.333333/4.366667；不是从输出UI时钟猜PTS。

raw与processed各自前一范围job failed/publicationStarted=false，目标文件未发布；后一范围各完整解码**恰2帧**、无音频，MP4实际时长.066667s，体现帧量化而非硬凑请求.07s。每个产物实际帧数均按源30fps半开范围另核，旧相对duration trim放宽end的问题未在此回归重现。

已存在本轮 sentinel 的字节保持；碰撞MP4先完整编码/验证、publicationStarted=true后failed，未覆盖。错误字符串仍带“PNG no-clobber”前缀，是文案命名不准，不能否认实际MP4文件保护，也不能把文案冒作另一格式事实。

取消job16返回requested，终态cancelled/publicationStarted=false、facts=null，取消目录空。**本次只证明接受取消与未发布**：没有在取消前观察active MP4 encoder或validation子进程，facts=null尤其不能称已杀回收活跃encoder。最后host退出及实测无FFmpeg子进程/无.tmp只支持最终清理，不能倒推取消发生阶段。active MP4 worker/验证期取消仍需后续新窄证，当前不重跑。

## 有限结论

本轮明确第二轨/selected/none、raw排除真实devicegain/mute与owned DSP/offset、processed旧输入持有、source0动态规范化重算、signed offset/补零、视频与字面字幕及有限AV时标、半开非对齐端点、无覆盖和接受取消，具有独立真实公共消费者、完整MP4/PCM/ffprobe/明确reference信号误差证据，有限通过。原门槛均保留，冻结已交还。

不等于完整M3或产品插件发布通过；未覆盖active MP4/验证期取消、processed native字幕、任意codec/平台、全局容量/所有失败路径、任意角度/色彩管理、长压力、物理屏幕/设备声音或实时DSP内部状态逐样本一致。保持source0全段preroll规则，不因本次短源快而暗改为短preroll；文件租约仍非抗恶意原地写的不可变存储。
