# 公共 MP4 视频尾部不足：独立有限通过

2026-10-18。唯一真实run `21xTnp`，明确冻结 source/SDK/feature binary、独占headless时段；host与外层消费者均exit0，随后即时交还运行窗口。未构建/改shared、未GUI/设备试验，不重跑旧绿色套件。`run/`保留本轮执行脚本、实际插件/输入、host stdio/command、全部capture/job/状态/源事件、工具命令/诊断、合法MP4、完整ffprobe视频帧JSON及实际解码PCM。原准备说明留在preparation.md。

## 独立固定输入与负例

输入为本轮自有audio-tail-preparation/source-video2-audio4.mkv，SHA `848339a163c67cca4d5900591365122b219ab6b8bb07663272926f5b41a54545`；origin0，实际60帧/2秒H264视频，192000mono48k PCM samples/4秒音频，容器metadata4秒。既有独立FFprobe/源帧PTS/正常裁剪反例证明音频与format可满足1.5秒而视频不足，来源事实不依赖worker自己的duration报告。

真实public snapshot/jobs分别执行rawMp4、processedMp4 [1.5,3)。两项均terminal failed/export-failed，错误明确 `source-unavailable: encoded video covers 0.5s, shorter than requested 1.5s beyond one frame (0.033433333333333336s)`；publicationStarted=false，最终target不存在。两项worker保留的实际output video stream.duration=.500000，而audio.duration=1.500000、format.duration=1.500000。这实际覆盖“音频/容器够长仍因视频不足拒发布”的缺口，不能仅看总时长或编码成功。拒绝的临时MP4按约清理，没有独立保留该未发布文件作二次解码；实际stream facts及不发布为本层证据。

## 合法区间对照

同一源raw/processed [.5,1.5)各completed且实际发布。processed明确includeSubtitles:false、无typed资源/offset，两个snapshot均固定audio stream1/video0/origin0；job前后snapshot exact不变，完成后release。

两文件都由独立full FFmpeg完整AV解码exit0；FFprobe实际H264/yuv420p320×180、30fps、30个解码帧，首PTS0、末.966667且严格递增。AAC48k mono音轨实际独立解码48000个有限f32 samples，恰1秒；门槛预留的一个AAC frame尾pad没有被用到。两产物SHA同为 `535a5d30c8231557f082b2f74d485b6b58a91b40f5d4c470118ce6ce2cb43854`，不是按format.duration推帧/样本数。完整PCM保留；本窄场景没有作波形误差/声学或全帧source参考相等性结论。

四作业结束目录只剩raw-valid.mp4与processed-valid.mp4，没有负例target或.tmp；公共source stamp保持，paused/position保持，消费者没有seek/unpause或重载源。具体原始请求/查询终态、worker FFmpeg/libavfilter/sourceTimeline及output facts见run/consumer-result.json、result.json；run/command.json另存实际binary SHA。

## 范围

此有限素材的两路径视频coverage拒绝、无发布/清理和合法完整解码已通过。未据此覆盖任意VFR/时戳跳变、所有容器/codec、物理声学或完整M3；不是producer/input/NES验收。未运行额外host补测。
