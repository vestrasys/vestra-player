# 有归属音频与有限 MP4 候选

本块仍是 `vestra.experimental/0.1` 的阶段D候选，不发布共同基线。processed PNG 已有[独立13张产物](snapshot-evidence/independent-processed/review.md)；本文 MP4 作者产物通过，[独立MP4](snapshot-evidence/independent-mp4/review.md)已通过；后续native/sampler本轮作者通过待独立。M1–M4及八产品完整范围保留。

## 音频配置与撤除

`host.processing.replace({stage:"audio",resourceId,revision,expectedGeneration,nodes,offsetMs})` 将偏移纳入同一完整typed资源及操作token；省略offsetMs归0。每项为−2000..2000ms、10ms整数倍，正数声音更晚。`nodes:[]`、非零offset是合法delay-only；空nodes+0撤该可见resource。操作高水位仍保留，迟到旧revision只能返回旧receipt/unknownExpired，不能重施已复位资源。

各resource偏移相加，但接受时分别限制正贡献总和≤2000、负贡献绝对总和≤2000，保证任意owner子集撤除后仍可表示。A+2000、B−2000可接受；再加C+2000即使净值2000也拒绝，否则停B会越界。停用幂等撤本实例资源、按实际逻辑变化推进configurationRevision，保他方配置。

仲裁串行应用成熟af更新与audio-delay，读取实际偏移确认；失败恢复旧图和旧偏移，无法确认恢复则backendUncertain并阻止新applied声明。后端多命令不是同一物理样本原子事务，失败重配可能重置DSP历史。没有音轨时接纳配置不表示听觉输出已证；实际输出仍须真实音频验证。旧裸audio-delay在存在owned audio资源时conflict；processed capture还核实际值与owned配置一致，不能把裸属性猜成可撤销资源。

设备输出用公开 `host.playback.set("muted",boolean)` 与volume；source.state.output返回真实mute/volume，设备静音不推进source意图。未开放任意bool/property写。

## 来源与快照

已持双独立File的本地普通容器通过每load公开 `demuxer=lavf` 读取；mpv官方不保证其它demuxer的ff-index等同FFmpeg。source.state.audioTracks暴露实际id/index、selected/external/codec/title/language；id不是index。

capture在同一source stamp仲裁中固定实际selected video的lavf ff-index、显式音轨Choice与已接受File租约。封面/外部video、映射不明或非lavf明确unsupported，不退到第一video。音轨参数：省略/`{kind:"selected"}`、`{kind:"track",trackId}`、`{kind:"none"}`。指定轨需真实存在且映射明确；none不输出音轨。PNG不输出音频，但摘要仍可固定供后续片段任务。

processed增加 `includeSubtitles:false`，不冻结/不烘焙当前字幕，也不改变播放器selection；此时不用expectedSubtitle。true（保持当前实验默认）仍核selection/presentation并持外部VTT/固定font。产品默认未勾选应显式传false。native字幕include=true当前明确unsupported；plain native SRT与其它codec的成熟冻结路径仍待后续实现，不把排除选项冒充完成该范围。

## MP4任务

```typescript
const source = await host.source.state();
const processing = await host.processing.state();
const snapshot = await host.snapshot.capture({operationId:1,mode:"processed",
  expected:source.stamp,time:2.3,expectedProcessingRevision:processing.configurationRevision,
  includeSubtitles:false,audio:{kind:"track",trackId:source.audioTracks![1].trackId}});
const job = await host.jobs.start({operationId:2,snapshotId:snapshot.snapshotId,
  kind:"processedMp4",end:4.3,destination:"/absolute/path/clip.mp4"});
```

rawMp4 / processedMp4必须匹配快照模式；范围为固定 `[snapshot.time,end)`，end在冻结duration内。video按解码PTS选第一帧≥start、末帧<end；保原呈现方向/SAR，H264 yuv420p libx264 veryfast/crf18，奇数最后行/列补黑到偶数，不能裁掉原图。AAC192k、48kHz，保持音频声道布局；无音轨输出合法无声MP4。raw不烘焙视频/字幕/音频图或用户偏移；两种都排除设备volume/mute。

processed从源0按冻结typed音频图重算，随后signed PTS shift、成熟aresample48k async1/first_pts0补开头静音或裁负PTS、apad补尾、atrim取范围并归零PTS。对应`y(t)=DSP(source(t-offset))`，不克隆实时DSP状态；算法固有延迟交成熟后端，不用猜固定补偿。长源允许准备较久并可取消；当前进度未知用running，尚未实现细化媒体进度。

后台worker执行真实版本查询（控制/UI线程不spawn/等锁），jobs.state.facts记录descriptorVersion=`vestra-finite/1`、实际FFmpeg/libavfilter版本、范围/音轨适用事实；MP4完整再解码并probe后记录实际codec、sampleRate/channels/layout和duration。临时文件、取消/Publishing CAS、owner清理、release已接job、no-clobber与已有任务设施共用；完整解码验证后才发布。版本查询/验证子进程同样有取消、kill/reap和有界stdio，不因加诊断破坏等待来源锁的取消。

## 当前实证边界

作者 `nGMVaR`：六秒双轨源（440/880Hz），实际选择第二轨stream2；4张MP4分别processed norm+200ms、delay-only −200ms、raw、none。H264/AAC两秒；三份AAC解码各192000个交错样本，与独立作者source0 recipe的NRMSE分别0.0156898/0.0156760/0.0156760。公开设备volume0/mutedtrue已从source.state确认，raw仍有音轨。此例没有偷用输出音量作音频处理；不是独立验收、livePCM/设备连续性、全部边界或DSP失败回滚证明。

新增版本查询曾在Reader获准前启动，导致原「等待共享Reader可取消且不启动编码器」单测失败；保原失败日志，实际修到Reader获准后再诊断，原预期不改。最终单测/Clippy/TS/build及lavf真90°窄回归另归档；不重跑未变的全部source/PNG绿色套件。

端点修正：[独立CLI反例](snapshot-evidence/endpoint-cli/result.json)证明原relative duration与低精度TB都有风险。当前mapper先settb=AVTB，在恢复字幕偏移后的原始PTS做绝对start:end，再reset；[作者实际宿主四case](snapshot-evidence/mp4-author/endpoint/result.json)证raw/processed无帧区间失败无发布、相邻非对齐区间各2frame。最终host88/probe75测试和Clippy/build0已归mp4-author，未把CLI当宿主证据。独立MP4消费者待新freeze后执行。

后续共同基线门槛（本次冻结不改源）：legacy brightness/contrast/saturation/gamma等mpv附加属性非默认时，不能让processed仅typed图而静默遗漏；须明确拒绝不可冻结附加值/退役旧写路由或固定其值。同理legacy sub-delay须与确认样式一致。设备mute/volume本次是白名单有限公共控制，完整输出操作幂等/unknown恢复与正向设置音量解除静音语义仍需基线前对齐。待用来源/实际音轨为空的确认不得冒充PCM输出证据。

## native plain字幕与显式采样增量（独立待运行）

snapshot.subtitle改判别联合：external保两digest/coverageValidated:true；native固定实际lavf trackId/streamIndex/codec/经冻结且worker核对的normalized origin、style/font、选择条件，coverageValidated:false且没有提前声称digest。native提取在既有worker并持Reader至child reap，固定-copyts -start_at_zero -c:s copy导出SRT≤2MiB、严格parse/plain/cmap→派生WebVTT。job.facts.nativeSubtitle记录解析后的两digest/cueCount/true，不回写snapshot摘要。ASS/SSA/bitmap/mov_text明确不明时间域unsupported；plain native正文出现标签/brace/backslash或缺字则任务failed并解释，不将富文本改成字面冒等效。

PNG jobs.start新增sampleTime可选，0≤t<冻结duration；MP4拒该字段。单一快照支持多个采样job，调用者TS负责有限连拍次数、间隔、唯一输出目录与不再启动剩余项；已发布保留，active job取消/owner cleanup沿既有机制。没有core按插件名做连拍逻辑。jobs.state.facts.range.start是该单张请求时刻，snapshot.time不变；当前未独立提供精确actual decoded PTS fence。

处理冻结还拒绝非default legacy brightness/contrast/saturation/gamma；字幕冻结比较实际sub-delay与typed offset，不拒合法非0typed偏移。此保护避免未追踪裸效果静默漏进processed。来源Empty/Error的processing load确认改waiting，不能保旧load冒当前已应用。

[作者8gKYz3](snapshot-evidence/native-author/README.md)已有5张旧快照采样、typed offset/legacy拒绝及真实active encoder取消证据；当前独立native/sampler尚待运行，不把这一小块当完整M3。二次validation active cancel、细化progress与剩余产品/共同设施仍登记未验。


当前独立MP4有限结论已归[完整报告](snapshot-evidence/independent-mp4/review.md)，原PCM全段/首300ms阈值、signed offset、实际冻结图/字幕/音频、raw设备音量静音排除和端点均通过。之后native/sampler块发现来源起点问题，修正与待复验语义见[时间域修正](media-api-candidate.md#当前媒体时间域修正待独立复验)。视频尾界检查使用实际video stream duration与一帧量化容差，不用可能被apad填满的format.duration证明视频完整。
