# 公共 MP4 视频尾部不足：独立消费者已准备，未执行

2026-10-18。仅新建本目录 `review.mjs`；node --check通过。当前没有运行host、FFmpeg编码/解码、设备或GUI，也没有构建/修改共享源码。必须收到maintainer/root明确最终source/SDK/binary冻结及时段后才能执行 `node doc/rebuild/snapshot-evidence/independent-audio-tail/review.mjs --run`。

## 实际固定输入

复用本轮新自有 `../audio-tail-preparation/source-video2-audio4.mkv`，SHA256 `848339a163c67cca4d5900591365122b219ab6b8bb07663272926f5b41a54545`。准备目录已独立证明origin0、H264320×180/30fps共60帧/2秒，PCM48k mono192000 samples/4秒，容器duration4。原ffprobe/实际帧PTS及独立裁剪反例可追溯，未重生成素材或重复绿色套件。

## 冻结后唯一run目标

真实公开source.open paused→Ready，核实际选定audio streamIndex1/video0。四项有限作业，均使用同source stamp与public snapshot/jobs：

1. raw MP4 [1.5,3)：必须failed/export-failed且明确video短缺；publicationStarted=false、无目标/tmp。核worker保留实际encoded stream facts，video约.5秒，而audio/format至少1.49秒，避免“总duration够长”误过。
2. raw MP4 [.5,1.5)：合法1秒，可完成并发布。
3. processed（无typed处理、includeSubtitles:false）[1.5,3)：同一视频短缺失败预期，不以raw反例代替processed。
4. processed [.5,1.5)：合法1秒，可完成发布。

每snapshot不被job重写，release后无.tmp；只允许两个合法输出文件。source保持同stamp/paused/position，不显式seek/unpause。作业运行/查询与终态、原capture参数/快照、actual failure facts、source events、host stdio/command全部保留到本轮新系统tmp，失败不改断言重跑。

宿主终止后独立full FFmpeg/FFprobe读取两合法产物：实际H264320×180、30个解码帧，首PTS0/末29÷30、递增；AAC48k mono，完整AV decode exit0，实际浮点PCM有限、样本48000..49024（预先只容一个AAC frame尾pad，原始sample数留证）。不拿format.duration当视频覆盖或完整解码证据。无声学/任意codec时序或完整M3结论。

本脚本只是一项缺口负例与合法正向对照，不是通用框架；不重跑旧native/MP4套件。当前**准备完成，公共worker尚未执行**。
