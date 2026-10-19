# 非帧对齐端点 CLI 对照

2026-10-17，root 实际运行本机 ffmpeg-full；只验证成熟滤镜的时间线行为，没有运行或修补 Rust 宿主。

输入为 1 秒 30fps 的 `testsrc2`，范围 `[.31,.32)` 内没有视频帧，相邻实际帧为 `.300000` 与 `.333333`。每个完整命令及原始 stderr 单独保存，可直接按 command.json 重放。

首轮工具假设原始低精度 timebase 下应为旧流程 1 帧、绝对裁剪 0 帧，实际却为 21 / 1 帧；原断言失败保存在 first-probe-assumption-failure.json。不能把这个工具假设当作宿主失败，也不能忽略原始 timebase 的量化行为。

补充对照先统一到成熟 `settb=AVTB`，只改变时间基精度、不改变源帧率：

| 图 | 实际输出帧 |
| --- | --- |
| trim start → PTS归零 → trim duration=.01 | 1 |
| trim start=.31:end=.32 → PTS归零 | 0 |

结果见 result.json。这证明相对首帧的 duration 不能替代原始时间线的 end；同时绝对 trim 也需要考虑低精度输入时间基。AVTB 为微秒精度，不声称任意有理时间戳绝对精确。

维护者正在修公共路径并准备真实非帧对齐产物。这里不代表修后宿主、MP4发布、取消、音频或独立消费者通过。
