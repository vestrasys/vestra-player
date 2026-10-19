# CPAL真实设备有限前提

源码 `src/experiments/producer-audio-probe/main.rs`，锁CPAL0.18.2，safe公开build_output_stream。命令与两段PCM真实生成配方在run/fixture-commands.json、run/diagnostic-command.json；PCM由成熟FFmpeg生成440/880Hz、48kHz双声道f32le低电平样本。Rust输入take(4MiB+1)后校验尺寸/finite，数据回调不锁、不分配、不I/O（错误回调另用有界sync_channel.try_send诊断）。

第一次真实run exit1：run/first-result.json及first-stdio保留。实际154callbacks，pause写零且A消费不变，但error callback1次；旧版只计数无详情，不能追溯原因，也不能称该次设备通过。

修后run60066 exit0：run/diagnostic-result.json。实际MacBook Air Speakers/48k stereo F32，153callbacks，phase frames29184/19968/29184，pause nonzero0、A消费不变。每phase首64sample bits有限签名与输入A/zero/B逐项相同且A≠B，证本例callback首buffer确取B起点。设备错误0/详情空，实测最大callback gap10.775666ms；没有设gap连续性门槛，不把该数值或回调计数当准确硬件时钟验收。

本例是预加载有界PCM的真实设备callback前提，不是producer transport或共同基线。没有声学录音/听觉连续性、硬件buffer drain/延迟、二进制背压、来源CAS/共用volume mute/输入release/切文件无残音证明；先前单次未分类错误仍保留，不能由后次0推断已解释。接下来只做必要共享AV纵切，不扩此probe成框架。

官方依据：https://docs.rs/cpal/0.18.2/cpal/traits/trait.DeviceTrait.html 。可复现：cargo build --manifest-path src/host/Cargo.toml --features producer-audio-probe --bin producer-audio-probe，然后按run/中的fixture和probe结构化命令执行。不是产品插件。
