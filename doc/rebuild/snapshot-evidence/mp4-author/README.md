# 有归属音频 / MP4 作者有限验证

本轮 `node src/scripts/verify-media-mp4.mjs` 实际 nGMVaR pass/host0，完整source、4MP4、解码与reference PCM、stdio/command/结果在run/。作者真实公开设置volume0/mutedtrue，source.state已确认。实际MP4 facts含FFmpeg9.0.2/libavfilter12.1.102、H264/AAC/48000/stereo/2秒。

lavf per-load路径变动后仅追加真90°窄回归udAjjm（官方display_rotation fixture，先ffprobe验证），host0/PNG180×320通过；未重跑整套已绿rotation/source/PNG。

原等待共享Reader取消测试因新版本诊断先启动而失败，tests-final.log保存反例；修到可取消Reader准入之后才探版本，保原断言，tests-fixed实际host87/probe74通过。Clippy-Dwarnings/build/fresh TS检查均退出0，完整原日志保存。首次unused-video函数Clippy与collapsible_if/TS end可选属性错误亦保留，不把失败log当通过。

freeze-sha256.json记录交独立消费者时当前源/SDK与实际host binary。独立尚未运行；作者不证明实时设备音频、逐样本live DSP一致、partial backendrollback或完整M3。未处理native字幕include、连拍、精细progress、5分钟保留/临时文件整体策略等剩余范围。

端点复核后的最终freeze：原先processed先reset再duration的静态缺陷已修；绝对原PTS上trim start:end并先settb=AVTB，避免低TB量化。真实public四case ZHNoEN host0/pass：raw/processed各[4.31,4.32)都failed/未发布，[4.31,4.38)各恰2frame。最终host88/probe75 tests、Clippy/build0（SDK未受该内部修复影响，TS前轮0），日志与全部fixture/产物在endpoint/。旧版本freeze摘要已以本次当前源/binary重算；早先nGMVaR仍明确属于修端点前的对齐区间作者证据，不冒充本次非对齐结果。
