# M2实时处理可行性实证（有限候选已确认）

当前索引：有限typed owner executor、暂停同帧像素、切源/同generation restart最新参数重放已实现并有独立稳定PPM证据（见末节）。实时设备音频/全部动态历史与故障恢复实效、完整M2共同基线仍未通过。下文按实际发现次序保留失败与当时缺口，后续收束取代相应历史状态。
2026-10-17，API maintainer；阶段D必要共享技术探针，不是产品插件实现或共同基线发布。源码`src/experiments/processing-probe/main.rs`、host新增独立`processing-probe` bin，既有0.1路由未改。第一方forbid unsafe，实际rsmpv安全software RenderContext和控制分别在独立线程，未运行GUI。

## 已有结论与未通过项

常驻lavfi图的13项runtime命令在本机mpv0.41/FFmpeg9.0.2构建均返回Ok；连续150轮音频更新超过5秒时，实际software renderer持续约30fps、position前进，没有每次loadfile或重建图。该结论是后台渲染/控制可行性，**不是屏幕菜单驻留/设备无断音验收**。

**首轮暂停调参失败，后续有限同位置refresh已得到正向像素证据，尚非所有复杂链验收通过。** `vf-command`修改brightness后，400ms内暂停位置保持但像素完全没变；mpv裸brightness property及同label局部vf reconfigure也像素MAE=0。不能用command Ok或多一次render count证明生效。后续采用已有公开vf label局部重配触发mpv自身同位置refresh，见末节。没有显式seek/unpause或另一套自写处理器。

## 可重放命令与证据

```sh
ffmpeg -v error -y -f lavfi -i testsrc2=size=640x360:rate=30 \
  -f lavfi -i 'aevalsrc=0.03*sin(2*PI*1000*t)+0.02*sin(2*PI*6000*t):s=48000' \
  -t 30 -c:v libx264 -preset ultrafast -c:a pcm_s16le /tmp/vestra-processing-source.mkv
cd src/host
cargo build --locked --bin processing-probe
./target/debug/processing-probe /tmp/vestra-processing-source.mkv ../../doc/rebuild/processing-evidence/null-runtime null
./target/debug/processing-probe /tmp/vestra-processing-source.mkv ../../doc/rebuild/processing-evidence/pcm-runtime pcm
./target/debug/processing-probe /tmp/vestra-processing-source.mkv ../../doc/rebuild/processing-evidence/paused-reconfigure null
```

三次均exit0表示探针执行完毕，不等于所有场景通过。前两次探针版本尚无paused-local-reconfigure记录且topology的ms错误包含400ms观察等待；第三次已修成纯command计时。保留旧证据不悄改数。

- [null report](processing-evidence/null-runtime/report.json) / [原始mpv事件](processing-evidence/null-runtime/events.jsonl)
- [PCM report](processing-evidence/pcm-runtime/report.json) / [实际mpv输出WAV](processing-evidence/pcm-runtime/actual-mpv-output.wav) / [频率系数](processing-evidence/pcm-runtime/spectral.json)
- [暂停局部重配report](processing-evidence/paused-reconfigure/report.json) / [事件](processing-evidence/paused-reconfigure/events.jsonl)；PPM before/command/property/local-reconfigure均保存在同目录。

host/Cargo.lock锁Rust依赖；mpv属性报告具体mpv/FFmpeg版本，ffmpeg filter help的T标记只用来选候选，不替实际命令执行。

## 常驻图与支持范围

video label `video`：`eq@tone`→`gblur@blur`，blur固定steps3；audio label `audio`：`volume@protect`→`equalizer@band`（1kHz/Q1）→`treble@high`（3kHz/slope0.5）→`dynaudnorm@norm`（framelen100/gausssize5）。一次set建立后持续用runtime command，不对每个mousemove重set整个图。

| 命令族 | target | 实测参数 | 返回 |
|---|---|---|---|
| vf-command video | eq@tone | brightness、contrast、saturation、gamma | 全Ok |
| vf-command video | gblur@blur | sigma、sigmaV | 全Ok |
| af-command audio | equalizer@band / treble@high / volume@protect | gain / gain / volume | 全Ok |
| af-command audio | dynaudnorm@norm | maxgain、peak、enable=0/1 | 全Ok |

命令形状为`[vf-command|af-command, outer-label, option, value, inner-filter-instance]`。这是后端诊断工具的明确命令，不向公共TS开放任意命令。13项Ok只证明命令通路接受；没有逐一隔离参数的像素/频率效果与dynamic history证明，不能全部标“已验证视觉/声音效果”。

| playing audio-slider区间 | 150轮（每轮EQ gain+norm peak） | position | renderer帧 | 区间最大间隔 |
|---|---|---|---|---|
| null音频输出 | 5.666s / 0失败 | +5.700s | +171 | 38.598ms |
| mpv PCM输出 | 5.620s / 0失败 | +5.633s | +169 | 38.131ms |

区间从report before.wall到after.wall筛frameTimes，不能用全程maxFrameGap（包含故意暂停约700ms）推断播放卡顿。UI draw尚未测量。

## 真实mpv处理PCM与局部拓扑

`ao=pcm`由同一个真实libmpv播放链写出48kHz mono s16le WAV，10.205秒；不是离线FFmpeg滤镜结果。源信号1k/6k系数约0.030/0.020，WAV基线相同，后续随处理变化（如源音频t=5秒约0.0588/0.0466）。它证明真实mpv处理路径有可分析样本，但多滤镜同时改变、缓存与时间偏移存在，不能归因每个参数，也不能证明扬声器输出时序/无爆音/AV延迟。原始样本保存以供独立分析。

第三次局部af add volume、remove、故意不存在滤镜：纯command耗时约0.638/0.328/0.165ms，最后返回Err(Command)，af属性保原audio graph，视频仍每0.4秒约12帧。这是局部变图与失败保已请求label的有限正向证据；不能从属性字符串推断runtime参数已反映、内部动态历史没重置或实际PCM不间断。尚未批准公共原子链替换。

## 暂停重绘定位

初始`vf-command brightness -0.3`：pause/position保持，PPM hash/MAE完全相同。裸property brightness35：renderer计数+1，像素仍同。第三次`vf add @video:lavfi=[...]`同label局部重配：约0.977ms返回Ok、计数+1、pause=true、position保持8.467s，但PPM MAE仍0。

该局部重配原始日志显示mpv内部Seek/hr-seek到同位置以及VideoReconfig；探针没有显式seek/unpause。产品要求是保持用户pause/position并真的更新像素：成熟后端同位置内部refresh不自动构成违规，但此处像素未更新所以仍失败。播放中不得每个参数事件重seek；不能把局部重配作为已解决方案。

下一步仅检查公开renderer请求重绘/刷新缓存帧的最小路径。若仍不满足，提交具体失败和成熟safe libavfilter缓存原始帧重处理候选，需核来源/PTS/字幕一致，不能把它未经实现称为已可用备用平台。

## 官方依据

[mpv runtime filters/commands](https://mpv.io/manual/stable/#command-interface)说明label、inner target及lavfi command机制；[FFmpeg filters](https://ffmpeg.org/ffmpeg-filters.html)提供目标算法选项/commands。手册用于设计实验；版本支持、返回值、像素和样本结论以上述本轮产物为准。


## 后续收束：同位置refresh可行候选与第一帧延迟

为分离首轮复杂交互，在同一合成source、640×360、30fps进行有限pause isolation：先初始中性eq+gblur（分别不含/含同一audio norm图），暂停后runtime brightness -0.3本身仍不刷新；随后仅替换video label为明确完整参数，观察1.2秒。两者都得到像素MAE48.492，pause=true、position1.633保持；没有插件显式seek/unpause/loadfile。该结果允许在**仅暂停时**使用成熟mpv内部同位置refresh，不允许播放中每mousemove重seek。

- [无norm隔离](processing-evidence/pause-isolation/report.json)
- [含norm隔离](processing-evidence/norm-pause-isolation/report.json)
- [含norm第一帧时间](processing-evidence/norm-pause-latency/report.json)：command约1.114ms，首次稀疏像素signature变化109.458ms；完整图像MAE48.492。signature仅用于首变延迟，完整PPM用于像素差，不能把123次重复render计数视作123个新解码帧。
- 强制重复render的[反例](processing-evidence/forced-redraw/report.json)：render次数+123仍MAE0，重复调用软件render不是独立解决办法。

首轮复杂配置还曾改contrast/saturation/gamma、runtime norm参数/enable、barebrightness，并在约8.4s暂停，400ms局部刷新仍MAE0。后续隔离并非完全同配置，所以**不将首轮失败草率归因norm或观察窗短**；此反例保留为资源API集成回归，尚需确认完整工作台路径。无norm/含norm两隔离证明可用有限候选，不证明任意组合都正确。

[官方mpv0.41 video.c](https://github.com/mpv-player/mpv/blob/v0.41.0/player/video.c)的mp_force_video_refresh会在暂停时issue_refresh_seek，和本轮日志一致。它是内部实现证据，不作为应用可直接调用的API；宿主只使用公开vf命令。产品判断仍是用户pause/position保持、像素确变且无临时播放/声响。

**下一步有限交付：** 不再扩无限探针。把已核runtime路径收敛为内部typed owner resource候选（完整参数校验/accepted-applied/回滚/label隔离/暂停refresh），将首轮复杂反例纳入其回归；同时M1接入session.rs有界操作本及实际mpv entry事件映射。仍未发布公共0.2，尚缺逐项效果对照、owner间动态历史保持、完整复杂链暂停回归、真实GUI/设备AV连续性等必要证明。

## Owner executor候选：真实协议消费者

`effects.rs`的有限descriptor已由`processing.rs`接入真实control worker。`processing.replace`按owner/stage/resourceId严格递增revision及来源generation校验，宿主注入manifestId；全参数校验后才调用mpv，成功才提交公共记录。固定拓扑的已测参数走`vf-command`/`af-command`；视频暂停时追加完整同label重配以请求成熟后端同位置刷新。拓扑变化目前重配整个该阶段图，**不承诺保留其他owner的DSP内部历史**。此次不是发布共同基线。

作者使用独立审查者原协议消费者真实运行exit0：[作者结果](processing-evidence/owner-author/result.json)。随后独立复验exit0：[独立结果](processing-evidence/owner-independent-review/result.json)。覆盖真实camelCase `gainDb`/`maxGain`等serde、多owner同localId、撤helper留self、过期revision/generation、未知字段及固定steps拒绝、30次video提交维持来源stamp/cycle及renderer推进。独立30次仅约932ms，**不是5秒实际slider/设备声音连续性证明**，也不证明图记录等于实际像素/PCM。

停用逐个尝试video/audio清理，不因第一阶段失败跳过另一阶段；逻辑owner始终撤销。清理失败在`processing.state.backendUncertain`保留stage原因，后续该阶段replace拒绝，必须重启宿主恢复，不能将空公共owner记录解释为后台已清干净。普通apply失败尝试恢复旧完整图，恢复失败同样标uncertain。当前没有注入真实故障证明回滚后的像素/PCM或DSP历史，仅实现失败处理路径；不宣称事务原子可见或任意错误都能恢复。

发现初轮复杂暂停反例仍未随owner候选完整回归；后续GUI与真实声音、逐项算法效果、拓扑重配连续性及实际失败恢复仍是门槛。设备volume/mute与有限Volume保护节点不同：后者仅-60..0dB，用户偏移尚未作为owner资源接入。

## 本轮共享owner API原生有限观察

root通过CUA操作V4本轮应用：亮度改变后实际画面变亮；pause位置140.31/frame4208，再改-.3后同一位置画面变暗、frame4210。该路径用真实TS SDK→JSONL→owner executor→libmpv→Slint，非返回Ok替代像素。但该暂停操作在audio/norm图创建前，不能覆盖初轮复杂反例。后续四次实际audio slider drag，最终同session1 160.83/frame4825；没有全程最大drawgap/设备音频波形或听觉证据。正式范围见`api-candidates.md`及root的`menu-evidence/`原生记录。仍不发布M2全通过。

另外，owner记录跨source存活不等于最新runtime参数在新load重建filter后已重新确认；当前尚缺source ready时按最新typed graph重放/确认的集成。不得把旧公共revision视作新source上效果已经相同。下一有限处理项是该接缝、含norm复杂pause最小回归和音频持续参数证据，不扩任意图平台。


## 已实现并独立验证：真实load重放最新typed参数

上文“尚缺source ready重放”是发现问题时的历史状态，此处予以取代。当前控制线程只在真正接受当前load entry/ticket的FileLoaded后重放所有owner最新typed图（不能只重用mpv最初属性字符串）；同generation restart也按新load重放。新loading置waiting，失败逐stage标uncertain并保原因；旧回调不能确认新source。公开load.cycle是load身份，不冒充像素fence或seek新周期。

[作者四窗证据](processing-evidence/replay-author/README.md)和[独立五窗证据](processing-evidence/replay-independent/README.md)保留完整PPM/fixtures压缩包及JSON/stdio。独立结果A灰中心128.0819→runtime brightness .2后186.3012→B startPaused仍186.3012→含norm暂停改.3为216.5747→同generation restart无新replace仍216.5747。每窗三PPM独算，A红/B蓝角标确认实际源，不把诊断observedSource当帧来源。B暂停pos0；restart gen2/cycle3/entry3。诊断最多64PPM、200ms采样，frameSourceAssociation明确none。

这证明该有限typed owner链经真实SDK/libmpv的软件像素在切源和重播后保最新参数，并正向覆盖含norm的最小暂停调参；不是任意复杂图首轮反例的全部解释，不证明首帧零旧像素、PCM/声音/动态DSP历史一致或最大屏幕drawgap。仍未发布M2共同基线。
