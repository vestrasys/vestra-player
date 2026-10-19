# 音频覆盖充足而视频尾部不足：仅素材准备

2026-10-17。没有运行 Vestra host、GUI 或构建，没有重复旧绿色套件。使用官方 full FFmpeg/FFprobe 9.0.2 独立生成与分析本轮自有素材，具体二进制、所有 argv、版本、退出码及 stdout/stderr 在 `commands.json` 和同目录原始文件。

`source-video2-audio4.mkv`：origin 0，H264/yuv420p 320×180/30fps 视频2秒，PCM s16le 48k mono音频4秒，容器报告4秒。独立 `source-video-frames.json` 有60个实际解码视频帧，首PTS0、末1.967；[1.5,3)只有15帧，即.5秒视频。实际源音频完整解码192000 samples=4秒，包时序也独立保留；没有使用-shortest裁掉音频。

独立正常 FFmpeg 对[1.5,3)做绝对时间 trim、重置输出PTS并编码 MP4，产物可完整解码。`clip-probe.json` 实际format.duration=1.5、audio.duration=1.5，但video.duration=.5、15帧。因此即使文件可解码且容器/音频时长满足请求，视频仍缺1秒。AAC实际独立解码为72000 mono samples=1.5秒；这是解码实测，不由容器时长推算，未将任意AAC padding行为推广到其他素材。

`preparation-result.json` 保存素材/产物SHA-256与核验事实。后续唯一冻结宿主负例应使用该源请求raw/processed MP4 [1.5,3)，期待export-failed/source-unavailable、publicationStarted=false、无最终文件及临时产物泄漏。**此公共worker断言尚未执行；当前只有负例素材与独立反例证明。**
