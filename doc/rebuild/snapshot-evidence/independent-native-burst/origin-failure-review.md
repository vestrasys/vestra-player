# Native 字幕与连拍独立核验：首轮失败保留

2026-10-17。独立消费者使用维护者明确冻结的真实 headless host/SDK；未构建、未运行 GUI，未引用作者产物作为参考。本轮结论为**未通过，等待修复**，不能标记 native/burst 或完整 M3 通过。已向 root/维护者交还冻结，原失败不重跑、不降低断言。

## 材料与执行

- `preparation/`：独立生成 1280×720/30fps 的红 A、蓝 B、三个中文字面 native SRT cue，以及 markup、ASS、真实 2 秒 origin 容器；保留独立 FFmpeg 命令、ffprobe、固定字体和各 sampleTime 的参考 PNG/MP4。五个相邻参考图的 RGB MAE 为 5.908、6.802、6.385、8.658，场景可区分不同采样时间。
- `origin-failure/`：唯一真实 run RMpbEu 的全部 fixture、实际产物、已执行生成消费者/helper、host stdio、command 和 `consumer-failure.json`。`executed-review.mjs` 保存该轮独立脚本。
- 命令为 `node src/tests/media-native-burst-review.mjs --run-work <preparation-work>`；host exit 0，但消费者 runtime.failed，外层因缺少完成标记退出 1。host 正常退出不等于测试通过。

## 失败及来源事实

消费者预期非零 origin native capture 返回 `unsupported`，实际 capture 成功，断言失败。只读核查实际 run 内 `fixtures/origin2.mkv`：与 prepare 输入 SHA-256 同为 `46cb50bd91b0022a4a7308a8ae773d8bed6dfc25972d9cda355d5c3f17068228`；完整 FFprobe 的 format、video stream 0、subrip stream 1 均 start_time=2.000000，字幕默认选中标记为 1。新读取结果保存在 `origin2-independent-postfailure-ffprobe.json`。

最后公开 source.state 为 generation/intentVersion/cycle=5、Ready、paused=true、position=2、duration=10、container=matroska,webm、snapshotSource=true。这与实际非零文件时间轴一致，但不能单凭该状态推定 mpv 内部字段为何漏判。

本轮 reject helper 丢弃了意外成功的 capture 回包，host stdout 为空。因此没有保存该 origin2 快照的 native 摘要、当时字幕 sid/选轨回包或实际 mpv demuxer-start-time 值；不能补造这些事实，也不能认定某种 mpv 重基准行为已证实。修复若采用 worker 阶段验证，必须实际拒绝不支持时间轴且不发布，并明确 pending descriptor/错误阶段契约；不能仅把当前错误改成默认 source-zero。

## 已走到的范围与未完成项

失败前公共路由断言已经执行：合法 owner audio delay-only 不误拒；legacy brightness/sub-delay 偏离 typed 值被拒；旧 native 快照保持 immutable、worker facts 返回 canonical/derived digest、3 cue 和 coverageValidated=true；切 B/撤贡献 owner 后五张及部分三张任务完成；sampleTime==duration 拒绝；markup worker 明确 unsupported 失败；ASS capture 明确 unsupported。具体值、任务状态和部分 PNG 活跃子进程取消记录均以原始 checks/jobs 为准。

磁盘已有五张 burst、三张 partial、raw/exclude PNG 及旧 native MP4。但外层遇消费者失败后未进入独立 RGB/完整 MP4 分析，`analysis/` 为空。故本报告不把这些任务完成或 metadata 判定升级为像素、glyph、采样时间或 MP4 视频内容通过。原生字体提供器排他性、设备音频、所有取消/发布竞争亦未验证。

真正活跃 libx264 MP4 子进程取消用例位于失败之后，**尚未执行**；既有前轮 queued-only 取消缺口继续保留。当前不启动新 host 补测，等待维护者修复及新冻结后依原范围一致复验。
